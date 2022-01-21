use solana_program_test::*;
use solana_sdk::signature::Signer;
use solana_sdk::signer::keypair::Keypair;

use mango::state::{MangoAccount, ZERO_I80F48};
use program_test::cookies::*;
use program_test::*;

mod program_test;

#[tokio::test]
async fn test_create_account() {
    // === Arrange ===
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 2, num_mints: 2 };
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    let num_precreated_mango_users = 0; // create manually
    mango_group_cookie
        .full_setup(&mut test, num_precreated_mango_users, config.num_mints - 1)
        .await;

    let mango_group_pk = &mango_group_cookie.address;

    //
    // paid for by owner (test.users[0])
    //
    let account0_pk = test.create_mango_account(mango_group_pk, 0, 0, None).await;
    test.create_spot_open_orders(
        mango_group_pk,
        &mango_group_cookie.mango_group,
        &account0_pk,
        0,
        0,
        None,
    )
    .await;

    //
    // paid for by separate payer (test.users[1]) still owned by test.users[0]
    //
    let payer = Keypair::from_base58_string(&test.users[1].to_base58_string());
    let payer_lamports = test.get_lamport_balance(payer.pubkey()).await;
    let owner_lamports = test.get_lamport_balance(test.users[0].pubkey()).await;
    let account1_pk = test.create_mango_account(mango_group_pk, 0, 1, Some(&payer)).await;
    let account1 = test.load_account::<MangoAccount>(account1_pk).await;
    assert_eq!(account1.owner, test.users[0].pubkey());
    assert_eq!(test.get_lamport_balance(test.users[0].pubkey()).await, owner_lamports);
    assert!(test.get_lamport_balance(payer.pubkey()).await < payer_lamports);
    assert!(account0_pk != account1_pk);

    test.create_spot_open_orders(
        mango_group_pk,
        &mango_group_cookie.mango_group,
        &account1_pk,
        0,
        0,
        Some(&payer),
    )
    .await;
    assert_eq!(test.get_lamport_balance(test.users[0].pubkey()).await, owner_lamports);
}
