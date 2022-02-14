// Tests related to liquidations
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

fn get_deposit_for_user(
    mango_group_cookie: &MangoGroupCookie,
    user_index: usize,
    mint_index: usize,
) -> I80F48 {
    mango_group_cookie.mango_accounts[user_index]
        .mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index)
        .unwrap()
}

fn get_borrow_for_user(
    mango_group_cookie: &MangoGroupCookie,
    user_index: usize,
    mint_index: usize,
) -> I80F48 {
    mango_group_cookie.mango_accounts[user_index]
        .mango_account
        .get_native_borrow(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index)
        .unwrap()
}

#[tokio::test]
/// Simple test for ix liquidate_token_and_perp
/// Transfers liqees quote deposits and quote positions to liqor
/// note: doesnt check the numbers to exact accuracy
async fn test_liquidation_token_and_perp_basic() {
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

    // assert deposit
    mango_group_cookie.run_keeper(&mut test).await;
    let bidder_quote_deposit =
        get_deposit_for_user(&mango_group_cookie, bidder_user_index, QUOTE_INDEX);
    // dbg!(bidder_quote_deposit);
    // [program/tests/test_liquidation_token_and_perp.rs:81] bidder_quote_deposit = 10000000000
    assert!(bidder_quote_deposit == I80F48::from_str("10000000000").unwrap());

    // Step 2: Place and match perp order
    match_perp_order_scenario(&mut test, &mut mango_group_cookie, &matched_perp_orders).await;

    // assert that bidder has a LONG
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
    // [program/tests/test_liquidation_token_and_perp.rs:93] bidder_quote_position = -10100000000.000015631940187
    // [program/tests/test_liquidation_token_and_perp.rs:94] bidder_base_position = 10000
    assert!(bidder_quote_position < I80F48::from_str("-10100000000").unwrap());
    assert!(bidder_base_position == I80F48::from_str("10000").unwrap());

    // Step 4: lower oracle price artificially to induce bad health
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price / 150.0).await;
    mango_group_cookie.run_keeper(&mut test).await;

    // Step 5: close base position by doing a reverse order of sorts
    let matched_perp_orders = vec![vec![
        (asker_user_index, mint_index, mango::matching::Side::Bid, base_size, base_price / 150.0),
        (bidder_user_index, mint_index, mango::matching::Side::Ask, base_size, base_price / 150.0),
    ]];
    match_perp_order_scenario(&mut test, &mut mango_group_cookie, &matched_perp_orders).await;

    // assert that bidder has no base position, but still a quote position due to price drop
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
    // [program/tests/test_liquidation_token_and_perp.rs:123] bidder_quote_position = -10034066000.00001573604891
    // [program/tests/test_liquidation_token_and_perp.rs:124] bidder_base_position = 0
    assert!(
        I80F48::from_str("-10100000000").unwrap() < bidder_quote_position
            && bidder_quote_position < I80F48::from_str("-10034066000").unwrap()
    );
    assert!(bidder_base_position == I80F48::from_str("0").unwrap());

    // Step 6: Perform a couple of liquidations
    for _ in 0..6 {
        mango_group_cookie.run_keeper(&mut test).await;
        // fixme not working
        test.perform_liquidate_token_and_perp(
            &mut mango_group_cookie,
            bidder_user_index, // The liqee
            liqor_user_index,
            AssetType::Token,
            QUOTE_INDEX,
            AssetType::Perp,
            mint_index,
            I80F48::from_str("100000").unwrap(),
        )
        .await;
    }

    mango_group_cookie.run_keeper(&mut test).await;

    // assert that bidders quote deposit has reduced
    let bidder_quote_deposit =
        get_deposit_for_user(&mango_group_cookie, bidder_user_index, QUOTE_INDEX);
    // dbg!(bidder_quote_deposit);
    // [program/tests/test_liquidation_token_and_perp.rs:155] bidder_quote_deposit = 9999400000.00000001278977
    assert!(bidder_quote_deposit < I80F48::from_str("9999500000").unwrap());

    // assert that liqors quote deposit has increased
    let liqor_quote_deposit =
        get_deposit_for_user(&mango_group_cookie, liqor_user_index, QUOTE_INDEX);
    // dbg!(liqor_quote_deposit);
    // [program/tests/test_liquidation_token_and_perp.rs:158] liqor_quote_deposit = 10000599999.99999998721023
    assert!(liqor_quote_deposit > I80F48::from_str("100005").unwrap());

    // assert that bidders quote position has reduced
    let bidder_quote_position = mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .perp_accounts[mint_index]
        .quote_position;
    // dbg!(bidder_quote_position);
    // [program/tests/test_liquidation_token_and_perp.rs:173] bidder_quote_position = -10033466000.00001573604891
    assert!(
        I80F48::from_str("-10034066000").unwrap() < bidder_quote_position
            && bidder_quote_position < I80F48::from_str("-10033466000").unwrap()
    );

    // assert that liqor has a quote position now
    let liqor_quote_position =
        mango_group_cookie.mango_accounts[liqor_user_index].mango_account.perp_accounts[mint_index]
            .quote_position;
    // dbg!(liqor_quote_position);
    // [program/tests/test_liquidation_token_and_perp.rs:164] liqor_quote_position = -60000
    assert!(liqor_quote_position == I80F48::from_str("-600000").unwrap());
}
