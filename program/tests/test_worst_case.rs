use std::collections::HashMap;

use fixed::types::I80F48;
use solana_program_test::*;

use mango::state::{MAX_NUM_IN_MARGIN_BASKET, ZERO_I80F48};
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;

mod program_test;

#[tokio::test]
async fn test_worst_case_v1() {
    // === Arrange ===
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let num_orders: usize = test.num_mints - 1;
    let user_index: usize = 0;
    let base_price: f64 = 10_000.0;
    let base_size: f64 = 1.0;
    let quote_mint = test.quote_mint;

    // Set oracles
    for mint_index in 0..num_orders {
        mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;
    }

    // Deposit amounts
    let user_deposits = vec![(user_index, test.quote_index, base_price * num_orders as f64)];

    // Spot Orders
    let mut user_spot_orders = vec![];
    for mint_index in 0..num_orders.min(MAX_NUM_IN_MARGIN_BASKET as usize) {
        user_spot_orders.push((
            user_index,
            mint_index,
            serum_dex::matching::Side::Bid,
            base_size,
            base_price,
        ));
    }

    // Perp Orders
    let mut user_perp_orders = vec![];
    for mint_index in 0..num_orders {
        user_perp_orders.push((
            user_index,
            mint_index,
            mango::matching::Side::Bid,
            base_size,
            base_price,
        ));
    }

    // === Act ===
    // Step 1: Deposit all tokens into mango account
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Place spot orders
    place_spot_order_scenario(&mut test, &mut mango_group_cookie, &user_spot_orders).await;

    // Step 3: Place perp orders
    place_perp_order_scenario(&mut test, &mut mango_group_cookie, &user_perp_orders).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let mut expected_values_vec: Vec<(usize, usize, HashMap<&str, I80F48>)> = Vec::new();
    for user_spot_order in user_spot_orders {
        let (user_index, mint_index, _, base_size, base_price) = user_spot_order;
        expected_values_vec.push((
            mint_index, // Mint index
            user_index, // User index
            [
                ("quote_free", ZERO_I80F48),
                ("quote_locked", test.to_native(&quote_mint, base_price * base_size)),
                ("base_free", ZERO_I80F48),
                ("base_locked", ZERO_I80F48),
            ]
            .iter()
            .cloned()
            .collect(),
        ));
    }

    for expected_values in expected_values_vec {
        assert_user_spot_orders(&mut test, &mango_group_cookie, expected_values).await;
    }

    assert_open_perp_orders(&mango_group_cookie, &user_perp_orders, STARTING_PERP_ORDER_ID);
}

#[tokio::test]
async fn test_worst_case_v2() {
    // === Arrange ===
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let borrower_user_index: usize = 0;
    let lender_user_index: usize = 1;
    let num_orders: usize = test.num_mints - 1;
    let base_price: f64 = 10_000.0;
    let base_deposit_size: f64 = 10.0;
    let base_withdraw_size: f64 = 1.0;
    let base_order_size: f64 = 1.0;

    // Set oracles
    for mint_index in 0..num_orders {
        mango_group_cookie.set_oracle(&mut test, mint_index, 10000.0000000001).await;
    }

    // Deposit amounts
    let mut user_deposits = vec![
        (
            borrower_user_index,
            test.quote_index,
            2.0 * base_price * base_order_size * num_orders as f64,
        ), // NOTE: If depositing exact amount throws insufficient
    ];
    user_deposits.extend(arrange_deposit_all_scenario(
        &mut test,
        lender_user_index,
        base_deposit_size,
        0.0,
    ));

    // Withdraw amounts
    let mut user_withdraws = vec![];
    for mint_index in 0..num_orders {
        user_withdraws.push((borrower_user_index, mint_index, base_withdraw_size, true));
    }

    // Spot Orders
    let mut user_spot_orders = vec![];
    for mint_index in 0..num_orders.min(MAX_NUM_IN_MARGIN_BASKET as usize) {
        user_spot_orders.push((
            lender_user_index,
            mint_index,
            serum_dex::matching::Side::Ask,
            base_order_size,
            base_price,
        ));
    }

    // Perp Orders
    let mut user_perp_orders = vec![];
    for mint_index in 0..num_orders {
        user_perp_orders.push((
            lender_user_index,
            mint_index,
            mango::matching::Side::Ask,
            base_order_size,
            base_price,
        ));
    }

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Make withdraws
    withdraw_scenario(&mut test, &mut mango_group_cookie, &user_withdraws).await;

    // Step 3: Check that lenders all deposits are not a nice number anymore (> 10 mint)
    mango_group_cookie.run_keeper(&mut test).await;

    // for mint_index in 0..num_orders {
    //     let base_mint = test.with_mint(mint_index);
    //     let base_deposit_amount = (base_deposit_size * base_mint.unit) as u64;
    //     let lender_base_deposit = &mango_group_cookie.mango_accounts[lender_user_index]
    //         .mango_account
    //         .get_native_deposit(
    //             &mango_group_cookie.mango_cache.root_bank_cache[mint_index],
    //             mint_index,
    //         )
    //         .unwrap();
    //     assert_ne!(
    //         lender_base_deposit.to_string(),
    //         I80F48::from_num(base_deposit_amount).to_string()
    //     );
    // }

    // Step 4: Place spot orders
    place_spot_order_scenario(&mut test, &mut mango_group_cookie, &user_spot_orders).await;

    // Step 5: Place perp orders
    place_perp_order_scenario(&mut test, &mut mango_group_cookie, &user_perp_orders).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let mut expected_values_vec: Vec<(usize, usize, HashMap<&str, I80F48>)> = Vec::new();
    for user_spot_order in user_spot_orders {
        let (user_index, mint_index, _, base_size, _) = user_spot_order;
        let mint = test.with_mint(mint_index);
        expected_values_vec.push((
            mint_index, // Mint index
            user_index, // User index
            [
                ("quote_free", ZERO_I80F48),
                ("quote_locked", ZERO_I80F48),
                ("base_free", ZERO_I80F48),
                ("base_locked", test.to_native(&mint, base_size)),
            ]
            .iter()
            .cloned()
            .collect(),
        ));
    }

    for _expected_values in expected_values_vec {
        // assert_user_spot_orders(&mut test, &mango_group_cookie, expected_values).await;
    }

    assert_open_perp_orders(&mango_group_cookie, &user_perp_orders, STARTING_PERP_ORDER_ID);
}
