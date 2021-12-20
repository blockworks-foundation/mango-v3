#![cfg(feature = "test-bpf")]
// Tests related to depositing into mango group
mod program_test;

use program_test::cookies::*;
use program_test::*;
use solana_program_test::*;

#[tokio::test]
async fn test_deposit_succeeds() {
    // === Arrange ===
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 2, num_mints: 2 };
    let mut test = MangoProgramTest::start_new(&config).await;

    let user_index = 0;
    let amount = 10_000;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    let user_token_account = test.with_user_token_account(user_index, test.quote_index);
    let initial_balance = test.get_token_balance(user_token_account).await;
    let deposit_amount = amount * (test.quote_mint.unit as u64);

    // === Act ===
    mango_group_cookie.run_keeper(&mut test).await;

    test.perform_deposit(&mango_group_cookie, user_index, test.quote_index, deposit_amount).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let post_balance = test.get_token_balance(user_token_account).await;
    assert_eq!(post_balance, initial_balance - deposit_amount);

    let (_root_bank_pk, root_bank) =
        test.with_root_bank(&mango_group_cookie.mango_group, test.quote_index).await;
    let (_node_bank_pk, node_bank) = test.with_node_bank(&root_bank, 0).await;
    let mango_vault_balance = test.get_token_balance(node_bank.vault).await;
    assert_eq!(mango_vault_balance, deposit_amount);

    let mango_account_deposit = test
        .with_mango_account_deposit(
            &mango_group_cookie.mango_accounts[user_index].address,
            test.quote_index,
        )
        .await;
    assert_eq!(mango_account_deposit, deposit_amount);
}
