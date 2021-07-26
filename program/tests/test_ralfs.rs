// Tests related to placing orders on a perp market
mod program_test;
use mango::{matching::*, state::*};
use program_test::*;
use solana_program::pubkey::Pubkey;
use solana_program_test::*;
use std::num::NonZeroU64;
use std::{mem::size_of, mem::size_of_val};

use serum_dex::instruction::{NewOrderInstructionV3, SelfTradeBehavior};

#[tokio::test]
async fn test_init_perp_markets() {
    // Arrange
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;
    // Disable all logs except error
    solana_logger::setup_with("error");

    let (mango_group_pk, _mango_group) = test.with_mango_group().await;
    // Act
    // Need to add oracles first in order to add perp_markets
    test.add_oracles_to_mango_group(&mango_group_pk).await;
    let (_perp_market_pks, perp_markets) =
        test.add_perp_markets_to_mango_group(&mango_group_pk).await;
    // Assert
    for perp_market in perp_markets {
        assert_eq!(size_of_val(&perp_market), size_of::<PerpMarket>());
    }
}

async fn place_perp_order_scenario(order_id: u64, order_side: Side) {
    // Arrange
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 2, num_mints: 2 };
    let mut test = MangoProgramTest::start_new(&config).await;
    // Disable all logs except error
    solana_logger::setup_with("error");

    let user_index: usize = 0;
    let mint_index: usize = 0;
    let market_index: usize = 0;

    let base_mint = test.with_mint(mint_index);
    let quote_mint = test.with_mint(test.quote_index);

    let base_price = 10000;
    let raw_order_size = 10000;

    let order_price = test.with_order_price(&quote_mint, &base_mint, base_price);
    let order_size = test.with_order_size(&base_mint, raw_order_size);
    let order_type = OrderType::Limit;

    let (mango_group_pk, mango_group) = test.with_mango_group().await;
    let (mango_account_pk, mut mango_account) =
        test.with_mango_account(&mango_group_pk, user_index).await;
    let oracle_pks = test.add_oracles_to_mango_group(&mango_group_pk).await;
    let (perp_market_pks, perp_markets) =
        test.add_perp_markets_to_mango_group(&mango_group_pk).await;

    let deposit_amount = (base_price * quote_mint.unit) as u64;
    test.perform_deposit(
        &mango_group,
        &mango_group_pk,
        &mango_account_pk,
        user_index,
        test.quote_index,
        deposit_amount,
    )
    .await;

    // Act
    test.place_perp_order(
        &mango_group,
        &mango_group_pk,
        &mango_account,
        &mango_account_pk,
        &perp_markets[market_index],
        &perp_market_pks[market_index],
        order_side,
        order_price,
        order_size,
        order_id,
        order_type,
        &oracle_pks,
        user_index,
    )
    .await
    .unwrap();

    // Assert
    mango_account = test.load_account::<MangoAccount>(mango_account_pk).await;
    let (client_order_id, _order_id, side) = mango_account.perp_accounts[market_index]
        .open_orders
        .orders_with_client_ids()
        .last()
        .unwrap();
    assert_eq!(client_order_id, NonZeroU64::new(order_id).unwrap());
    assert_eq!(side, order_side);
}

#[tokio::test]
async fn test_place_perp_order() {
    // Scenario 1
    place_perp_order_scenario(1212, Side::Bid).await;
    // Scenario 2
    place_perp_order_scenario(1212, Side::Ask).await;
}

#[tokio::test]
async fn test_list_spot_market_on_serum() {
    // Arrange
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;
    // Disable all logs except error
    solana_logger::setup_with("error");

    let mint_index: usize = 0;
    // Act
    let market_pubkeys = test.list_spot_market(mint_index).await.unwrap();
    // Assert
    println!("Serum Market PK: {}", market_pubkeys.market.to_string());
    // Todo: Figure out how to assert this
}

#[tokio::test]
async fn test_init_spot_markets() {
    // Arrange
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;
    // Disable all logs except error
    solana_logger::setup_with("error");

    let (mango_group_pk, _mango_group) = test.with_mango_group().await;
    test.add_oracles_to_mango_group(&mango_group_pk).await;
    test.add_spot_markets_to_mango_group(&mango_group_pk).await;
}

#[tokio::test]
async fn test_place_spot_order() {
    // Arrange
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 2, num_mints: 2 };
    let mut test = MangoProgramTest::start_new(&config).await;
    // Disable all logs except error
    solana_logger::setup_with("error");

    let user_index: usize = 0;
    let mint_index: usize = 0;

    let base_mint = test.with_mint(mint_index);
    let quote_mint = test.with_mint(test.quote_index);

    let base_price = 10000;

    let (mango_group_pk, mut mango_group) = test.with_mango_group().await;
    let (mango_account_pk, mut mango_account) =
        test.with_mango_account(&mango_group_pk, user_index).await;
    let (mango_cache_pk, _mango_cache) = test.with_mango_cache(&mango_group).await;
    let oracle_pks = test.add_oracles_to_mango_group(&mango_group_pk).await;

    let oracle_price = test.with_oracle_price(&base_mint, base_price as u64);
    test.set_oracle(&mango_group, &mango_group_pk, &oracle_pks[mint_index], oracle_price).await;
    let spot_markets = test.add_spot_markets_to_mango_group(&mango_group_pk).await;
    mango_group = test.load_account::<MangoGroup>(mango_group_pk).await;

    let deposit_amount = (base_price * quote_mint.unit) as u64;
    test.perform_deposit(
        &mango_group,
        &mango_group_pk,
        &mango_account_pk,
        user_index,
        test.quote_index,
        deposit_amount,
    )
    .await;

    // Act
    let order = NewOrderInstructionV3 {
        side: serum_dex::matching::Side::Bid,
        limit_price: NonZeroU64::new(base_price as u64).unwrap(),
        max_coin_qty: NonZeroU64::new(1).unwrap(),
        max_native_pc_qty_including_fees: NonZeroU64::new(base_price as u64).unwrap(),
        self_trade_behavior: SelfTradeBehavior::DecrementTake,
        order_type: serum_dex::matching::OrderType::Limit,
        client_order_id: 1000,
        limit: u16::MAX,
    };
    test.place_spot_order(
        &mango_group_pk,
        &mango_group,
        &mango_account_pk,
        &mango_account,
        &mango_cache_pk,
        spot_markets[mint_index],
        &oracle_pks,
        user_index,
        mint_index,
        order,
    )
    .await;

    // Assert
    mango_account = test.load_account::<MangoAccount>(mango_account_pk).await;
    assert_ne!(mango_account.spot_open_orders[0], Pubkey::default());
    // TODO: More assertions
}

#[tokio::test]
async fn test_worst_case_scenario() {
    // Arrange
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 2, num_mints: 16 };
    let mut test = MangoProgramTest::start_new(&config).await;
    // Supress some of the logs
    solana_logger::setup_with_default(
        "solana_rbpf::vm=info,\
             solana_runtime::message_processor=debug,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info",
    );

    // num_orders specifies how many orders you want to perform to spot and perp markets
    // If this number is larger than test.num_mints - 1 you will get an out_of_bounds error
    let num_orders: usize = test.num_mints - 1;
    let user_index: usize = 0;

    let quote_mint = test.with_mint(test.quote_index);

    let base_price = 10000;

    let (mango_group_pk, mut mango_group) = test.with_mango_group().await;
    let (mango_account_pk, mut mango_account) =
        test.with_mango_account(&mango_group_pk, user_index).await;
    let (mango_cache_pk, _mango_cache) = test.with_mango_cache(&mango_group).await;

    let oracle_pks = test.add_oracles_to_mango_group(&mango_group_pk).await;
    let spot_markets = test.add_spot_markets_to_mango_group(&mango_group_pk).await;
    let (perp_market_pks, perp_markets) =
        test.add_perp_markets_to_mango_group(&mango_group_pk).await;
    test.cache_all_perp_markets(&mango_group, &mango_group_pk, &perp_market_pks).await;
    // Need to reload mango group because add_spot_markets add tokens in to mango_group
    mango_group = test.load_account::<MangoGroup>(mango_group_pk).await;

    // Act
    // Perform deposit for the quote for as many orders as we make
    let deposit_amount = (base_price * quote_mint.unit) as u64;
    test.perform_deposit(
        &mango_group,
        &mango_group_pk,
        &mango_account_pk,
        user_index,
        test.quote_index,
        deposit_amount * num_orders as u64,
    )
    .await;

    // Perform deposit for the rest of tokens just so we have open deposits
    for mint_index in 0..num_orders {
        let base_mint = test.with_mint(mint_index);
        let mint_deposit_amount = (1 * base_mint.unit) as u64;
        test.perform_deposit(
            &mango_group,
            &mango_group_pk,
            &mango_account_pk,
            user_index,
            mint_index,
            mint_deposit_amount,
        )
        .await;
    }

    // Place `num_orders` spot orders
    let starting_order_id = 1000;
    for mint_index in 0..num_orders.min(MAX_NUM_IN_MARGIN_BASKET as usize) {
        let base_mint = test.with_mint(mint_index);
        let oracle_price = test.with_oracle_price(&base_mint, base_price as u64);
        test.set_oracle(&mango_group, &mango_group_pk, &oracle_pks[mint_index], oracle_price).await;
        let order = NewOrderInstructionV3 {
            side: serum_dex::matching::Side::Bid,
            limit_price: NonZeroU64::new(base_price as u64).unwrap(),
            max_coin_qty: NonZeroU64::new(1).unwrap(),
            max_native_pc_qty_including_fees: NonZeroU64::new(base_price as u64).unwrap(),
            self_trade_behavior: SelfTradeBehavior::DecrementTake,
            order_type: serum_dex::matching::OrderType::Limit,
            client_order_id: starting_order_id + mint_index as u64,
            limit: std::u16::MAX,
        };
        test.place_spot_order(
            &mango_group_pk,
            &mango_group,
            &mango_account_pk,
            &mango_account,
            &mango_cache_pk,
            spot_markets[mint_index],
            &oracle_pks,
            user_index,
            mint_index,
            order,
        )
        .await;
        mango_account = test.load_account::<MangoAccount>(mango_account_pk).await;
    }

    mango_account = test.load_account::<MangoAccount>(mango_account_pk).await;

    // Place `num_orders` perp orders
    let starting_perp_order_id = 2000;
    for mint_index in 0..num_orders {
        let base_mint = test.with_mint(mint_index);

        let order_side = Side::Bid;
        let order_price = test.with_order_price(&quote_mint, &base_mint, base_price);
        let order_size = test.with_order_size(&base_mint, 1);
        let order_type = OrderType::Limit;

        test.place_perp_order(
            &mango_group,
            &mango_group_pk,
            &mango_account,
            &mango_account_pk,
            &perp_markets[mint_index],
            &perp_market_pks[mint_index],
            order_side,
            order_price,
            order_size,
            starting_perp_order_id + mint_index as u64,
            order_type,
            &oracle_pks,
            user_index,
        )
        .await
        .unwrap();

        mango_account = test.load_account::<MangoAccount>(mango_account_pk).await;
    }

    // Assert
    mango_account = test.load_account::<MangoAccount>(mango_account_pk).await;
    for spot_open_orders_index in 0..num_orders.min(MAX_NUM_IN_MARGIN_BASKET as usize) {
        assert_ne!(mango_account.spot_open_orders[spot_open_orders_index], Pubkey::default());
    }
    // TODO: more assertions
}

#[tokio::test]
async fn test_worst_case_scenario_with_fractions() {
    // Arrange
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 2, num_mints: 2 };
    let mut test = MangoProgramTest::start_new(&config).await;
    // Supress some of the logs
    solana_logger::setup_with_default(
        "solana_rbpf::vm=info,\
             solana_runtime::message_processor=debug,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info",
    );

    let borrower_user_index: usize = 0;
    let lender_user_index: usize = 1;
    let mint_index: usize = 0;
    let base_mint = test.with_mint(mint_index);
    let base_price = 10000;

    let (mango_group_pk, mut mango_group) = test.with_mango_group().await;
    let (borrower_mango_account_pk, mut borrower_mango_account) =
        test.with_mango_account(&mango_group_pk, borrower_user_index).await;
    let (lender_mango_account_pk, mut lender_mango_account) =
        test.with_mango_account(&mango_group_pk, lender_user_index).await;
    let (mut mango_cache_pk, mut mango_cache) = test.with_mango_cache(&mango_group).await;

    let oracle_pks = test.add_oracles_to_mango_group(&mango_group_pk).await;
    let spot_markets = test.add_spot_markets_to_mango_group(&mango_group_pk).await;
    let (perp_market_pks, perp_markets) =
        test.add_perp_markets_to_mango_group(&mango_group_pk).await;
    test.cache_all_perp_markets(&mango_group, &mango_group_pk, &perp_market_pks).await;
    // Need to reload mango group because `add_spot_markets` adds tokens in to mango_group
    mango_group = test.load_account::<MangoGroup>(mango_group_pk).await;
    let oracle_price = test.with_oracle_price(&base_mint, base_price as u64);
    test.set_oracle(&mango_group, &mango_group_pk, &oracle_pks[mint_index], oracle_price).await;

    // Act

    // Step 1: Make deposits from 2 accounts (Borrower / Lender)
    // Deposit 100_000 USDC as the borrower
    let quote_deposit_amount = (100_000 * test.quote_mint.unit) as u64;
    test.perform_deposit(
        &mango_group,
        &mango_group_pk,
        &borrower_mango_account_pk,
        borrower_user_index,
        test.quote_index,
        quote_deposit_amount,
    )
    .await;

    // Deposit 10 BTC as the lender
    let base_deposit_amount = (10 * base_mint.unit) as u64;
    test.perform_deposit(
        &mango_group,
        &mango_group_pk,
        &lender_mango_account_pk,
        lender_user_index,
        mint_index,
        base_deposit_amount,
    )
    .await;

    // Step 2: Withdraw (with borrow) 1 BTC @ 10000 as the borrower
    test.update_all_root_banks(&mango_group, &mango_group_pk, 0).await;
    test.cache_all_root_banks(&mango_group, &mango_group_pk).await;
    test.cache_all_prices(&mango_group, &mango_group_pk, &oracle_pks).await;

    let withdraw_amount = (1 * base_mint.unit) as u64;
    test.perform_withdraw(
        &mango_group_pk,
        &mango_group,
        &borrower_mango_account_pk,
        &borrower_mango_account,
        borrower_user_index,
        mint_index,
        withdraw_amount,
        true, // Allow borrow
    )
    .await;

    // Step 3: Advance clock and update root banks causing the deposits and borrows to change
    test.advance_clock().await;
    test.update_all_root_banks(&mango_group, &mango_group_pk, 0).await;
    test.cache_all_root_banks(&mango_group, &mango_group_pk).await;

    // Step 4: Check that lenders deposit is not a nice number anymore (!= 2 BTC)
    mango_cache = test.load_account::<MangoCache>(mango_cache_pk).await;
    lender_mango_account = test.load_account::<MangoAccount>(lender_mango_account_pk).await;
    let lender_account_deposit = lender_mango_account.get_native_deposit(&mango_cache.root_bank_cache[0], 0).unwrap().to_string();
    println!("Lender Deposits: {}", lender_account_deposit);

    // Step 5: Place a spot order for BTC
    let starting_order_id = 1000;
    let order = NewOrderInstructionV3 {
        side: serum_dex::matching::Side::Ask,
        limit_price: NonZeroU64::new(base_price as u64).unwrap(),
        max_coin_qty: NonZeroU64::new(1).unwrap(),
        max_native_pc_qty_including_fees: NonZeroU64::new(base_price as u64).unwrap(),
        self_trade_behavior: SelfTradeBehavior::DecrementTake,
        order_type: serum_dex::matching::OrderType::Limit,
        client_order_id: starting_order_id + mint_index as u64,
        limit: std::u16::MAX,
    };
    test.place_spot_order(
        &mango_group_pk,
        &mango_group,
        &lender_mango_account_pk,
        &lender_mango_account,
        &mango_cache_pk,
        spot_markets[mint_index],
        &oracle_pks,
        lender_user_index,
        mint_index,
        order,
    )
    .await;
    // TODO: Maybe make into a loop to stress test
    // Assert
    lender_mango_account = test.load_account::<MangoAccount>(lender_mango_account_pk).await;
    println!("SOO: {}", lender_mango_account.spot_open_orders[mint_index].to_string());
    assert_ne!(lender_mango_account.spot_open_orders[mint_index], Pubkey::default());
}

#[tokio::test]
async fn test_worst_case_scenario_with_fractions_x10() {
    // Arrange
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 2, num_mints: 11 };
    let mut test = MangoProgramTest::start_new(&config).await;
    // Supress some of the logs
    solana_logger::setup_with_default(
        "solana_rbpf::vm=info,\
             solana_runtime::message_processor=debug,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info",
    );

    let borrower_user_index: usize = 0;
    let lender_user_index: usize = 1;
    let mint_index: usize = 0;
    let num_orders: usize = test.num_mints - 1;
    let base_price = 10000;

    let (mango_group_pk, mut mango_group) = test.with_mango_group().await;
    let (borrower_mango_account_pk, mut borrower_mango_account) =
        test.with_mango_account(&mango_group_pk, borrower_user_index).await;
    let (lender_mango_account_pk, mut lender_mango_account) =
        test.with_mango_account(&mango_group_pk, lender_user_index).await;
    let (mut mango_cache_pk, mut mango_cache) = test.with_mango_cache(&mango_group).await;

    let oracle_pks = test.add_oracles_to_mango_group(&mango_group_pk).await;
    let spot_markets = test.add_spot_markets_to_mango_group(&mango_group_pk).await;
    let (perp_market_pks, perp_markets) =
        test.add_perp_markets_to_mango_group(&mango_group_pk).await;
    test.cache_all_perp_markets(&mango_group, &mango_group_pk, &perp_market_pks).await;
    // Need to reload mango group because `add_spot_markets` adds tokens in to mango_group
    mango_group = test.load_account::<MangoGroup>(mango_group_pk).await;

    // Act

    // Step 1: Deposit 100_000 USDC as the borrower for collateral
    let quote_deposit_amount = (110_000 * test.quote_mint.unit) as u64;
    test.perform_deposit(
        &mango_group,
        &mango_group_pk,
        &borrower_mango_account_pk,
        borrower_user_index,
        test.quote_index,
        quote_deposit_amount,
    )
    .await;

    // Step 2: Deposit 10 of each mint as the lender
    for mint_index in 0..num_orders {
        let base_mint = test.with_mint(mint_index);
        let base_deposit_amount = (10 * base_mint.unit) as u64;
        // Set oracle price for each mint
        let oracle_price = test.with_oracle_price(&base_mint, base_price as u64);
        test.set_oracle(&mango_group, &mango_group_pk, &oracle_pks[mint_index], oracle_price).await;

        test.perform_deposit(
            &mango_group,
            &mango_group_pk,
            &lender_mango_account_pk,
            lender_user_index,
            mint_index,
            base_deposit_amount,
        )
        .await;
    }

    // Step 3: Update and cache everything
    test.update_all_root_banks(&mango_group, &mango_group_pk, 0).await;
    test.cache_all_root_banks(&mango_group, &mango_group_pk).await;
    test.cache_all_prices(&mango_group, &mango_group_pk, &oracle_pks).await;

    // Step 4: Withdraw (with borrow) 1 of each mint @ 10000 as the borrower
    for mint_index in 0..num_orders {
        let base_mint = test.with_mint(mint_index);
        let withdraw_amount = (1 * base_mint.unit) as u64;
        test.perform_withdraw(
            &mango_group_pk,
            &mango_group,
            &borrower_mango_account_pk,
            &borrower_mango_account,
            borrower_user_index,
            mint_index,
            withdraw_amount,
            true, // Allow borrow
        )
        .await;
    }

    // Step 5: Advance clock and update root banks causing the deposits and borrows to change
    test.advance_clock().await;
    test.update_all_root_banks(&mango_group, &mango_group_pk, 0).await;
    test.cache_all_root_banks(&mango_group, &mango_group_pk).await;

    // Step 6: Check that lenders deposit is not a nice number anymore (!= 2 BTC)
    mango_cache = test.load_account::<MangoCache>(mango_cache_pk).await;
    lender_mango_account = test.load_account::<MangoAccount>(lender_mango_account_pk).await;
    let lender_account_deposit = lender_mango_account.get_native_deposit(&mango_cache.root_bank_cache[0], 0).unwrap().to_string();
    // TODO: Assert all deposits > 10

    // Step 7: Place a spot order ASK for each mint
    let starting_order_id = 1000;
    for mint_index in 0..num_orders {
        let order = NewOrderInstructionV3 {
            side: serum_dex::matching::Side::Ask,
            limit_price: NonZeroU64::new(base_price as u64).unwrap(),
            max_coin_qty: NonZeroU64::new(1).unwrap(),
            max_native_pc_qty_including_fees: NonZeroU64::new(base_price as u64).unwrap(),
            self_trade_behavior: SelfTradeBehavior::DecrementTake,
            order_type: serum_dex::matching::OrderType::Limit,
            client_order_id: starting_order_id + mint_index as u64,
            limit: std::u16::MAX,
        };
        test.place_spot_order(
            &mango_group_pk,
            &mango_group,
            &lender_mango_account_pk,
            &lender_mango_account,
            &mango_cache_pk,
            spot_markets[mint_index],
            &oracle_pks,
            lender_user_index,
            mint_index,
            order,
        )
        .await;
        lender_mango_account = test.load_account::<MangoAccount>(lender_mango_account_pk).await;
    }
    // Assert
    lender_mango_account = test.load_account::<MangoAccount>(lender_mango_account_pk).await;
    for spot_open_orders_index in 0..num_orders.min(MAX_NUM_IN_MARGIN_BASKET as usize) {
        assert_ne!(lender_mango_account.spot_open_orders[spot_open_orders_index], Pubkey::default());
    }
}
