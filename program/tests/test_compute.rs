use std::num::NonZeroU64;
use std::{mem::size_of, mem::size_of_val, thread::sleep, time::Duration};

use arrayref::array_ref;
use bytemuck::cast_ref;
use fixed::types::I80F48;
use mango_common::Loadable;
use solana_program::{account_info::AccountInfo, pubkey::Pubkey};
use solana_program_test::*;
use solana_sdk::account::ReadableAccount;
use solana_sdk::{
    account::Account, commitment_config::CommitmentLevel, signature::Keypair, signer::Signer,
    transaction::Transaction,
};

use helpers::*;
use mango::{
    entrypoint::process_instruction, instruction::*, matching::*, oracle::StubOracle, queue::*,
    state::*,
};
use program_test::*;

mod helpers;
mod program_test;

#[tokio::test]
async fn test_add_all_markets_to_mango_group() {
    // Arrange
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 1, num_mints: 32 };
    let mut test = MangoProgramTest::start_new(&config).await;
    solana_logger::setup_with_default(
        "solana_rbpf::vm=info,\
             solana_runtime::message_processor=info,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info",
    );

    let quote_index = config.num_mints - 1;

    let (mango_group_pk, mango_group) = test.with_mango_group().await;
    let oracle_pks = test.with_oracles(&mango_group_pk, quote_index).await;
    test.add_markets_to_mango_group(&mango_group_pk).await;

    let mango_group = test.load_account::<MangoGroup>(mango_group_pk).await;

    let user_index = 0;
    let (mango_account_pk, mut mango_account) =
        test.with_mango_account(&mango_group_pk, user_index).await;
    println!("Performing deposit");

    for i in 0..config.num_mints {
        test.perform_deposit(
            &mango_group,
            &mango_group_pk,
            &mango_account_pk,
            user_index,
            i as usize,
            1000000,
        )
        .await;
    }
}
