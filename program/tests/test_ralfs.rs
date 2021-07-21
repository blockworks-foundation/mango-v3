// Tests related to placing orders on a perp market
mod helpers;
mod program_test;
use arrayref::array_ref;
use bytemuck::cast_ref;
use fixed::types::I80F48;
use helpers::*;
use mango_common::Loadable;
use program_test::*;
use std::num::NonZeroU64;
use std::{mem::size_of, mem::size_of_val, thread::sleep, time::Duration};

use mango::{
    entrypoint::process_instruction, instruction::*, matching::*, oracle::StubOracle, queue::*,
    state::*,
};
use solana_program::{account_info::AccountInfo, pubkey::Pubkey};
use solana_program_test::*;
use solana_sdk::account::ReadableAccount;
use solana_sdk::{
    account::Account, commitment_config::CommitmentLevel, signature::Keypair, signer::Signer,
    transaction::Transaction,
};

use serum_dex::instruction::{NewOrderInstructionV3, SelfTradeBehavior};
use serum_dex::state::{MarketState, OpenOrders, State, ToAlignedBytes};

#[tokio::test]
async fn test_init_perp_market_ralfs() {
    // Arrange
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mint_index = 0;
    let market_index = 0;
    let (mango_group_pk, mango_group) = test.with_mango_group().await;
    // Act
    let (perp_market_pk, perp_market) =
        test.with_perp_market(&mango_group_pk, mint_index, market_index).await;
    // Assert
    assert_eq!(size_of_val(&perp_market), size_of::<PerpMarket>());
}

async fn place_and_cancel_order_scenario(my_order_id: u64, order_side: Side) {
    // Arrange
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let user_index = 0;
    let mint_index = 0;
    let quote_index = config.num_mints - 1;
    let market_index = 0;

    let quote_mint = test.with_mint(quote_index as usize);
    let base_mint = test.with_mint(mint_index as usize);

    let base_price = 10000;
    let raw_order_size = 10000;

    let (mango_group_pk, mango_group) = test.with_mango_group().await;
    let (mango_account_pk, mut mango_account) =
        test.with_mango_account(&mango_group_pk, user_index).await;
    let (mango_cache_pk, mango_cache) = test.with_mango_cache(&mango_group).await;

    let oracle_pks = test.with_oracles(&mango_group_pk, quote_index).await;
    let deposit_amount = (base_price * quote_mint.unit) as u64;
    let (perp_market_pk, perp_market) =
        test.with_perp_market(&mango_group_pk, mint_index, market_index).await;

    test.perform_deposit(
        &mango_group,
        &mango_group_pk,
        &mango_account_pk,
        user_index,
        quote_index as usize,
        deposit_amount,
    )
    .await;

    let order_price = test.with_order_price(&quote_mint, &base_mint, base_price);
    let order_size = test.with_order_size(&base_mint, raw_order_size);
    let order_type = OrderType::Limit;

    // Act
    test.place_perp_order(
        &mango_group,
        &mango_group_pk,
        &mango_account,
        &mango_account_pk,
        &perp_market,
        &perp_market_pk,
        order_side,
        order_price,
        order_size,
        my_order_id,
        order_type,
        &oracle_pks,
        user_index,
    )
    .await;

    // Assert
    mango_account = test.load_account::<MangoAccount>(mango_account_pk).await;
    let (client_order_id, order_id, side) =
        mango_account.perp_accounts[0].open_orders.orders_with_client_ids().last().unwrap();
    assert_eq!(client_order_id, NonZeroU64::new(my_order_id).unwrap());
    assert_eq!(side, order_side);
}

#[tokio::test]
async fn test_place_and_cancel_order_ralfs() {
    // Scenario 1
    place_and_cancel_order_scenario(1212, Side::Bid).await;
    // Scenario 2
    place_and_cancel_order_scenario(1212, Side::Ask).await;
}

#[tokio::test]
async fn test_list_market_on_serum() {
    // Arrange
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mint_index = 0;
    let quote_index = config.num_mints - 1;
    let base_mint = test.with_mint(mint_index);
    // Act
    let market_pubkeys = test.list_market(mint_index as usize, quote_index as usize).await.unwrap();
    // Assert
    println!("Serum Market PK: {}", market_pubkeys.market.to_string());
    // Todo: Figure out how to assert this
}

#[tokio::test]
async fn test_add_all_markets_to_mango_group() {
    // Arrange
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let quote_index = config.num_mints - 1;

    let (mango_group_pk, mango_group) = test.with_mango_group().await;
    let oracle_pks = test.with_oracles(&mango_group_pk, quote_index).await;
    test.add_markets_to_mango_group(&mango_group_pk).await;
}

#[tokio::test]
async fn test_place_spot_order() {
    // Arrange
    let user_index: usize = 0;
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let num_markets = config.num_mints - 1;
    let quote_index = num_markets as usize;
    let quote_mint = test.with_mint(quote_index);
    let (mango_group_pk, mut mango_group) = test.with_mango_group().await;
    let (mango_account_pk, mut mango_account) =
        test.with_mango_account(&mango_group_pk, user_index).await;
    let (mango_cache_pk, mango_cache) = test.with_mango_cache(&mango_group).await;
    let oracle_pks = test.with_oracles(&mango_group_pk, num_markets).await;

    let base_price = 10000;
    let mint_index: usize = 0;
    let base_mint = test.with_mint(mint_index);
    let deposit_amount = (base_price * quote_mint.unit) as u64;
    let oracle_price = test.with_oracle_price(&quote_mint, &base_mint, base_price as u64);
    test.set_oracle(&mango_group_pk, &oracle_pks[mint_index], oracle_price).await;
    let spot_markets = test.add_markets_to_mango_group(&mango_group_pk).await;
    mango_group = test.load_account::<MangoGroup>(mango_group_pk).await;

    // Act
    test.perform_deposit(
        &mango_group,
        &mango_group_pk,
        &mango_account_pk,
        user_index,
        quote_index,
        deposit_amount,
    )
    .await;
    let order = NewOrderInstructionV3 {
        side: serum_dex::matching::Side::Bid,
        limit_price: NonZeroU64::new(base_price as u64).unwrap(),
        max_coin_qty: NonZeroU64::new(1).unwrap(),
        max_native_pc_qty_including_fees: NonZeroU64::new(base_price as u64).unwrap(),
        self_trade_behavior: SelfTradeBehavior::DecrementTake,
        order_type: serum_dex::matching::OrderType::Limit,
        client_order_id: 1000,
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

    // Assert
    mango_account = test.load_account::<MangoAccount>(mango_account_pk).await;
    // TODO
}

#[tokio::test]
async fn test_worst_case_scenario() {
    // Arrange
    let user_index: usize = 0;
    let num_markets = 31;
    let config =
        MangoProgramTestConfig { num_mints: num_markets + 1, ..MangoProgramTestConfig::default() };
    let mut test = MangoProgramTest::start_new(&config).await;
    solana_logger::setup_with_default(
        "solana_rbpf::vm=info,\
             solana_runtime::message_processor=debug,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info",
    );

    let quote_index = num_markets as usize;
    let quote_mint = test.with_mint(quote_index);
    let (mango_group_pk, mut mango_group) = test.with_mango_group().await;
    let (mango_account_pk, mut mango_account) =
        test.with_mango_account(&mango_group_pk, user_index).await;
    let (mango_cache_pk, mango_cache) = test.with_mango_cache(&mango_group).await;
    let oracle_pks = test.with_oracles(&mango_group_pk, num_markets).await;

    let spot_markets = test.add_markets_to_mango_group(&mango_group_pk).await;
    let (perp_market_pks, perp_markets) =
        test.add_perp_markets_to_mango_group(&mango_group_pk).await;

    test.cache_all_perp_markets(&mango_group, &mango_group_pk, &perp_market_pks).await;

    mango_group = test.load_account::<MangoGroup>(mango_group_pk).await;

    let base_price = 10000;
    let deposit_amount = (base_price * quote_mint.unit) as u64;
    // Perform deposit for the quote
    test.perform_deposit(
        &mango_group,
        &mango_group_pk,
        &mango_account_pk,
        user_index,
        quote_index,
        deposit_amount * num_markets,
    )
    .await;

    // Perform deposit for the rest of tokens
    for mint_index in 0..num_markets {
        let base_mint = test.with_mint(mint_index as usize);
        let mint_deposit_amount = (1 * base_mint.unit) as u64;
        test.perform_deposit(
            &mango_group,
            &mango_group_pk,
            &mango_account_pk,
            user_index,
            mint_index as usize,
            mint_deposit_amount,
        )
        .await;
    }

    // Place 31 spot orders
    let starting_order_id = 1000;
    for mint_index in 0..10 {
        println!("== PLACING SPOT ORDER {} ==", mint_index);
        let mint_index_u = mint_index as usize;
        let base_mint = test.with_mint(mint_index_u);
        let oracle_price = test.with_oracle_price(&quote_mint, &base_mint, base_price as u64);
        test.set_oracle(&mango_group_pk, &oracle_pks[mint_index_u], oracle_price).await;
        let order = NewOrderInstructionV3 {
            side: serum_dex::matching::Side::Bid,
            limit_price: NonZeroU64::new(base_price as u64).unwrap(),
            max_coin_qty: NonZeroU64::new(1).unwrap(),
            max_native_pc_qty_including_fees: NonZeroU64::new(base_price as u64).unwrap(),
            self_trade_behavior: SelfTradeBehavior::DecrementTake,
            order_type: serum_dex::matching::OrderType::Limit,
            client_order_id: starting_order_id + mint_index,
            limit: std::u16::MAX,
        };
        test.place_spot_order(
            &mango_group_pk,
            &mango_group,
            &mango_account_pk,
            &mango_account,
            &mango_cache_pk,
            spot_markets[mint_index_u],
            &oracle_pks,
            user_index,
            mint_index_u,
            order,
        )
        .await;
        println!("== PLACED SPOT ORDER {} ==", mint_index);
        mango_account = test.load_account::<MangoAccount>(mango_account_pk).await;
        // test.advance_clock().await;
    }

    mango_account = test.load_account::<MangoAccount>(mango_account_pk).await;

    let mut active_assets = mango_account.get_active_assets(&mango_group);
    for x in active_assets {
        println!("AA: {}", x);
    }

    // Long 31 perp markets
    let starting_perp_order_id = 2000;
    for mint_index in 0..num_markets {
        println!("== PLACING PERP ORDER {} ==", mint_index);
        let mint_index_u = mint_index as usize;
        let base_mint = test.with_mint(mint_index_u);

        let order_side = Side::Bid;
        let order_price = test.with_order_price(&quote_mint, &base_mint, base_price);
        let order_size = test.with_order_size(&base_mint, 1);
        let order_type = OrderType::Limit;

        // Act
        test.place_perp_order(
            &mango_group,
            &mango_group_pk,
            &mango_account,
            &mango_account_pk,
            &perp_markets[mint_index_u],
            &perp_market_pks[mint_index_u],
            order_side,
            order_price,
            order_size,
            starting_perp_order_id + mint_index,
            order_type,
            &oracle_pks,
            user_index,
        )
        .await;

        println!("== PLACED PERP ORDER {} ==", mint_index);
        mango_account = test.load_account::<MangoAccount>(mango_account_pk).await;
    }
    // Act

    // Assert
    mango_account = test.load_account::<MangoAccount>(mango_account_pk).await;

    let mut active_assets = mango_account.get_active_assets(&mango_group);
    for x in active_assets {
        println!("AA: {}", x);
    }

    for x in 0..mango_account.spot_open_orders.len() {
        println!("SOO: {}", mango_account.spot_open_orders[x].to_string());
        // let oo = test.get_account(mango_account.spot_open_orders[x]);
        // let market = MarketState::load(&spot_markets[x].market, &test.serum_program_id).unwrap();
        // println!("MARKET BASE DEPOSITS: {}", market.coin_deposits_total);
        // println!("MARKET QUOTE DEPOSITS: {}", market.pc_deposits_total);
    }
    // TODO
}
