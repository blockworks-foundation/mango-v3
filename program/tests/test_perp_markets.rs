mod program_test;
use mango::{matching::*, state::*};
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;
use std::{mem::size_of, mem::size_of_val};

#[tokio::test]
async fn test_init_perp_markets() {
    // === Arrange ===
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;

    // === Act ===
    // Need to add oracles first in order to add perp_markets
    let oracle_pks = test.add_oracles_to_mango_group(&mango_group_cookie.address).await;
    let perp_market_cookies =
        mango_group_cookie.add_perp_markets(&mut test, config.num_mints - 1, &oracle_pks).await;
    mango_group_cookie.mango_group =
        test.load_account::<MangoGroup>(mango_group_cookie.address).await;
    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    for perp_market_cookie in perp_market_cookies {
        assert_eq!(size_of_val(&perp_market_cookie.perp_market), size_of::<PerpMarket>());
    }
}

#[tokio::test]
async fn test_place_perp_order() {
    // === Arrange ===
    let config = MangoProgramTestConfig::default_two_mints();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let user_index: usize = 0;
    let mint_index: usize = 0;
    let base_price: f64 = 10_000.0;
    let base_size: f64 = 1.0;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Deposit amounts
    let user_deposits = vec![
        (user_index, test.quote_index, base_price * base_size),
        (user_index, mint_index, base_size),
    ];

    // Perp Orders
    let user_perp_orders = vec![
        (user_index, mint_index, Side::Bid, 1.0, base_price),
        (user_index, mint_index, Side::Ask, 1.0, base_price * 2.0),
    ];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Place perp orders
    place_perp_order_scenario(&mut test, &mut mango_group_cookie, &user_perp_orders).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    assert_open_perp_orders(&mango_group_cookie, &user_perp_orders, STARTING_PERP_ORDER_ID);
}

#[tokio::test]
async fn test_match_perp_order() {
    // === Arrange ===
    let config = MangoProgramTestConfig::default_two_mints();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let bidder_user_index: usize = 0;
    let asker_user_index: usize = 1;
    let mint_index: usize = 0;
    let base_price: f64 = 10_000.0;
    let base_size: f64 = 1.0;

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

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Place and match spot order
    match_perp_order_scenario(&mut test, &mut mango_group_cookie, &matched_perp_orders).await;

    // Step 3: Settle pnl
    mango_group_cookie.run_keeper(&mut test).await;
    for matched_perp_order in matched_perp_orders {
        mango_group_cookie.settle_perp_funds(&mut test, &matched_perp_order).await;
    }

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    // assert_matched_perp_orders(&mango_group_cookie, &user_perp_orders);

    let bidder_base_position = mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .perp_accounts[mint_index]
        .base_position as f64;
    let bidder_quote_position = mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .perp_accounts[mint_index]
        .quote_position;
    let asker_base_position =
        mango_group_cookie.mango_accounts[asker_user_index].mango_account.perp_accounts[mint_index]
            .base_position as f64;
    let asker_quote_position =
        mango_group_cookie.mango_accounts[asker_user_index].mango_account.perp_accounts[mint_index]
            .quote_position;

    println!("bidder_base_position: {}", bidder_base_position);
    println!(
        "bidder_quote_position: {}",
        bidder_quote_position.checked_round().unwrap().to_string()
    );
    println!("asker_base_position: {}", asker_base_position);
    println!("asker_quote_position: {}", asker_quote_position.checked_round().unwrap().to_string());

    // assert!(bidder_base_position == base_position);
    // assert!(bidder_quote_position == quote_position);
    // assert!(asker_base_position == -base_position);
    // assert!(asker_quote_position <= quote_position); // TODO Figure this out...
}
