// Tests related to placing orders on a perp market
mod helpers;
mod program_test;
use arrayref::array_ref;
use bytemuck::cast_ref;
use fixed::types::I80F48;
use helpers::*;
use program_test::*;
use mango_common::Loadable;
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
    let (mango_group_pk, mango_group) = test.with_mango_group().await;
    let quote_unit_config = test.with_unit_config(&mango_group, config.num_mints - 1, 10);
    let base_unit_config = test.with_unit_config(&mango_group, 0, 100);
    // Act
    let (perp_market_pk, perp_market) = test.with_perp_market(&mango_group_pk, &quote_unit_config, &base_unit_config, 0).await;
    // Assert
    assert_eq!(size_of_val(&perp_market), size_of::<PerpMarket>());
}

#[tokio::test]
async fn test_place_and_cancel_order_ralfs() {
    // Arrange
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;
    let (mango_group_pk, mango_group) = test.with_mango_group().await;
    let (mango_account_pk, mango_account) = test.with_mango_account(&mango_group_pk, 0).await;
    let oracle_pks = test.with_oracles(&mango_group_pk, config.num_mints - 1).await;
    let quote_unit_config = test.with_unit_config(&mango_group, config.num_mints - 1, 10);
    let base_unit_config = test.with_unit_config(&mango_group, 0, 100);
    let oracle_price = test.with_oracle_price(&quote_unit_config, &base_unit_config, 10000);
    let (perp_market_pk, perp_market) = test.with_perp_market(&mango_group_pk, &quote_unit_config, &base_unit_config, 0).await;
    test.perform_deposit(&mango_group, &mango_group_pk, &mango_account_pk, 0, (config.num_mints - 1) as usize, 10000 * quote_unit_config.unit);
}
