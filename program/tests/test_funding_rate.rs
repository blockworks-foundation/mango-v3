mod program_test;
use mango::matching::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;

#[tokio::test]
async fn test_funding_rate() {
    // === Arrange ===
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 2, num_mints: 2 };
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let bidder_user_index: usize = 0;
    let asker_user_index: usize = 1;
    let mint_index: usize = 0;
    let base_price: f64 = 10_000.0;
    let base_size: f64 = 1.0;
    let new_bid_price: f64 = 10_000.0;
    let new_ask_price: f64 = 10_200.0;
    let clock = test.get_clock().await;
    let start_time = clock.unix_timestamp;
    let end_time = start_time + 3600 * 48; // 48 Hours
                                           // TODO: Figure out assertion

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Deposit amounts
    let user_deposits = vec![
        (bidder_user_index, test.quote_index, base_price),
        (asker_user_index, mint_index, 1.0),
    ];

    // Matched Perp Orders
    let matched_perp_orders = vec![vec![
        (asker_user_index, mint_index, mango::matching::Side::Ask, base_size, base_price),
        (bidder_user_index, mint_index, mango::matching::Side::Bid, base_size, base_price),
    ]];

    // Perp Orders
    let user_perp_orders = vec![
        (bidder_user_index, mint_index, Side::Bid, base_size, new_bid_price),
        (asker_user_index, mint_index, Side::Ask, base_size, new_ask_price),
    ];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Place and match spot order
    match_perp_order_scenario(&mut test, &mut mango_group_cookie, &matched_perp_orders).await;

    // Step 3: Place perp orders
    place_perp_order_scenario(&mut test, &mut mango_group_cookie, &user_perp_orders).await;

    // Step 4: Record / Log quote positions before funding
    let bidder_quote_position_before = mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .perp_accounts[mint_index]
        .quote_position;
    let asker_quote_position_before =
        mango_group_cookie.mango_accounts[asker_user_index].mango_account.perp_accounts[mint_index]
            .quote_position;
    println!("bidder_quote_position before: {}", bidder_quote_position_before.to_string());
    println!("asker_quote_position before: {}", asker_quote_position_before.to_string());

    // Step 5: Skip x hours ahead
    test.advance_clock_past_timestamp(end_time).await;

    // Step 6: Settle pnl
    mango_group_cookie.run_keeper(&mut test).await;
    for matched_perp_order in matched_perp_orders {
        mango_group_cookie.settle_perp_funds(&mut test, &matched_perp_order).await;
    }

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let bidder_quote_position_after = mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .perp_accounts[mint_index]
        .quote_position;
    let asker_quote_position_after =
        mango_group_cookie.mango_accounts[asker_user_index].mango_account.perp_accounts[mint_index]
            .quote_position;
    println!("bidder_quote_position after: {}", bidder_quote_position_after.to_string());
    println!("asker_quote_position after: {}", asker_quote_position_after.to_string());
}

// bidder_quote_position after 0 hours: -10100000000.000015631940187
// bidder_quote_position after 24 hours: -10200000000.000031263880373
//
//
// asker_quote_position after 0 hours: 9900000000
// asker_quote_position after 1 hours: 9904762731.481464873013465
// asker_quote_position after 2 hours: 9910231481.4814637849949
// asker_quote_position after 3 hours: 9913765046.296277919957163
// asker_quote_position after 6 hours: 9928306712.962941613014323
// asker_quote_position after 12 hours: 9953276620.370343922061807
// asker_quote_position after 18 hours: 9980568287.037005206379092
// asker_quote_position after 20 hours: 9987693287.037003703581206
// asker_quote_position after 22 hours: 9999422453.703668027245044
// asker_quote_position after 24 hours: 10000000000
// asker_quote_position after 48 hours: 10000000000
