mod program_test;
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;

#[tokio::test]
async fn test_vault_net_deposit_diff() {
    // === Arrange ===
    let config =
        MangoProgramTestConfig { num_users: 2, ..MangoProgramTestConfig::default_two_mints() };

    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, 2, 1).await;
    mango_group_cookie.set_oracle(&mut test, 0, 2.0).await;

    // General parameters
    let base_index = 0;
    let quote_index = 1;
    let base_deposit_size: f64 = 1000.0001;
    let base_withdraw_size: f64 = 1400.0001;

    // Deposit amounts
    let user_deposits = vec![
        (0, base_index, base_deposit_size),
        (0, quote_index, base_deposit_size),
        (1, base_index, base_deposit_size),
        (1, quote_index, base_deposit_size),
    ];

    // Withdraw amounts
    let user_withdraws = vec![(0, base_index, base_withdraw_size, true)];
    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Make withdraws
    withdraw_scenario(&mut test, &mut mango_group_cookie, &user_withdraws).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;
    assert_vault_net_deposit_diff(&mut test, &mut mango_group_cookie, 0).await;
}
