// Tests related to liquidations
mod program_test;
use mango::{matching::*, state::*};
use program_test::*;
use solana_program::pubkey::Pubkey;
use solana_program_test::*;
use std::num::NonZeroU64;
use std::{mem::size_of, mem::size_of_val};

use serum_dex::instruction::{NewOrderInstructionV3, SelfTradeBehavior};

#[tokio::test]
async fn test_spot_liquidation() {
    // Arrange
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 3, num_mints: 2 };
    let mut test = MangoProgramTest::start_new(&config).await;
    // Supress some of the logs
    solana_logger::setup_with_default(
        "solana_rbpf::vm=info,\
             solana_runtime::message_processor=debug,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info",
    );

    let (mango_group_pk, mut mango_group) = test.with_mango_group().await;
    let oracle_pks = test.add_oracles_to_mango_group(&mango_group_pk).await;
    let spot_markets = test.add_spot_markets_to_mango_group(&mango_group_pk).await;
    // Need to reload mango group because `add_spot_markets` adds tokens in to mango_group
    mango_group = test.load_account::<MangoGroup>(mango_group_pk).await;

    let liqee_user_index: usize = 0;
    let asker_user_index: usize = 1;
    let liqor_user_index: usize = 2;
    let mint_index: usize = 0;
    let base_mint = test.with_mint(mint_index);
    let base_price = 10000;
    let oracle_price = test.with_oracle_price(&base_mint, base_price as u64);
    test.set_oracle(&mango_group, &mango_group_pk, &oracle_pks[mint_index], oracle_price).await;

    let (liqee_mango_account_pk, mut liqee_mango_account) =
        test.with_mango_account(&mango_group_pk, liqee_user_index).await;
    let (asker_mango_account_pk, mut asker_mango_account) =
        test.with_mango_account(&mango_group_pk, asker_user_index).await;
    let (liqor_mango_account_pk, mut liqor_mango_account) =
        test.with_mango_account(&mango_group_pk, liqor_user_index).await;

    // Act
    // Step 1: Make deposits from 2 accounts (Liqee / Asker)
    // Deposit 10_000 USDC as the liqee
    let quote_deposit_amount = (10_000 * test.quote_mint.unit) as u64;
    test.perform_deposit(
        &mango_group,
        &mango_group_pk,
        &liqee_mango_account_pk,
        liqee_user_index,
        test.quote_index,
        quote_deposit_amount,
    )
    .await;

    // Deposit 10 BTC as the asker
    let base_deposit_amount = (10 * base_mint.unit) as u64;
    test.perform_deposit(
        &mango_group,
        &mango_group_pk,
        &asker_mango_account_pk,
        asker_user_index,
        mint_index,
        base_deposit_amount,
    )
    .await;


}
