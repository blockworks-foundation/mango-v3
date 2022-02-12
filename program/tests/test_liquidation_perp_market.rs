mod program_test;

use fixed::types::I80F48;
use fixed::FixedI128;
use mango::matching::Side;
use mango::state::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;
use std::cmp::min;
use std::ops::Div;
use std::str::FromStr;

#[tokio::test]
/// Simple test for ix liquidate_perp_market
/// Transfers liqees base and quote positions to liqor
/// note: doesnt check the numbers to exact accuracy
async fn test_liquidation_perp_market_basic() {
    // === Arrange ===
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 3, num_mints: 2 };
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let bidder_user_index: usize = 0;
    let asker_user_index: usize = 1;
    let liqor_user_index: usize = 2;
    let mint_index: usize = 0;
    let base_price: f64 = 10_000.0;
    let base_size: f64 = 1.0;
    let clock = test.get_clock().await;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Deposit amounts
    let user_deposits = vec![
        (bidder_user_index, test.quote_index, base_price),
        (asker_user_index, mint_index, 1.0),
        (liqor_user_index, test.quote_index, base_price),
    ];

    // Matched Perp Orders
    let matched_perp_orders = vec![vec![
        (asker_user_index, mint_index, mango::matching::Side::Ask, base_size, base_price),
        (bidder_user_index, mint_index, mango::matching::Side::Bid, base_size, base_price),
    ]];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Place and match perp order
    match_perp_order_scenario(&mut test, &mut mango_group_cookie, &matched_perp_orders).await;

    // assert that bidder has open LONG
    let bidder_quote_position = mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .perp_accounts[mint_index]
        .quote_position;
    let bidder_base_position = mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .perp_accounts[mint_index]
        .base_position;
    // dbg!(bidder_quote_position);
    // dbg!(bidder_base_position);
    // [program/tests/test_liquidation_perp_market.rs:93] bidder_quote_position = -10100000000.000015631940187
    // [program/tests/test_liquidation_perp_market.rs:94] bidder_base_position = 10000
    assert!(bidder_quote_position < I80F48::from_str("-10100000000").unwrap());
    assert!(bidder_base_position == I80F48::from_str("10000").unwrap());

    // assert that liqor has no base & quote positions
    let liqor_quote_position =
        mango_group_cookie.mango_accounts[liqor_user_index].mango_account.perp_accounts[mint_index]
            .quote_position;
    let liqor_base_position =
        mango_group_cookie.mango_accounts[liqor_user_index].mango_account.perp_accounts[mint_index]
            .base_position;
    // dbg!(liqor_quote_position);
    // dbg!(liqor_base_position);
    // [program/tests/test_liquidation_perp_market.rs:95] liqor_quote_position = 0
    // [program/tests/test_liquidation_perp_market.rs:96] liqor_base_position = 0
    assert!(liqor_quote_position == I80F48::from_str("0").unwrap());
    assert!(liqor_base_position == I80F48::from_str("0").unwrap());

    // Step 3: lower oracle price artificially to induce bad health
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price / 150.0).await;
    mango_group_cookie.run_keeper(&mut test).await;

    // Step 4: Perform a couple of liquidations
    for _ in 0..6 {
        mango_group_cookie.run_keeper(&mut test).await;
        test.perform_liquidate_perp_market(
            &mut mango_group_cookie,
            mint_index,
            bidder_user_index,
            liqor_user_index,
            1000,
        )
        .await;
    }

    // quote and base position should have been transferred to liqor

    // assert that bidder has lowered quote and base positions
    let bidder_quote_position = mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .perp_accounts[mint_index]
        .quote_position;
    let bidder_base_position = mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .perp_accounts[mint_index]
        .base_position;
    // dbg!(bidder_quote_position);
    // dbg!(bidder_base_position);
    // [program/tests/test_liquidation_perp_market.rs:127] bidder_quote_position = -10061000000.000015572325644
    // [program/tests/test_liquidation_perp_market.rs:128] bidder_base_position = 4000
    assert!(
        I80F48::from_str("-10100000000").unwrap() < bidder_quote_position
            && bidder_quote_position < I80F48::from_str("-10060000000").unwrap()
    );
    assert!(bidder_base_position == I80F48::from_str("4000").unwrap());

    // assert that liqor has non zero quote and base positions
    let liqor_quote_position =
        mango_group_cookie.mango_accounts[liqor_user_index].mango_account.perp_accounts[mint_index]
            .quote_position;
    let liqor_base_position =
        mango_group_cookie.mango_accounts[liqor_user_index].mango_account.perp_accounts[mint_index]
            .base_position;
    // dbg!(liqor_quote_position);
    // dbg!(liqor_base_position);
    // [program/tests/test_liquidation_perp_market.rs:129] liqor_quote_position = -39000000.000000059614543
    // [program/tests/test_liquidation_perp_market.rs:130] liqor_base_position = 6000
    assert!(liqor_quote_position < I80F48::from_str("-39000000").unwrap());
    assert!(liqor_base_position == I80F48::from_str("6000").unwrap());
}
