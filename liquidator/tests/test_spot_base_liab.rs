use std::collections::HashMap;
use std::time::Duration;

use fixed::types::I80F48;
use solana_program_test::*;

use crate::tokio::time::sleep;
use mango::state::{QUOTE_INDEX, ZERO_I80F48};
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;

mod program_test;

#[tokio::test]
async fn test_delegate() {
    // === Arrange ===
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 3, num_mints: 2 };
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let liqor_index: usize = 0;
    let liqee_index: usize = 1;
    let mm_index: usize = 2;
    let mint_index: usize = 0;
    let base_price: f64 = 10_000.0;
    let base_size: f64 = 1.0;
    let quote_mint = test.quote_mint;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Deposit amounts
    let user_deposits = vec![
        (liqor_index, test.quote_index, 100.0),
        (liqee_index, mint_index, 0.001),
        (mm_index, test.quote_index, 1000.0),
    ];

    // Matched Spot Orders
    let matched_spot_orders = vec![vec![
        (liqee_index, mint_index, serum_dex::matching::Side::Bid, base_size, base_price),
        (mm_index, mint_index, serum_dex::matching::Side::Ask, base_size, base_price),
    ]];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Place and match an order for 1 BTC @ 15_000
    match_spot_order_scenario(&mut test, &mut mango_group_cookie, &matched_spot_orders).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;
    // TODO
}
