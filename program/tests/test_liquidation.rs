// Tests related to liquidations
mod program_test;
use fixed::types::I80F48;
use mango::state::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;

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

    let bidder_base_deposit = &mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index)
        .unwrap();
    let asker_base_deposit = &mango_group_cookie.mango_accounts[asker_user_index]
        .mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index)
        .unwrap();

    assert_eq!(bidder_base_deposit.to_string(), I80F48::from_num(1000000).to_string());
    assert_eq!(asker_base_deposit.to_string(), I80F48::from_num(0).to_string());

    // Step 5: Change the oracle price so that bidder becomes liqee
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price / 15.0).await;

    // Step 6: Perform a coulple liquidations
    for _ in 0..5 {
        mango_group_cookie.run_keeper(&mut test).await;
        test.perform_liquidate_token_and_token(
            &mut mango_group_cookie,
            bidder_user_index, // The liqee
            liqor_user_index,
            mint_index,  // Asset index
            QUOTE_INDEX, // Liab index
        )
        .await;
    }

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let bidder_base_deposit = &mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index)
        .unwrap();
    let liqor_base_deposit = &mango_group_cookie.mango_accounts[liqor_user_index]
        .mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index)
        .unwrap();

    let bidder_base_borrow = &mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .get_native_borrow(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index)
        .unwrap();
    let liqor_base_borrow = &mango_group_cookie.mango_accounts[liqor_user_index]
        .mango_account
        .get_native_borrow(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index)
        .unwrap();

    println!("bidder_base_deposit: {}", bidder_base_deposit.to_string());
    println!("liqor_base_deposit: {}", liqor_base_deposit.to_string());
    println!("bidder_base_borrow: {}", bidder_base_borrow.to_string());
    println!("liqor_base_borrow: {}", liqor_base_borrow.to_string());
    // TODO: Actually assert here
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
        (asker_user_index, test.quote_index, 10_000.0),
        (liqor_user_index, test.quote_index, 10_000.0),
    ];
    user_deposits.extend(arrange_deposit_all_scenario(&mut test, bidder_user_index, 1.0, 10_000.0));

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

    let bidder_base_deposit = &mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[liq_mint_index], 0)
        .unwrap();
    let asker_base_deposit = &mango_group_cookie.mango_accounts[asker_user_index]
        .mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[liq_mint_index], 0)
        .unwrap();

    assert_eq!(bidder_base_deposit.to_string(), I80F48::from_num(3000000).to_string());
    assert_eq!(asker_base_deposit.to_string(), I80F48::from_num(0).to_string());

    // Step 6: Change the oracle price so that bidder becomes liqee
    for mint_index in 0..num_orders {
        mango_group_cookie.set_oracle(&mut test, mint_index, 0.0000000001).await;
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
        )
        .await;
    }

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let bidder_base_deposit = &mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[liq_mint_index], 0)
        .unwrap();
    let liqor_base_deposit = &mango_group_cookie.mango_accounts[liqor_user_index]
        .mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[liq_mint_index], 0)
        .unwrap();

    let bidder_base_borrow = &mango_group_cookie.mango_accounts[bidder_user_index]
        .mango_account
        .get_native_borrow(&mango_group_cookie.mango_cache.root_bank_cache[liq_mint_index], 0)
        .unwrap();
    let liqor_base_borrow = &mango_group_cookie.mango_accounts[liqor_user_index]
        .mango_account
        .get_native_borrow(&mango_group_cookie.mango_cache.root_bank_cache[liq_mint_index], 0)
        .unwrap();

    println!("bidder_base_deposit: {}", bidder_base_deposit.to_string());
    println!("liqor_base_deposit: {}", liqor_base_deposit.to_string());
    println!("bidder_base_borrow: {}", bidder_base_borrow.to_string());
    println!("liqor_base_borrow: {}", liqor_base_borrow.to_string());
    // TODO: Actually assert here
}
