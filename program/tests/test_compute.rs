mod program_test;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;

#[tokio::test]
async fn test_add_all_markets_to_mango_group() {
    // === Arrange ===
    let config = MangoProgramTestConfig { num_users: 1, ..MangoProgramTestConfig::default() };

    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    let user_index = 0;
    println!("Performing deposit");

    let user_deposits = arrange_deposit_all_scenario(&mut test, user_index, 1000000.0, 1000000.0);

    // === Act ===
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;
}
