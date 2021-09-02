mod program_test;
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;

#[tokio::test]
async fn test_vault_net_deposit_diff() {
    // === Arrange ===
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 2, num_mints: 2 };
    let mut test = MangoProgramTest::start_new(&config).await;
    // Supress some of the logs
    solana_logger::setup_with_default(
        "solana_rbpf::vm=info,\
             solana_runtime::message_processor=debug,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info",
    );

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let user_index: usize = 0;
    let mint_index: usize = 0;
    let mut base_deposit_size: f64 = 1000.0;
    let mut base_withdraw_size: f64 = 600.9999;



    // === Act ===
    for _ in 0..10 {
        base_deposit_size *= 2.0;
        base_withdraw_size *= 2.0;
        // Deposit amounts
        let user_deposits = vec![
            (user_index, mint_index, base_deposit_size),
        ];

        // Withdraw amounts
        let user_withdraws = vec![
            (user_index, mint_index, base_withdraw_size, true),
        ];
        // Step 1: Make deposits
        deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

        // Step 2: Make withdraws
        withdraw_scenario(&mut test, &mut mango_group_cookie, &user_withdraws).await;
    }


    // === Assert ===
    // mango_group_cookie.run_keeper(&mut test).await;
    assert_vault_net_deposit_diff(
        &mut test,
        &mut mango_group_cookie,
        user_index,
        mint_index,
    ).await;

}
