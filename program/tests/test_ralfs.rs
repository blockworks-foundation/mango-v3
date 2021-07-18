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
    let oracle_price = test.with_oracle_price(&quote_mint, &base_mint, base_price as u64);
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
        &oracle_pks[0],
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
    test.add_market_to_mango_group(&mango_group_pk).await;
}
