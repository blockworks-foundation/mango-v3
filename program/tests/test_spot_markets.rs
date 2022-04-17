use std::collections::HashMap;

use fixed::types::I80F48;
use fixed_macro::types::I80F48;
use solana_program_test::*;

use mango::processor::get_leverage_weights;
use mango::state::{MangoGroup, RootBank, QUOTE_INDEX, ZERO_I80F48};
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;

mod program_test;

#[tokio::test]
async fn test_list_spot_market_on_serum() {
    // === Arrange ===
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mint_index: usize = 0;
    // === Act ===
    let spot_market_cookie = test.list_spot_market(mint_index).await;
    // === Assert ===
    println!("Serum Market PK: {}", spot_market_cookie.market.to_string());
    // Todo: Figure out how to assert this
}

#[tokio::test]
async fn test_init_spot_markets() {
    // === Arrange ===
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;

    // === Act ===
    let oracle_pks = test.add_oracles_to_mango_group(&mango_group_cookie.address).await;
    mango_group_cookie.add_spot_markets(&mut test, config.num_mints - 1, &oracle_pks).await;

    // === Assert ===
    // TODO: Figure out how to assert
}

#[tokio::test]
async fn test_change_spot_market_params() {
    // === Arrange ===
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;

    // === Act ===
    let oracle_pks = test.add_oracles_to_mango_group(&mango_group_cookie.address).await;
    let mut spot_markets =
        mango_group_cookie.add_spot_markets(&mut test, config.num_mints - 1, &oracle_pks).await;

    // == Change spot market params, and verify them ==
    let init_leverage = Option::from(I80F48::from_num(20));
    let maint_leverage = Option::from(init_leverage.unwrap() * 2);
    let liquidation_fee = Option::from(I80F48::from_num(0.015));
    let optimal_util = Option::from(I80F48::from_num(0.8));
    let optimal_rate = Option::from(I80F48::from_num(0.09));
    let max_rate = Option::from(I80F48::from_num(1.4));
    let version = Option::from(0);

    // load anew, after adding spot market
    let mango_group = test.load_account::<MangoGroup>(mango_group_cookie.address).await;
    let root_bank_pk = mango_group.tokens[0].root_bank;

    spot_markets[0]
        .change_params(
            &mut test,
            &mango_group_cookie.address,
            &root_bank_pk,
            init_leverage,
            maint_leverage,
            liquidation_fee,
            optimal_util,
            optimal_rate,
            max_rate,
            version,
        )
        .await;

    // load anew, after changing spot market params
    let mango_group = test.load_account::<MangoGroup>(mango_group_cookie.address).await;
    let spot_market_info = mango_group.spot_markets[0];
    let (init_asset_weight, init_liab_weight) = get_leverage_weights(init_leverage.unwrap());
    assert!(spot_market_info.init_asset_weight == init_asset_weight);
    assert!(spot_market_info.init_liab_weight == init_liab_weight);
    let (maint_asset_weight, maint_liab_weight) = get_leverage_weights(maint_leverage.unwrap());
    assert!(spot_market_info.maint_asset_weight == maint_asset_weight);
    assert!(spot_market_info.maint_liab_weight == maint_liab_weight);

    let root_bank = test.load_account::<RootBank>(root_bank_pk).await;
    assert!(root_bank.optimal_util == optimal_util.unwrap());
    assert!(root_bank.optimal_rate == optimal_rate.unwrap());
    assert!(root_bank.max_rate == max_rate.unwrap());
    assert_eq!(root_bank.meta_data.version, version.unwrap());
}

#[tokio::test]
async fn test_place_spot_order() {
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
    let quote_mint = test.quote_mint;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Deposit amounts
    let user_deposits = vec![(user_index, test.quote_index, base_price)];

    // Spot Orders
    let user_spot_orders =
        vec![(user_index, mint_index, serum_dex::matching::Side::Bid, base_size, base_price)];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Place spot orders
    place_spot_order_scenario(&mut test, &mut mango_group_cookie, &user_spot_orders).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let expected_values_vec: Vec<(usize, usize, HashMap<&str, I80F48>)> = vec![(
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
    )];

    for expected_values in expected_values_vec {
        assert_user_spot_orders(&mut test, &mango_group_cookie, expected_values).await;
    }
}

#[tokio::test]
async fn test_match_spot_order() {
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
    let mint = test.with_mint(mint_index);
    let quote_mint = test.quote_mint;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Deposit amounts
    let user_deposits = vec![
        (bidder_user_index, test.quote_index, base_price),
        (asker_user_index, mint_index, 1.0),
    ];

    // Matched Spot Orders
    let matched_spot_orders = vec![vec![
        (bidder_user_index, mint_index, serum_dex::matching::Side::Bid, base_size, base_price),
        (asker_user_index, mint_index, serum_dex::matching::Side::Ask, base_size, base_price),
    ]];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Place and match spot order
    match_spot_order_scenario(&mut test, &mut mango_group_cookie, &matched_spot_orders).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let expected_deposits_vec: Vec<(usize, HashMap<usize, I80F48>)> = vec![
        (
            bidder_user_index, // User index
            [(mint_index, ZERO_I80F48), (QUOTE_INDEX, ZERO_I80F48)].iter().cloned().collect(),
        ),
        (
            asker_user_index, // User index
            [
                (mint_index, ZERO_I80F48),
                (QUOTE_INDEX, test.to_native(&quote_mint, 9996.0)), // taker fee: 0.04% of base price
            ]
            .iter()
            .cloned()
            .collect(),
        ),
    ];
    for expected_deposits in expected_deposits_vec {
        assert_deposits(&mango_group_cookie, expected_deposits);
    }

    let expected_values_vec: Vec<(usize, usize, HashMap<&str, I80F48>)> = vec![
        (
            mint_index,        // Mint index
            bidder_user_index, // User index
            [
                ("quote_free", test.to_native(&quote_mint, 0.0)), // maker fee: 0.0% of base_price
                ("quote_locked", ZERO_I80F48),
                ("base_free", test.to_native(&mint, base_size)),
                ("base_locked", ZERO_I80F48),
            ]
            .iter()
            .cloned()
            .collect(),
        ),
        (
            mint_index,       // Mint index
            asker_user_index, // User index
            [
                ("quote_free", test.to_native(&quote_mint, 0.8)), // referrer rebate: 1/5 of taker fee
                ("quote_locked", ZERO_I80F48),
                ("base_free", ZERO_I80F48),
                ("base_locked", ZERO_I80F48),
            ]
            .iter()
            .cloned()
            .collect(),
        ),
    ];

    for expected_values in expected_values_vec {
        assert_user_spot_orders(&mut test, &mango_group_cookie, expected_values).await;
    }
}

#[tokio::test]
async fn test_match_and_settle_spot_order() {
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
    let mint = test.with_mint(mint_index);
    let quote_mint = test.quote_mint;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Deposit amounts
    let user_deposits = vec![
        (bidder_user_index, test.quote_index, base_price),
        (asker_user_index, mint_index, 1.0),
    ];

    // Matched Spot Orders
    let matched_spot_orders = vec![vec![
        (bidder_user_index, mint_index, serum_dex::matching::Side::Bid, base_size, base_price),
        (asker_user_index, mint_index, serum_dex::matching::Side::Ask, base_size, base_price),
    ]];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Place and match spot order
    match_spot_order_scenario(&mut test, &mut mango_group_cookie, &matched_spot_orders).await;

    // Step 3: Settle all spot
    for matched_spot_order in matched_spot_orders {
        mango_group_cookie.settle_spot_funds(&mut test, &matched_spot_order).await;
    }

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let expected_deposits_vec: Vec<(usize, HashMap<usize, I80F48>)> = vec![
        (
            bidder_user_index, // User index
            [
                (mint_index, test.to_native(&mint, 1.0)),
                (QUOTE_INDEX, test.to_native(&quote_mint, 0.0)), // serum_dex fee
            ]
            .iter()
            .cloned()
            .collect(),
        ),
        (
            asker_user_index, // User index
            [
                (mint_index, ZERO_I80F48),
                // Match the fractional I80F48 result, which is not exactly 9982.4
                // The result is 10000, minus taker fee (4), plus referrer rebate (0.8).
                (QUOTE_INDEX, test.to_native_fixedint(&quote_mint, I80F48!(9996.8))),
            ]
            .iter()
            .cloned()
            .collect(),
        ),
    ];
    for expected_deposits in expected_deposits_vec {
        assert_deposits_approx(&mango_group_cookie, expected_deposits, I80F48!(0.0001));
    }

    let expected_values_vec: Vec<(usize, usize, HashMap<&str, I80F48>)> = vec![
        (
            mint_index,        // Mint index
            bidder_user_index, // User index
            [
                ("quote_free", ZERO_I80F48),
                ("quote_locked", ZERO_I80F48),
                ("base_free", ZERO_I80F48),
                ("base_locked", ZERO_I80F48),
            ]
            .iter()
            .cloned()
            .collect(),
        ),
        (
            mint_index,       // Mint index
            asker_user_index, // User index
            [
                ("quote_free", ZERO_I80F48),
                ("quote_locked", ZERO_I80F48),
                ("base_free", ZERO_I80F48),
                ("base_locked", ZERO_I80F48),
            ]
            .iter()
            .cloned()
            .collect(),
        ),
    ];

    for expected_values in expected_values_vec {
        assert_user_spot_orders(&mut test, &mango_group_cookie, expected_values).await;
    }
}

#[tokio::test]
async fn test_cancel_all_spot_orders() {
    let config = MangoProgramTestConfig::default_two_mints();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let user_index: usize = 0;
    let mint_index: usize = 0;
    let base_price: f64 = 10_000.0;
    let base_size: f64 = 1.0;
    let mint = test.with_mint(mint_index);
    let quote_mint = test.quote_mint;
    let mango_account_pk = mango_group_cookie.mango_accounts[user_index].address;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Deposit amounts
    let user_deposits =
        vec![(user_index, test.quote_index, base_price * 10.0), (user_index, mint_index, 10.0)];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;
    let deposit_base = test.with_mango_account_deposit(&mango_account_pk, mint_index).await;
    let deposit_quote = test.with_mango_account_deposit(&mango_account_pk, test.quote_index).await;

    // Step 2: Place spot orders

    // Spot Orders
    let user_spot_orders1 =
        vec![(user_index, mint_index, serum_dex::matching::Side::Bid, base_size, base_price)];
    let user_spot_orders2 = vec![(
        user_index,
        mint_index,
        serum_dex::matching::Side::Bid,
        base_size,
        base_price * 0.95,
    )];
    let user_spot_orders3 = vec![(
        user_index,
        mint_index,
        serum_dex::matching::Side::Ask,
        base_size,
        base_price * 1.10,
    )];

    place_spot_order_scenario(&mut test, &mut mango_group_cookie, &user_spot_orders1).await;
    mango_group_cookie.run_keeper(&mut test).await;
    place_spot_order_scenario(&mut test, &mut mango_group_cookie, &user_spot_orders2).await;
    mango_group_cookie.run_keeper(&mut test).await;
    place_spot_order_scenario(&mut test, &mut mango_group_cookie, &user_spot_orders3).await;
    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let expected_values_vec: Vec<(usize, usize, HashMap<&str, I80F48>)> = vec![(
        mint_index, // Mint index
        user_index, // User index
        [
            ("quote_free", ZERO_I80F48),
            (
                "quote_locked",
                test.to_native(
                    &quote_mint,
                    base_price * base_size + (base_price * 0.95) * base_size,
                ),
            ),
            ("base_free", ZERO_I80F48),
            ("base_locked", test.to_native(&mint, base_size)),
        ]
        .iter()
        .cloned()
        .collect(),
    )];
    for expected_values in expected_values_vec {
        assert_user_spot_orders(&mut test, &mango_group_cookie, expected_values).await;
    }

    // check deposits
    let deposit_base_after_order =
        test.with_mango_account_deposit(&mango_account_pk, mint_index).await;
    let deposit_quote_after_order =
        test.with_mango_account_deposit(&mango_account_pk, test.quote_index).await;
    let expected_quote_diff: u64 = test
        .to_native(&quote_mint, base_price * base_size + (base_price * 0.95) * base_size)
        .to_num();

    assert_eq!(
        deposit_base - deposit_base_after_order,
        test.to_native(&mint, base_size).to_num::<u64>()
    );
    assert_eq!(deposit_quote - deposit_quote_after_order, expected_quote_diff);

    // cancel all orders for a token
    let spot_market_cookie = mango_group_cookie.spot_markets[mint_index];
    test.cancel_all_spot_orders(&mango_group_cookie, &spot_market_cookie, user_index).await;
    mango_group_cookie.run_keeper(&mut test).await;

    let expected_values_vec: Vec<(usize, usize, HashMap<&str, I80F48>)> = vec![(
        mint_index, // Mint index
        user_index, // User index
        [
            ("quote_free", ZERO_I80F48),
            ("quote_locked", ZERO_I80F48),
            ("base_free", ZERO_I80F48),
            ("base_locked", ZERO_I80F48),
        ]
        .iter()
        .cloned()
        .collect(),
    )];

    for expected_values in expected_values_vec {
        assert_user_spot_orders(&mut test, &mango_group_cookie, expected_values).await;
    }
    // check deposits
    let deposit_base_after_cancel =
        test.with_mango_account_deposit(&mango_account_pk, mint_index).await;
    let deposit_quote_after_cancel =
        test.with_mango_account_deposit(&mango_account_pk, test.quote_index).await;
    assert_eq!(deposit_base, deposit_base_after_cancel);
    assert_eq!(deposit_quote, deposit_quote_after_cancel);
}
