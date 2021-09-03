mod program_test;
use solana_program_test::*;
use program_test::*;
use program_test::cookies::*;
use program_test::scenarios::*;

#[tokio::test]
async fn test_add_all_markets_to_mango_group() {
    // === Arrange ===
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 1, num_mints: 16 };
    let mut test = MangoProgramTest::start_new(&config).await;
    // Supress some of the logs
    solana_logger::setup_with_default(
        "solana_rbpf::vm=info,\
             solana_runtime::message_processor=debug,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info",
    );
    // Disable all logs except error
    // solana_logger::setup_with("error");

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    let user_index = 0;
    println!("Performing deposit");

    let user_deposits =
        arrange_deposit_all_scenario(&mut test, user_index, 1000000.0, 1000000.0);

    // === Act ===
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

}
