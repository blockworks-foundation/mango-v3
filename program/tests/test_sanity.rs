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
        MangoProgramTestConfig { num_users: 4, ..MangoProgramTestConfig::default_two_mints() };

    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let user_index_0: usize = 0;
    let user_index_1: usize = 1;
    let user_index_2: usize = 2;
    let user_index_3: usize = 3;
    let mint_index: usize = 0;
    let base_deposit_size: f64 = 1000.0001;
    let base_withdraw_size: f64 = 600.0001;

    // Deposit amounts
    let user_deposits = vec![
        (user_index_0, mint_index, base_deposit_size),
        (user_index_1, mint_index, base_deposit_size * 2.3),
        (user_index_2, mint_index, base_deposit_size * 20.7),
        (user_index_3, mint_index, base_deposit_size * 2000.9),
    ];

    // Withdraw amounts
    let user_withdraws = vec![
        (user_index_0, mint_index, base_withdraw_size, true),
        (user_index_1, mint_index, base_withdraw_size * 2.3, true),
        (user_index_2, mint_index, base_withdraw_size * 2.7, true),
        (user_index_3, mint_index, base_withdraw_size * 2000.39, true),
    ];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Make withdraws
    withdraw_scenario(&mut test, &mut mango_group_cookie, &user_withdraws).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;
    assert_vault_net_deposit_diff(&mut test, &mut mango_group_cookie, mint_index).await;
}
