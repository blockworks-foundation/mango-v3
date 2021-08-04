// Tests related to placing orders on a perp market
mod program_test;
use mango::{matching::*, state::*};
use program_test::*;
use program_test::cookies::*;
use solana_program::pubkey::Pubkey;
use solana_program_test::*;
use std::num::NonZeroU64;
use std::{mem::size_of, mem::size_of_val};
use fixed::types::I80F48;

use serum_dex::instruction::{NewOrderInstructionV3, SelfTradeBehavior};

#[tokio::test]
async fn test_init_perp_markets() {
    // Arrange
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;
    // Supress some of the logs
    solana_logger::setup_with_default(
        "solana_rbpf::vm=info,\
             solana_runtime::message_processor=debug,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info",
    );
    // Disable all logs except error
    // solana_logger::setup_with("error");

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;

    // Act
    // Need to add oracles first in order to add perp_markets
    test.add_oracles_to_mango_group(&mango_group_cookie.address.unwrap()).await;
    let perp_market_cookies =
        mango_group_cookie.add_perp_markets(&mut test, config.num_mints - 1).await;
    mango_group_cookie.mango_group =
        Some(test.load_account::<MangoGroup>(mango_group_cookie.address.unwrap()).await);
    // Assert
    mango_group_cookie.run_keeper(&mut test).await;

    for perp_market_cookie in perp_market_cookies {
        assert_eq!(size_of_val(&perp_market_cookie.perp_market), size_of::<PerpMarket>());
    }
}

async fn place_perp_order_scenario(order_id: u64, order_side: Side) {
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
    // // Disable all logs except error
    // solana_logger::setup_with("error");

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    let user_index: usize = 0;
    let mint_index: usize = 0;
    let base_mint = test.with_mint(mint_index);
    let base_price = 10000;

    // Act
    // Step 1: Deposit 10_000 USDC into mango account
    mango_group_cookie.run_keeper(&mut test).await;

    let deposit_amount = (base_price * test.quote_mint.unit) as u64;
    test.perform_deposit(
        &mango_group_cookie,
        user_index,
        test.quote_index,
        deposit_amount,
    ).await;

    // Step 2: Place a perp order for 1 BTC @ 10_000
    mango_group_cookie.run_keeper(&mut test).await;

    let mut perp_market_cookie = mango_group_cookie.perp_markets[mint_index];
    perp_market_cookie.place_order(
        &mut test,
        &mut mango_group_cookie,
        user_index,
        order_id,
        order_side,
        1,
        base_price,
    ).await;

    // Assert
    mango_group_cookie.run_keeper(&mut test).await;

    let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account.unwrap();
    let (client_order_id, _order_id, side) = mango_account.perp_accounts[mint_index]
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
    // Disable all logs except error
    // solana_logger::setup_with("error");

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // num_orders specifies how many orders you want to perform to spot and perp markets
    // If this number is larger than test.num_mints - 1 you will get an out_of_bounds error
    let num_orders: usize = test.num_mints - 1;
    let user_index: usize = 0;
    let base_price = 10000;

    // Act
    // Step 1: Set oracles for all the markets
    mango_group_cookie.run_keeper(&mut test).await;

    for mint_index in 0..num_orders.min(MAX_NUM_IN_MARGIN_BASKET as usize) {
        mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;
    }

    // Step 2: Deposit all tokens into mango account
    mango_group_cookie.run_keeper(&mut test).await;

    for mint_index in 0..test.num_mints {
        let mint = test.with_mint(mint_index);

        // Deposit quote mint for the regular deposit * num_orders
        let mint_deposit_amount = if mint_index == test.quote_index {
            (base_price * test.quote_mint.unit) * (num_orders as u64)
        } else {
            1 * mint.unit
        };

        test.perform_deposit(
            &mango_group_cookie,
            user_index,
            mint_index,
            mint_deposit_amount as u64,
        ).await;
    }

    // Step 3: Place `num_orders` spot orders
    mango_group_cookie.run_keeper(&mut test).await;

    let starting_spot_order_id = 1000;
    for mint_index in 0..num_orders.min(MAX_NUM_IN_MARGIN_BASKET as usize) {
        let mut spot_market_cookie = mango_group_cookie.spot_markets[mint_index];
        spot_market_cookie.place_order(
            &mut test,
            &mut mango_group_cookie,
            user_index,
            starting_spot_order_id + mint_index as u64,
            serum_dex::matching::Side::Bid,
            1,
            base_price,
        ).await;
    }

    // Step 4: Place `num_orders` perp orders
    mango_group_cookie.run_keeper(&mut test).await;

    let starting_perp_order_id = 2000;
    for mint_index in 0..num_orders {
        let mut perp_market_cookie = mango_group_cookie.perp_markets[mint_index];
        perp_market_cookie.place_order(
            &mut test,
            &mut mango_group_cookie,
            user_index,
            starting_perp_order_id + mint_index as u64,
            Side::Bid,
            1,
            base_price,
        ).await;
    }

    // Assert
    mango_group_cookie.run_keeper(&mut test).await;

    let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account.unwrap();
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
    // Disable all logs except error
    // solana_logger::setup_with("error");

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    let borrower_user_index: usize = 0;
    let lender_user_index: usize = 1;
    let mint_index: usize = 0;
    let base_mint = test.with_mint(mint_index);
    let base_price = 10000;

    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Act

    // Step 1: Make deposits from 2 accounts (Borrower / Lender)
    mango_group_cookie.run_keeper(&mut test).await;
    // Deposit 100_000 USDC as the borrower
    let quote_deposit_amount = (100_000 * test.quote_mint.unit) as u64;
    test.perform_deposit(
        &mango_group_cookie,
        borrower_user_index,
        test.quote_index,
        quote_deposit_amount,
    ).await;

    // Deposit 10 BTC as the lender
    let base_deposit_amount = (10 * base_mint.unit) as u64;
    test.perform_deposit(
        &mango_group_cookie,
        lender_user_index,
        mint_index,
        base_deposit_amount,
    ).await;

    // Step 2: Withdraw (with borrow) 1 BTC @ 10000 as the borrower
    mango_group_cookie.run_keeper(&mut test).await;

    let withdraw_amount = (1 * base_mint.unit) as u64;
    test.perform_withdraw(
        &mango_group_cookie,
        borrower_user_index,
        mint_index,
        withdraw_amount,
        true, // Allow borrow
    ).await;

    // Step 3: Check that lenders deposit is not a nice number anymore (> 2 BTC)
    mango_group_cookie.run_keeper(&mut test).await;

    let lender_base_deposit =
        &mango_group_cookie.mango_accounts[lender_user_index].mango_account.unwrap()
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index).unwrap();
    assert_ne!(lender_base_deposit.to_string(), I80F48::from_num(base_deposit_amount).to_string());

    // Step 4: Place a spot order for BTC
    mango_group_cookie.run_keeper(&mut test).await;

    let starting_spot_order_id = 1000;
    let mut spot_market_cookie = mango_group_cookie.spot_markets[mint_index];
    spot_market_cookie.place_order(
        &mut test,
        &mut mango_group_cookie,
        lender_user_index,
        starting_spot_order_id + mint_index as u64,
        serum_dex::matching::Side::Ask,
        1,
        base_price,
    ).await;
    // Assert
    mango_group_cookie.run_keeper(&mut test).await;

    let lender_mango_account =
        mango_group_cookie.mango_accounts[lender_user_index].mango_account.unwrap();
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
    // Disable all logs except error
    // solana_logger::setup_with("error");

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    let borrower_user_index: usize = 0;
    let lender_user_index: usize = 1;
    let mint_index: usize = 0;
    let num_orders: usize = test.num_mints - 1;
    let base_price = 10000;

    // Act
    // Step 1: Set oracles for all the markets
    mango_group_cookie.run_keeper(&mut test).await;

    for mint_index in 0..num_orders.min(MAX_NUM_IN_MARGIN_BASKET as usize) {
        mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;
    }

    // Step 2: Deposit 110_000 USDC as the borrower for collateral
    mango_group_cookie.run_keeper(&mut test).await;

    let quote_deposit_amount = (110_000 * test.quote_mint.unit) as u64;
    test.perform_deposit(
        &mango_group_cookie,
        borrower_user_index,
        test.quote_index,
        quote_deposit_amount,
    ).await;

    // Step 3: Deposit 10 of each mint as the lender
    mango_group_cookie.run_keeper(&mut test).await;

    for mint_index in 0..num_orders.min(MAX_NUM_IN_MARGIN_BASKET as usize) {
        let base_mint = test.with_mint(mint_index);
        let base_deposit_amount = (10 * base_mint.unit) as u64;
        test.perform_deposit(
            &mango_group_cookie,
            lender_user_index,
            mint_index,
            base_deposit_amount,
        ).await;
    }

    // Step 4: Withdraw (with borrow) 1 of each mint @ 10000 as the borrower
    mango_group_cookie.run_keeper(&mut test).await;

    for mint_index in 0..num_orders {
        let base_mint = test.with_mint(mint_index);
        let withdraw_amount = (1 * base_mint.unit) as u64;
        test.perform_withdraw(
            &mango_group_cookie,
            borrower_user_index,
            mint_index,
            withdraw_amount,
            true, // Allow borrow
        ).await;
    }

    // Step 6: Check that lenders all 10 deposits are not a nice number anymore (> 2 mint)
    mango_group_cookie.run_keeper(&mut test).await;

    for mint_index in 0..num_orders.min(MAX_NUM_IN_MARGIN_BASKET as usize) {
        let base_mint = test.with_mint(mint_index);
        let base_deposit_amount = (10 * base_mint.unit) as u64;
        let lender_base_deposit =
            &mango_group_cookie.mango_accounts[lender_user_index].mango_account.unwrap()
            .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index).unwrap();
        assert_ne!(lender_base_deposit.to_string(), I80F48::from_num(base_deposit_amount).to_string());
    }

    // Step 7: Place a spot order ASK for each mint
    mango_group_cookie.run_keeper(&mut test).await;

    let starting_spot_order_id = 1000;
    for mint_index in 0..num_orders {
        let mut spot_market_cookie = mango_group_cookie.spot_markets[mint_index];
        spot_market_cookie.place_order(
            &mut test,
            &mut mango_group_cookie,
            lender_user_index,
            starting_spot_order_id + mint_index as u64,
            serum_dex::matching::Side::Ask,
            1,
            base_price,
        ).await;
    }
    // Assert
    mango_group_cookie.run_keeper(&mut test).await;

    let lender_mango_account = mango_group_cookie.mango_accounts[lender_user_index].mango_account.unwrap();
    for spot_open_orders_index in 0..num_orders.min(MAX_NUM_IN_MARGIN_BASKET as usize) {
        assert_ne!(lender_mango_account.spot_open_orders[spot_open_orders_index], Pubkey::default());
    }
}
