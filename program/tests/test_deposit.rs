#![cfg(feature = "test-bpf")]
// Tests related to depositing into mango group
use solana_program_test::*;

use program_test::*;
use program_test::cookies::*;

mod program_test;

#[tokio::test]
async fn test_deposit_succeeds() {
    // Arrange
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;
    // Disable solana log output
    solana_logger::setup_with("error");

    let user_index = 0;
    let quantity = 10000;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    let deposit_amount = (quantity * mango_group_cookie.quote_mint.unwrap().unit) as u64;
    let user_token_account = test.with_user_token_account(user_index, test.quote_index);
    let initial_balance = test.get_token_balance(user_token_account).await;

    // Act
    test.perform_deposit(
        &mango_group_cookie,
        user_index,
        test.quote_index,
        deposit_amount,
    ).await;

    // Assert
    let post_balance = test.get_token_balance(user_token_account).await;
    assert_eq!(post_balance, initial_balance - deposit_amount);

    let (_root_bank_pk, root_bank) = test.with_root_bank(&mango_group_cookie.mango_group.unwrap(), test.quote_index).await;
    let (_node_bank_pk, node_bank) = test.with_node_bank(&root_bank, 0).await;
    let mango_vault_balance = test.get_token_balance(node_bank.vault).await;
    assert_eq!(mango_vault_balance, deposit_amount);
    let mango_account_deposit = test.with_mango_account_deposit(
        &mango_group_cookie.mango_accounts[user_index].address.unwrap(),
        test.quote_index,
    ).await;
    assert_eq!(mango_account_deposit, deposit_amount);
}
