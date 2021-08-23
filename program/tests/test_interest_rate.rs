mod program_test;
use fixed::types::I80F48;
use mango::{matching::*, state::*};
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;
use std::{mem::size_of, mem::size_of_val};

#[tokio::test]
async fn test_interest_rate() {
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
    // Disable all logs except error
    // solana_logger::setup_with("error");

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let borrower_user_index: usize = 0;
    let lender_user_index: usize = 1;
    let num_orders: usize = test.num_mints - 1;
    let mint_index: usize = 0;
    let base_price: f64 = 10_000.0;
    let base_size: f64 = 1.0;
    let base_deposit_size: f64 = 0.01;
    let base_withdraw_size: f64 = 0.000001;
    let base_order_size: f64 = 1.0;
    let mut clock = test.get_clock().await;
    let start_time = clock.unix_timestamp;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Deposit amounts
    let user_deposits = vec![
        (borrower_user_index, test.quote_index, base_price * base_deposit_size * 2.0),
        (lender_user_index, mint_index, base_deposit_size),
    ];

    // Withdraw amounts
    let user_withdraws = vec![(borrower_user_index, mint_index, base_withdraw_size, true)];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, user_deposits).await;

    // Step 2: Make withdraws
    withdraw_scenario(&mut test, &mut mango_group_cookie, user_withdraws).await;

    //Assert
    clock = test.get_clock().await;
    let end_time = clock.unix_timestamp - 1;
    mango_group_cookie.run_keeper(&mut test).await;
    let borrower_base_borrow = &mango_group_cookie.mango_accounts[borrower_user_index]
        .mango_account
        .get_native_borrow(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index)
        .unwrap();
    let lender_base_deposit = &mango_group_cookie.mango_accounts[lender_user_index]
        .mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index)
        .unwrap();

    let mint = test.with_mint(mint_index);
    let optimal_util = I80F48::from_num(0.7);
    let optimal_rate = I80F48::from_num(0.06) / YEAR;
    let max_rate = I80F48::from_num(1.5) / YEAR;
    let native_borrows = I80F48::from_num(test.base_size_number_to_lots(&mint, 1000000.0));
    let native_deposits = I80F48::from_num(test.base_size_number_to_lots(&mint, 1000000.0));

    let utilization = native_borrows.checked_div(native_deposits).unwrap_or(ZERO_I80F48);
    let interest_rate = if utilization > optimal_util {
        let extra_util = utilization - optimal_util;
        let slope = (max_rate - optimal_rate) / (ONE_I80F48 - optimal_util);
        optimal_rate + slope * extra_util
    } else {
        let slope = optimal_rate / optimal_util;
        slope * utilization
    };
    let borrow_interest =
        interest_rate.checked_mul(I80F48::from_num(start_time - end_time)).unwrap();
    let deposit_interest = borrow_interest.checked_mul(utilization).unwrap();
    let new_borrow = borrower_base_borrow + borrow_interest;
    println!("new_borrow: {}", new_borrow.to_string());
    // TODO: Assert
    // Deposit: 1, Borrow: 1 = 0.000000047564683
    // Deposit: 1, Borrow: 0.5 = 0.000000001358988
    // Deposit: 1, Borrow: 0.05 = 0.0000000001359
    // Deposit: 2, Borrow: 0.05 = 0.00000000006795
}
