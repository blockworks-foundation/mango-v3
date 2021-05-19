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
    entrypoint::process_instruction, instruction::init_merps_account, state::MerpsAccount,
};

#[tokio::test]
async fn test_init_merps_group() {
    // Mostly a test to ensure we can successfully create the testing harness
    // Also gives us an alert if the InitMerpsGroup tx ends up using too much gas
    let program_id = Pubkey::new_unique();

    let mut test = ProgramTest::new("merps", program_id, processor!(process_instruction));

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(20_000);

    let merps_group = add_merps_group_prodlike(&mut test, program_id);

    assert_eq!(merps_group.num_markets, 0);

    let (mut banks_client, payer, recent_blockhash) = test.start().await;

    let mut transaction = Transaction::new_with_payer(
        &[merps_group.init_merps_group(&payer.pubkey())],
        Some(&payer.pubkey()),
    );

    transaction.sign(&[&payer], recent_blockhash);

    assert!(banks_client.process_transaction(transaction).await.is_ok());
}

#[tokio::test]
async fn test_init_merps_account() {
    let program_id = Pubkey::new_unique();

    let mut test = ProgramTest::new("merps", program_id, processor!(process_instruction));

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(20_000);

    let merps_group = add_merps_group_prodlike(&mut test, program_id);
    let merps_account_pk = Pubkey::new_unique();
    test.add_account(
        merps_account_pk,
        Account::new(u32::MAX as u64, size_of::<MerpsAccount>(), &program_id),
    );
    let user = Keypair::new();
    test.add_account(user.pubkey(), Account::new(u32::MAX as u64, 0, &user.pubkey()));

    let (mut banks_client, payer, recent_blockhash) = test.start().await;

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
        ],
        Some(&payer.pubkey()),
    );

    transaction.sign(&[&payer, &user], recent_blockhash);
    assert!(banks_client.process_transaction(transaction).await.is_ok());

    let mut account = banks_client.get_account(merps_account_pk).await.unwrap().unwrap();
    let account_info: AccountInfo = (&merps_account_pk, &mut account).into();
    let merps_account =
        MerpsAccount::load_mut_checked(&account_info, &program_id, &merps_group.merps_group_pk)
            .unwrap();
}
