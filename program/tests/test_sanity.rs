mod program_test;
use mango::{matching::*, state::*};
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;
use std::{mem::size_of, mem::size_of_val};

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
    let base_deposit_size: f64 = 10.0;
    let base_withdraw_size: f64 = 10.0;

    // Deposit amounts
    let user_deposits = vec![
        (user_index, mint_index, base_deposit_size),
    ];

    // Withdraw amounts
    let mut user_withdraws = vec![
        (user_index, mint_index, base_withdraw_size, true),
    ];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, user_deposits).await;

    // Step 2: Make withdraws
    withdraw_scenario(&mut test, &mut mango_group_cookie, user_withdraws).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;
    assert_vault_net_deposit_diff(
        &mut test,
        &mut mango_group_cookie,
        user_index,
        mint_index,
    ).await;

}
