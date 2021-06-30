#![cfg(feature = "test-bpf")]

mod helpers;

use helpers::*;
use solana_program::account_info::AccountInfo;
use solana_program_test::*;
use solana_sdk::{
    account::Account,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use std::mem::size_of;

use mango::instruction::cache_root_banks;
use mango::{
    entrypoint::process_instruction,
    instruction::{deposit, init_mango_account},
    state::{MangoAccount, QUOTE_INDEX},
};

#[tokio::test]
async fn test_deposit_succeeds() {
    let program_id = Pubkey::new_unique();

    let mut test = ProgramTest::new("mango", program_id, processor!(process_instruction));

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(50_000);

    let initial_amount = 2;
    let deposit_amount = 1;

    // setup mango group
    let mango_group = add_mango_group_prodlike(&mut test, program_id);

    // setup user account
    let user = Keypair::new();
    test.add_account(user.pubkey(), Account::new(u32::MAX as u64, 0, &user.pubkey()));

    // setup user token accounts
    let user_account =
        add_token_account(&mut test, user.pubkey(), mango_group.tokens[0].pubkey, initial_amount);

    let mango_account_pk = Pubkey::new_unique();
    test.add_account(
        mango_account_pk,
        Account::new(u32::MAX as u64, size_of::<MangoAccount>(), &program_id),
    );

    let (mut banks_client, payer, recent_blockhash) = test.start().await;

    {
        let mut transaction = Transaction::new_with_payer(
            &[
                mango_group.init_mango_group(&payer.pubkey()),
                init_mango_account(
                    &program_id,
                    &mango_group.mango_group_pk,
                    &mango_account_pk,
                    &user.pubkey(),
                )
                .unwrap(),
                cache_root_banks(
                    &program_id,
                    &mango_group.mango_group_pk,
                    &mango_group.mango_cache_pk,
                    &[mango_group.root_banks[0].pubkey],
                )
                .unwrap(),
                deposit(
                    &program_id,
                    &mango_group.mango_group_pk,
                    &mango_account_pk,
                    &user.pubkey(),
                    &mango_group.mango_cache_pk,
                    &mango_group.root_banks[0].pubkey,
                    &mango_group.root_banks[0].node_banks[0].pubkey,
                    &mango_group.root_banks[0].node_banks[0].vault,
                    &user_account.pubkey,
                    deposit_amount,
                )
                .unwrap(),
            ],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[&payer, &user], recent_blockhash);

        assert!(banks_client.process_transaction(transaction).await.is_ok());

        let final_user_balance = get_token_balance(&mut banks_client, user_account.pubkey).await;
        assert_eq!(final_user_balance, initial_amount - deposit_amount);

        let mango_vault_balance =
            get_token_balance(&mut banks_client, mango_group.root_banks[0].node_banks[0].vault)
                .await;
        assert_eq!(mango_vault_balance, deposit_amount);

        let mut mango_account = banks_client.get_account(mango_account_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&mango_account_pk, &mut mango_account).into();

        let mango_account =
            MangoAccount::load_mut_checked(&account_info, &program_id, &mango_group.mango_group_pk)
                .unwrap();
        assert_eq!(mango_account.deposits[QUOTE_INDEX], deposit_amount);
    }
}
