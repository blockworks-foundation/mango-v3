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

use mango::{
    entrypoint::process_instruction, instruction::init_mango_account, state::MangoAccount,
    state::MangoGroup,
};

#[tokio::test]
async fn test_init_mango_group() {
    // Mostly a test to ensure we can successfully create the testing harness
    // Also gives us an alert if the InitMangoGroup tx ends up using too much gas
    let program_id = Pubkey::new_unique();

    let mut test = ProgramTest::new("mango", program_id, processor!(process_instruction));

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(20_000);

    let mango_group = add_mango_group_prodlike(&mut test, program_id);

    assert_eq!(mango_group.num_oracles, 0);

    let (mut banks_client, payer, recent_blockhash) = test.start().await;

    let mut transaction = Transaction::new_with_payer(
        &[mango_group.init_mango_group(&payer.pubkey())],
        Some(&payer.pubkey()),
    );

    transaction.sign(&[&payer], recent_blockhash);

    assert!(banks_client.process_transaction(transaction).await.is_ok());

    let mut account = banks_client.get_account(mango_group.mango_group_pk).await.unwrap().unwrap();
    let account_info: AccountInfo = (&mango_group.mango_group_pk, &mut account).into();
    let mango_group_loaded = MangoGroup::load_mut_checked(&account_info, &program_id).unwrap();

    assert_eq!(mango_group_loaded.valid_interval, 5)
}

#[tokio::test]
async fn test_init_mango_account() {
    let program_id = Pubkey::new_unique();

    let mut test = ProgramTest::new("mango", program_id, processor!(process_instruction));

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(20_000);

    let mango_group = add_mango_group_prodlike(&mut test, program_id);
    let mango_account_pk = Pubkey::new_unique();
    test.add_account(
        mango_account_pk,
        Account::new(u32::MAX as u64, size_of::<MangoAccount>(), &program_id),
    );
    let user = Keypair::new();
    test.add_account(user.pubkey(), Account::new(u32::MAX as u64, 0, &user.pubkey()));

    let (mut banks_client, payer, recent_blockhash) = test.start().await;

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
        ],
        Some(&payer.pubkey()),
    );

    transaction.sign(&[&payer, &user], recent_blockhash);
    assert!(banks_client.process_transaction(transaction).await.is_ok());

    let mut account = banks_client.get_account(mango_account_pk).await.unwrap().unwrap();
    let account_info: AccountInfo = (&mango_account_pk, &mut account).into();
    let mango_account =
        MangoAccount::load_mut_checked(&account_info, &program_id, &mango_group.mango_group_pk)
            .unwrap();
}
