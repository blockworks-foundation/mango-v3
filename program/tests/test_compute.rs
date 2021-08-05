mod program_test;
use solana_program_test::*;
use program_test::*;
use program_test::cookies::*;

#[tokio::test]
async fn test_add_all_markets_to_mango_group() {
    // Arrange
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 1, num_mints: 16 };
    let mut test = MangoProgramTest::start_new(&config).await;
    solana_logger::setup_with_default(
        "solana_rbpf::vm=info,\
             solana_runtime::message_processor=info,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info",
    );

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    let user_index = 0;
    println!("Performing deposit");

    for i in 0..test.num_mints {
        test.perform_deposit(
            &mango_group_cookie,
            user_index,
            i,
            1000000,
        ).await;
    }
}
