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

use merps::{
    entrypoint::process_instruction,
    instruction::{deposit, init_merps_account},
    state::{MerpsAccount, QUOTE_INDEX},
};

#[tokio::test]
async fn test_deposit_succeeds() {
    let program_id = Pubkey::new_unique();

    let mut test = ProgramTest::new("merps", program_id, processor!(process_instruction));

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(50_000);

    let initial_amount = 2;
    let deposit_amount = 1;

    // setup merps group
    let merps_group = add_merps_group_prodlike(&mut test, program_id);

    // setup user account
    let user = Keypair::new();
    test.add_account(user.pubkey(), Account::new(u32::MAX as u64, 0, &user.pubkey()));

    // setup user token accounts
    let user_account =
        add_token_account(&mut test, user.pubkey(), merps_group.tokens[0].pubkey, initial_amount);

    let merps_account_pk = Pubkey::new_unique();
    test.add_account(
        merps_account_pk,
        Account::new(u32::MAX as u64, size_of::<MerpsAccount>(), &program_id),
    );

    let (mut banks_client, payer, recent_blockhash) = test.start().await;

    {
        let mut transaction = Transaction::new_with_payer(
            &[
                merps_group.init_merps_group(&payer.pubkey()),
                init_merps_account(
                    &program_id,
                    &merps_group.merps_group_pk,
                    &merps_account_pk,
                    &user.pubkey(),
                )
                .unwrap(),
                deposit(
                    &program_id,
                    &merps_group.merps_group_pk,
                    &merps_account_pk,
                    &user.pubkey(),
                    &merps_group.root_banks[0].pubkey,
                    &merps_group.root_banks[0].node_banks[0].pubkey,
                    &merps_group.root_banks[0].node_banks[0].vault,
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

        let merps_vault_balance =
            get_token_balance(&mut banks_client, merps_group.root_banks[0].node_banks[0].vault)
                .await;
        assert_eq!(merps_vault_balance, deposit_amount);

        let mut merps_account = banks_client.get_account(merps_account_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&merps_account_pk, &mut merps_account).into();

        let merps_account =
            MerpsAccount::load_mut_checked(&account_info, &program_id, &merps_group.merps_group_pk)
                .unwrap();
        assert_eq!(merps_account.deposits[QUOTE_INDEX], deposit_amount);
    }
}
