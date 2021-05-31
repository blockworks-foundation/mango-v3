// Tests related to borrowing on a MerpsGroup
#![cfg(feature = "test-bpf")]

mod helpers;

use fixed::types::I80F48;
use helpers::*;
use merps::{
    entrypoint::process_instruction,
    instruction::{
        add_spot_market, add_to_basket, cache_prices, cache_root_banks, deposit,
        init_merps_account, withdraw,
    },
    state::{MerpsAccount, MerpsGroup, NodeBank, QUOTE_INDEX},
};
use solana_program::account_info::AccountInfo;
use solana_program_test::*;
use solana_sdk::{
    account::Account,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use std::mem::size_of;

#[tokio::test]
async fn test_borrow_succeeds() {
    // Test that the borrow instruction succeeds and the expected side effects occurr
    let program_id = Pubkey::new_unique();

    let mut test = ProgramTest::new("merps", program_id, processor!(process_instruction));

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(50_000);

    let quote_index = 0;
    let initial_amount = 200;
    let deposit_amount = 100;
    // 5x leverage
    let borrow_and_withdraw_amount = (deposit_amount * 5) / PRICE_BTC;

    let merps_group = add_merps_group_prodlike(&mut test, program_id);
    let merps_group_pk = merps_group.merps_group_pk;

    let user = Keypair::new();
    let admin = Keypair::new();
    test.add_account(user.pubkey(), Account::new(u32::MAX as u64, 0, &user.pubkey()));

    let user_quote_account = add_token_account(
        &mut test,
        user.pubkey(),
        merps_group.tokens[quote_index].pubkey,
        initial_amount,
    );

    let merps_account_pk = Pubkey::new_unique();
    test.add_account(
        merps_account_pk,
        Account::new(u32::MAX as u64, size_of::<MerpsAccount>(), &program_id),
    );

    let btc_vault_init_amount = 600;
    let btc_mint = add_mint(&mut test, 6);
    let btc_vault =
        add_token_account(&mut test, merps_group.signer_pk, btc_mint.pubkey, btc_vault_init_amount);
    let btc_node_bank = add_node_bank(&mut test, &program_id, btc_vault.pubkey);
    let btc_root_bank = add_root_bank(&mut test, &program_id, btc_node_bank);

    let unit = 10u64.pow(6);
    let btc_usdt = add_aggregator(&mut test, "BTC:USDT", 6, PRICE_BTC * unit, &program_id);

    let dex_program_pk = Pubkey::new_unique();
    let btc_usdt_spot_mkt = add_dex_empty(
        &mut test,
        btc_mint.pubkey,
        merps_group.tokens[quote_index].pubkey,
        dex_program_pk,
    );

    let user_btc_account = add_token_account(&mut test, user.pubkey(), btc_mint.pubkey, 0);

    let (mut banks_client, payer, recent_blockhash) = test.start().await;

    // setup merps group and merps account, make a deposit, add market to basket
    {
        let mut transaction = Transaction::new_with_payer(
            &[
                merps_group.init_merps_group(&admin.pubkey()),
                init_merps_account(&program_id, &merps_group_pk, &merps_account_pk, &user.pubkey())
                    .unwrap(),
                deposit(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_pk,
                    &user.pubkey(),
                    &merps_group.root_banks[quote_index].pubkey,
                    &merps_group.root_banks[quote_index].node_banks[quote_index].pubkey,
                    &merps_group.root_banks[quote_index].node_banks[quote_index].vault,
                    &user_quote_account.pubkey,
                    deposit_amount,
                )
                .unwrap(),
                add_spot_market(
                    &program_id,
                    &merps_group_pk,
                    &btc_usdt_spot_mkt.pubkey,
                    &dex_program_pk,
                    &btc_mint.pubkey,
                    &btc_root_bank.node_banks[0].pubkey,
                    &btc_vault.pubkey,
                    &btc_root_bank.pubkey,
                    &btc_usdt.pubkey,
                    &admin.pubkey(),
                    I80F48::from_num(0.83),
                    I80F48::from_num(1),
                )
                .unwrap(),
                add_to_basket(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_pk,
                    &user.pubkey(),
                    &btc_usdt_spot_mkt.pubkey,
                )
                .unwrap(),
            ],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[&payer, &admin, &user], recent_blockhash);

        // Test transaction succeeded
        assert!(banks_client.process_transaction(transaction).await.is_ok());

        let mut node_bank = banks_client.get_account(btc_node_bank.pubkey).await.unwrap().unwrap();
        let account_info: AccountInfo = (&btc_node_bank.pubkey, &mut node_bank).into();
        let node_bank = NodeBank::load_mut_checked(&account_info, &program_id).unwrap();
        assert_eq!(node_bank.borrows, 0);

        let btc_vault_balance = get_token_balance(&mut banks_client, btc_vault.pubkey).await;
        assert_eq!(btc_vault_balance, 600)
    }

    // make a borrow and withdraw
    {
        let mut merps_account = banks_client.get_account(merps_account_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&merps_account_pk, &mut merps_account).into();
        let merps_account =
            MerpsAccount::load_mut_checked(&account_info, &program_id, &merps_group_pk).unwrap();
        let mut merps_group = banks_client.get_account(merps_group_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&merps_group_pk, &mut merps_group).into();
        let merps_group = MerpsGroup::load_mut_checked(&account_info, &program_id).unwrap();
        let borrow_token_index = 0;

        println!("borrow amount: {}", borrow_and_withdraw_amount);

        let mut transaction = Transaction::new_with_payer(
            &[
                cache_prices(
                    &program_id,
                    &merps_group_pk,
                    &merps_group.merps_cache,
                    &[btc_usdt.pubkey],
                )
                .unwrap(),
                cache_root_banks(
                    &program_id,
                    &merps_group_pk,
                    &merps_group.merps_cache,
                    &[merps_group.root_banks[QUOTE_INDEX], btc_root_bank.pubkey],
                )
                .unwrap(),
                withdraw(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_pk,
                    &user.pubkey(),
                    &merps_group.merps_cache,
                    &merps_group.root_banks[borrow_token_index],
                    &btc_root_bank.node_banks[0].pubkey,
                    &btc_vault.pubkey,
                    &user_btc_account.pubkey,
                    &merps_group.signer_key,
                    &merps_account.spot_open_orders,
                    borrow_and_withdraw_amount,
                    true, // allow_borrow
                )
                .unwrap(),
            ],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[&payer, &user], recent_blockhash);

        let result = banks_client.process_transaction(transaction).await;

        // Test transaction succeeded
        assert!(result.is_ok());

        let mut merps_account = banks_client.get_account(merps_account_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&merps_account_pk, &mut merps_account).into();
        let merps_account =
            MerpsAccount::load_mut_checked(&account_info, &program_id, &merps_group_pk).unwrap();

        // Test expected borrow is in merps account
        assert_eq!(merps_account.borrows[borrow_token_index], borrow_and_withdraw_amount);

        // // Test expected borrow is added to total in node bank
        let mut node_bank = banks_client.get_account(btc_node_bank.pubkey).await.unwrap().unwrap();
        let account_info: AccountInfo = (&btc_node_bank.pubkey, &mut node_bank).into();
        let node_bank = NodeBank::load_mut_checked(&account_info, &program_id).unwrap();
        assert_eq!(node_bank.borrows, borrow_and_withdraw_amount);

        let btc_vault_balance = get_token_balance(&mut banks_client, btc_vault.pubkey).await;
        assert_eq!(btc_vault_balance, 600 - borrow_and_withdraw_amount)
    }
}

#[tokio::test]
async fn test_borrow_fails_overleveraged() {}
