use fixed::types::I80F48;
use fixed_macro::types::I80F48;
use mango::state::QUOTE_INDEX;
use solana_program_test::*;

use crate::assertions::EPSILON;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;

// Tests related to liquidations
mod program_test;

pub fn get_deposit_for_user(
    mango_group_cookie: &MangoGroupCookie,
    user_index: usize,
    mint_index: usize,
) -> I80F48 {
    mango_group_cookie.mango_accounts[user_index]
        .mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index)
        .unwrap()
}

pub fn get_borrow_for_user(
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
async fn test_token_and_token_liquidation_v1() {
    // === Arrange ===
    let config =
        MangoProgramTestConfig { num_users: 3, ..MangoProgramTestConfig::default_two_mints() };

    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let bidder_user_index: usize = 0;
    let asker_user_index: usize = 1;
    let liqor_user_index: usize = 2;
    let mint_index: usize = 0;
    let base_price: f64 = 15_000.0;
    let base_size: f64 = 1.0;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Deposit amounts
    let user_deposits = vec![
        (bidder_user_index, test.quote_index, 10_000.0),
        (asker_user_index, mint_index, 1.0),
        (asker_user_index, test.quote_index, 10_000.0),
        (liqor_user_index, test.quote_index, 10_000.0),
    ];

    // Matched Spot Orders
    let matched_spot_orders = vec![vec![
        (bidder_user_index, mint_index, serum_dex::matching::Side::Bid, base_size, base_price),
        (asker_user_index, mint_index, serum_dex::matching::Side::Ask, base_size, base_price),
    ]];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Place and match an order for 1 BTC @ 15_000
    match_spot_order_scenario(&mut test, &mut mango_group_cookie, &matched_spot_orders).await;

    // Step 3: Settle all spot order
    for matched_spot_order in matched_spot_orders {
        mango_group_cookie.settle_spot_funds(&mut test, &matched_spot_order).await;
    }

    // Step 4: Assert that the order has been matched and the bidder has 1 BTC in deposits
    mango_group_cookie.run_keeper(&mut test).await;

    // assert that bidder has btc deposit and quote borrows
    let bidder_btc_deposit =
        get_deposit_for_user(&mango_group_cookie, bidder_user_index, mint_index);
    let bidder_quote_borrow =
        get_borrow_for_user(&mango_group_cookie, bidder_user_index, QUOTE_INDEX);
    // dbg!(bidder_btc_deposit);
    // dbg!(bidder_quote_borrow);
    // [program/tests/test_liquidation_token_and_token:92] bidder_btc_deposit = 1000000
    // [program/tests/test_liquidation_token_and_token:93] bidder_quote_borrow = 5000000000
    assert!(bidder_btc_deposit == I80F48!(1000000));
    assert!(bidder_quote_borrow == I80F48!(5000000000));

    // assert that liqor has no btc deposit and full quote deposits
    let liqor_btc_deposit = get_deposit_for_user(&mango_group_cookie, liqor_user_index, mint_index);
    let liqor_quote_deposit =
        get_deposit_for_user(&mango_group_cookie, liqor_user_index, QUOTE_INDEX);
    // dbg!(liqor_btc_deposit);
    // dbg!(liqor_quote_deposit);
    // [program/tests/test_liquidation_token_and_token.rs:101] liqor_btc_deposit = 0
    // [program/tests/test_liquidation_token_and_token.rs:102] liqor_quote_deposit = 10000000000
    assert!(liqor_btc_deposit.is_zero());
    assert!(liqor_quote_deposit == I80F48!(10000000000));

    // Step 5: Change the oracle price so that bidder becomes liqee
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price / 15.0).await;

    // Step 6: Perform a couple of liquidations
    for _ in 0..6 {
        mango_group_cookie.run_keeper(&mut test).await;
        test.perform_liquidate_token_and_token(
            &mut mango_group_cookie,
            bidder_user_index, // The liqee
            liqor_user_index,
            mint_index,  // Asset index
            QUOTE_INDEX, // Liab index
            I80F48!(10_000),
        )
        .await;
    }

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    // assert that bidders btc deposits and quote borrows have reduced
    let bidder_btc_deposit =
        get_deposit_for_user(&mango_group_cookie, bidder_user_index, mint_index);
    let bidder_quote_borrow =
        get_borrow_for_user(&mango_group_cookie, bidder_user_index, QUOTE_INDEX);
    // dbg!(bidder_btc_deposit);
    // dbg!(bidder_quote_borrow);
    // [program/tests/test_liquidation_token_and_token:123] bidder_btc_deposit = 999938.5000000060586
    // [program/tests/test_liquidation_token_and_token:124] bidder_quote_borrow = 4999940000.000000011937118
    assert_approx_eq!(bidder_btc_deposit, I80F48!(999938.5), I80F48::ONE);
    assert_approx_eq!(bidder_quote_borrow, I80F48!(4999940000), I80F48::ONE);

    // assert that liqors btc deposits have increased and quote deposits have reduced
    let liqor_btc_deposit = get_deposit_for_user(&mango_group_cookie, liqor_user_index, mint_index);
    let liqor_quote_deposit =
        get_deposit_for_user(&mango_group_cookie, liqor_user_index, QUOTE_INDEX);
    // dbg!(liqor_btc_deposit);
    // dbg!(liqor_quote_deposit);
    // [program/tests/test_liquidation_token_and_token:125] liqor_btc_deposit = 61.4999999939414
    // [program/tests/test_liquidation_token_and_token:126] liqor_quote_deposit = 9999940000.000000011937118
    assert_approx_eq!(liqor_btc_deposit, I80F48!(61.5), I80F48::ONE);
    assert_approx_eq!(liqor_quote_deposit, I80F48!(9999940000), I80F48::ONE);
}

#[tokio::test]
async fn test_token_and_token_liquidation_v2() {
    // === Arrange ===
    let config = MangoProgramTestConfig { num_users: 3, ..MangoProgramTestConfig::default() };
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let bidder_user_index: usize = 0;
    let asker_user_index: usize = 1;
    let liqor_user_index: usize = 2;
    let num_orders: usize = test.num_mints - 1;
    let base_price: f64 = 15_000.0;
    let base_size: f64 = 2.0;
    let liq_mint_index: usize = 0;
    // TODO: Make the order prices into variables

    // Set oracles
    for mint_index in 0..num_orders {
        mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;
    }

    // Deposit amounts
    let mut user_deposits = vec![
        (asker_user_index, liq_mint_index, 2.0),
        (asker_user_index, test.quote_index, 100_000.0),
        (liqor_user_index, test.quote_index, 10_000.0),
    ];
    user_deposits.extend(arrange_deposit_all_scenario(&mut test, bidder_user_index, 1.0, 0.0));

    // // Perp Orders
    let mut user_perp_orders = vec![];
    for mint_index in 0..num_orders {
        user_perp_orders.push((
            bidder_user_index,
            mint_index,
            mango::matching::Side::Ask,
            1.0,
            base_price,
        ));
    }

    // Matched Spot Orders
    let matched_spot_orders = vec![vec![
        (bidder_user_index, liq_mint_index, serum_dex::matching::Side::Bid, base_size, base_price),
        (asker_user_index, liq_mint_index, serum_dex::matching::Side::Ask, base_size, base_price),
    ]];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Place perp orders
    place_perp_order_scenario(&mut test, &mut mango_group_cookie, &user_perp_orders).await;

    // Step 3: Place and match an order for 1 BTC @ 15_000
    match_spot_order_scenario(&mut test, &mut mango_group_cookie, &matched_spot_orders).await;

    // Step 4: Settle all spot orders
    for matched_spot_order in matched_spot_orders {
        mango_group_cookie.settle_spot_funds(&mut test, &matched_spot_order).await;
    }

    // Step 5: Assert that the order has been matched and the bidder has 3 BTC in deposits
    mango_group_cookie.run_keeper(&mut test).await;

    let bidder_base_deposit = mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .get_native_deposit(
            &mango_group_cookie.mango_cache.root_bank_cache[liq_mint_index],
            liq_mint_index,
        )
        .unwrap();
    let asker_base_deposit = mango_group_cookie.mango_accounts[asker_user_index]
        .mango_account
        .get_native_deposit(
            &mango_group_cookie.mango_cache.root_bank_cache[liq_mint_index],
            liq_mint_index,
        )
        .unwrap();
    assert_eq!(bidder_base_deposit, I80F48!(3_000_000));
    assert_eq!(asker_base_deposit, I80F48::ZERO);

    // Step 6: Change the oracle price so that bidder becomes liqee
    for mint_index in 0..num_orders {
        mango_group_cookie.set_oracle(&mut test, mint_index, 1000.0).await;
    }

    // Step 7: Force cancel perp orders
    mango_group_cookie.run_keeper(&mut test).await;
    for mint_index in 0..num_orders {
        let perp_market_cookie = mango_group_cookie.perp_markets[mint_index];
        test.force_cancel_perp_orders(&mango_group_cookie, &perp_market_cookie, bidder_user_index)
            .await;
    }

    // Step 8: Perform a couple liquidations
    for _ in 0..5 {
        mango_group_cookie.run_keeper(&mut test).await;
        test.perform_liquidate_token_and_token(
            &mut mango_group_cookie,
            bidder_user_index, // The liqee
            liqor_user_index,
            liq_mint_index, // Asset index
            QUOTE_INDEX,    // Liab index
            I80F48!(100_000_000),
        )
        .await;
    }

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let bidder_base_net = mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .get_net(&mango_group_cookie.mango_cache.root_bank_cache[liq_mint_index], liq_mint_index);
    let liqor_base_net = mango_group_cookie.mango_accounts[liqor_user_index]
        .mango_account
        .get_net(&mango_group_cookie.mango_cache.root_bank_cache[liq_mint_index], liq_mint_index);

    let bidder_quote_net =
        mango_group_cookie.mango_accounts[bidder_user_index].mango_account.get_net(
            &mango_group_cookie.mango_cache.root_bank_cache[test.quote_index],
            test.quote_index,
        );
    let liqor_quote_net =
        mango_group_cookie.mango_accounts[liqor_user_index].mango_account.get_net(
            &mango_group_cookie.mango_cache.root_bank_cache[test.quote_index],
            test.quote_index,
        );

    assert_approx_eq!(bidder_base_net, I80F48!(2487500), EPSILON);
    assert_approx_eq!(liqor_base_net, I80F48!(512500), EPSILON);
    assert_eq!(bidder_quote_net, I80F48!(-29500000000));
    assert_eq!(liqor_quote_net, I80F48!(9500000000));

    // TODO: Actually assert here
}
