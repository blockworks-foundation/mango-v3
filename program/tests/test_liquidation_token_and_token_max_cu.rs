mod program_test;

use fixed::types::I80F48;
use fixed::FixedI128;
use mango::state::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;
use std::cmp::min;
use std::ops::Div;
use std::str::FromStr;

/// for ix liquidate_token_and_token, test max cu usage (that it doesnt exceed 200k),
/// by having spot open orders accounts, orders,
/// and perp positions across as many markets as possible
#[tokio::test]
async fn test_liquidation_token_and_token_max_cu() {
    let config = MangoProgramTestConfig {
        num_users: 3,
        compute_limit: 160_000, // 151171 of 160000 compute units
        ..MangoProgramTestConfig::default()
    };

    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    let bidder_user_index: usize = 0;
    let asker_user_index: usize = 1;
    let mint_index: usize = 0;
    let base_price: f64 = 15_000.0;
    let base_size: f64 = 1.0;

    {
        mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

        let user_deposits = vec![
            (bidder_user_index, test.quote_index, 11_000.0),
            (asker_user_index, mint_index, 1.0),
            (asker_user_index, test.quote_index, 11_001.0),
        ];
        deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

        // borrow some assets by placing and settling a trade
        let matched_spot_orders = vec![vec![
            (bidder_user_index, mint_index, serum_dex::matching::Side::Bid, base_size, base_price),
            (asker_user_index, mint_index, serum_dex::matching::Side::Ask, base_size, base_price),
        ]];
        match_spot_order_scenario(&mut test, &mut mango_group_cookie, &matched_spot_orders).await;
        for matched_spot_order in matched_spot_orders {
            mango_group_cookie.settle_spot_funds(&mut test, &matched_spot_order).await;
        }

        // create a corresponding perp position position to max out cu usage
        let matched_perp_orders = vec![vec![
            (asker_user_index, mint_index, mango::matching::Side::Ask, 0.0001, base_price),
            (bidder_user_index, mint_index, mango::matching::Side::Bid, 0.0001, base_price),
        ]];
        match_perp_order_scenario(&mut test, &mut mango_group_cookie, &matched_perp_orders).await;
    }

    // create open orders account for 5 these markets, place and settle trade across all these
    // 5 markets,
    // also create perp positions across all markets
    // ...to max out cu usage
    for market_index in 1..6 {
        mango_group_cookie.set_oracle(&mut test, market_index, 1.0).await;

        let user_deposits = vec![
            (bidder_user_index, test.quote_index, 2.0),
            (asker_user_index, market_index, 1.0),
            (asker_user_index, test.quote_index, 1.0),
        ];
        deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

        let matched_spot_orders = vec![vec![
            (bidder_user_index, market_index, serum_dex::matching::Side::Bid, base_size, 1.),
            (asker_user_index, market_index, serum_dex::matching::Side::Ask, base_size, 1.),
        ]];
        match_spot_order_scenario(&mut test, &mut mango_group_cookie, &matched_spot_orders).await;
        for matched_spot_order in matched_spot_orders {
            mango_group_cookie.settle_spot_funds(&mut test, &matched_spot_order).await;
        }

        let matched_perp_orders = vec![vec![
            (asker_user_index, market_index, mango::matching::Side::Ask, base_size, 1.),
            (bidder_user_index, market_index, mango::matching::Side::Bid, base_size, 1.),
        ]];
        match_perp_order_scenario(&mut test, &mut mango_group_cookie, &matched_perp_orders).await;
    }

    // create open orders account for across all remaining 9 markets, place (unmatched) orders across
    // all these 9 markets, 9 is maximum number of markets across which user can have orders,
    // also create perp positions across all markets
    // ...to max out cu usage
    for market_index in 6..15 {
        mango_group_cookie.set_oracle(&mut test, market_index, 1.0).await;

        let user_deposits = vec![
            (bidder_user_index, test.quote_index, 2.0),
            (asker_user_index, market_index, 1.0),
            (asker_user_index, test.quote_index, 1.0),
        ];
        deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

        let matched_spot_orders = vec![vec![
            (bidder_user_index, market_index, serum_dex::matching::Side::Bid, base_size, 0.9),
            (asker_user_index, market_index, serum_dex::matching::Side::Ask, base_size, 1.1),
        ]];
        match_spot_order_scenario(&mut test, &mut mango_group_cookie, &matched_spot_orders).await;

        let matched_perp_orders = vec![vec![
            (asker_user_index, market_index, mango::matching::Side::Ask, base_size, 1.),
            (bidder_user_index, market_index, mango::matching::Side::Bid, base_size, 1.),
        ]];
        match_perp_order_scenario(&mut test, &mut mango_group_cookie, &matched_perp_orders).await;
    }

    // change the oracle price so that bidder becomes liqee
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price / 15.0).await;

    mango_group_cookie.run_keeper(&mut test).await;

    // perform a liquidation to test cu usage
    test.perform_liquidate_token_and_token(
        &mut mango_group_cookie,
        bidder_user_index, // The liqee
        asker_user_index,
        mint_index,  // Asset index
        QUOTE_INDEX, // Liab index
    )
    .await;
}
