// Tests related to placing orders on a perp market
mod helpers;
mod program_test;
use arrayref::array_ref;
use bytemuck::cast_ref;
use fixed::types::I80F48;
use helpers::*;
use program_test::*;
use mango_common::Loadable;
use std::{mem::size_of, thread::sleep, time::Duration};

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
    let (oracle_pks) = test.with_oracles(&mango_group_pk, config.num_mints - 1).await;
    let (mango_account_pk, mango_account) = test.with_mango_account(&mango_group_pk).await;
    let quote_unit_config = test.with_unit_config(&mango_group, 0, 10);
    let base_unit_config = test.with_unit_config(&mango_group, 1, 100);
    let oracle_price = test.with_oracle_price(&quote_unit_config, &base_unit_config, 420);
}
