// Tests related to spot markets
mod program_test;
use mango::{matching::*, state::*};
use program_test::*;
use solana_program::pubkey::Pubkey;
use solana_program_test::*;
use std::num::NonZeroU64;
use fixed::types::I80F48;
use std::{mem::size_of, mem::size_of_val};

use serum_dex::instruction::{NewOrderInstructionV3, SelfTradeBehavior};


#[tokio::test]
async fn test_list_spot_market_on_serum() {
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
    // Supress some of the logs
    solana_logger::setup_with_default(
        "solana_rbpf::vm=info,\
             solana_runtime::message_processor=debug,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info",
    );
    // Disable all logs except error
    // solana_logger::setup_with("error");

    let (mango_group_pk, _mango_group) = test.with_mango_group().await;
    test.add_oracles_to_mango_group(&mango_group_pk).await;
    test.add_spot_markets_to_mango_group(&mango_group_pk).await;
}

#[tokio::test]
async fn test_place_spot_order() {
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
        limit_price: NonZeroU64::new(test.priceNumberToLots(&base_mint, base_price) as u64).unwrap(),
        max_coin_qty: NonZeroU64::new(test.baseSizeNumberToLots(&base_mint, 1) as u64).unwrap(),
        max_native_pc_qty_including_fees: NonZeroU64::new(test.quoteSizeNumberToLots(&base_mint, base_price) as u64).unwrap(),
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
async fn test_match_spot_order() {
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

    let (mango_group_pk, mut mango_group) = test.with_mango_group().await;
    let oracle_pks = test.add_oracles_to_mango_group(&mango_group_pk).await;
    let spot_markets = test.add_spot_markets_to_mango_group(&mango_group_pk).await;
    // Need to reload mango group because `add_spot_markets` adds tokens in to mango_group
    mango_group = test.load_account::<MangoGroup>(mango_group_pk).await;
    let (mango_cache_pk, mut mango_cache) = test.with_mango_cache(&mango_group).await;

    let bidder_user_index: usize = 0;
    let asker_user_index: usize = 1;
    let mint_index: usize = 0;
    let base_mint = test.with_mint(mint_index);
    let base_price = 10_000;
    let oracle_price = test.with_oracle_price(&base_mint, base_price as u64);
    test.set_oracle(&mango_group, &mango_group_pk, &oracle_pks[mint_index], oracle_price).await;

    let (bidder_mango_account_pk, mut bidder_mango_account) =
        test.with_mango_account(&mango_group_pk, bidder_user_index).await;
    let (asker_mango_account_pk, mut asker_mango_account) =
        test.with_mango_account(&mango_group_pk, asker_user_index).await;

    // Act
    // Step 1: Make deposits from 2 accounts (Bidder / Asker)
    // Deposit 10_000 USDC as the bidder
    let quote_deposit_amount = (10_000 * test.quote_mint.unit) as u64;
    test.perform_deposit(
        &mango_group,
        &mango_group_pk,
        &bidder_mango_account_pk,
        bidder_user_index,
        test.quote_index,
        quote_deposit_amount,
    )
    .await;

    // Deposit 1 BTC as the asker
    let base_deposit_amount = (1 * base_mint.unit) as u64;
    test.perform_deposit(
        &mango_group,
        &mango_group_pk,
        &asker_mango_account_pk,
        asker_user_index,
        mint_index,
        base_deposit_amount,
    )
    .await;

    // Step 2: Place a bid for 1 BTC @ 10_000 USDC
    test.run_keeper(&mango_group, &mango_group_pk, &oracle_pks, &[]).await;

    let starting_order_id = 1000;

    let limit_price = test.priceNumberToLots(&base_mint, base_price) as i64;
    let max_coin_qty = test.baseSizeNumberToLots(&base_mint, 1) as u64;
    let max_native_pc_qty_including_fees = test.quoteSizeNumberToLots(&base_mint, 1 * limit_price) as u64;

    let order = NewOrderInstructionV3 {
        side: serum_dex::matching::Side::Bid,
        limit_price: NonZeroU64::new(limit_price as u64).unwrap(),
        max_coin_qty: NonZeroU64::new(max_coin_qty).unwrap(),
        max_native_pc_qty_including_fees: NonZeroU64::new(max_native_pc_qty_including_fees).unwrap(),
        self_trade_behavior: SelfTradeBehavior::DecrementTake,
        order_type: serum_dex::matching::OrderType::Limit,
        client_order_id: starting_order_id as u64,
        limit: u16::MAX,
    };
    test.place_spot_order(
        &mango_group_pk,
        &mango_group,
        &bidder_mango_account_pk,
        &bidder_mango_account,
        spot_markets[mint_index],
        &oracle_pks,
        bidder_user_index,
        mint_index,
        order,
    )
    .await;


    // Step 3: Place an ask for 1 BTC @ 10_000 USDC
    test.run_keeper(&mango_group, &mango_group_pk, &oracle_pks, &[]).await;

    let order = NewOrderInstructionV3 {
        side: serum_dex::matching::Side::Ask,
        limit_price: NonZeroU64::new(limit_price as u64).unwrap(),
        max_coin_qty: NonZeroU64::new(max_coin_qty).unwrap(),
        max_native_pc_qty_including_fees: NonZeroU64::new(max_native_pc_qty_including_fees).unwrap(),
        self_trade_behavior: SelfTradeBehavior::DecrementTake,
        order_type: serum_dex::matching::OrderType::Limit,
        client_order_id: starting_order_id + 1 as u64,
        limit: u16::MAX,
    };
    test.place_spot_order(
        &mango_group_pk,
        &mango_group,
        &asker_mango_account_pk,
        &asker_mango_account,
        spot_markets[mint_index],
        &oracle_pks,
        asker_user_index,
        mint_index,
        order,
    )
    .await;

    // Step 4: Consume events
    test.run_keeper(&mango_group, &mango_group_pk, &oracle_pks, &[]).await;
    bidder_mango_account = test.load_account::<MangoAccount>(bidder_mango_account_pk).await;
    asker_mango_account = test.load_account::<MangoAccount>(asker_mango_account_pk).await;

    test.consume_events(
        spot_markets[mint_index],
        vec![
            &bidder_mango_account.spot_open_orders[0],
            &asker_mango_account.spot_open_orders[0],
        ],
        bidder_user_index,
        mint_index,
    ).await;

    // Step 5: Settle funds so that deposits get updated
    test.run_keeper(&mango_group, &mango_group_pk, &oracle_pks, &[]).await;
    bidder_mango_account = test.load_account::<MangoAccount>(bidder_mango_account_pk).await;
    asker_mango_account = test.load_account::<MangoAccount>(asker_mango_account_pk).await;
    // Settling bidder
    test.settle_funds(
        &mango_group_pk,
        &mango_group,
        &bidder_mango_account_pk,
        &bidder_mango_account,
        spot_markets[mint_index],
        &oracle_pks,
        bidder_user_index,
        mint_index,
    ).await;
    // Settling asker
    test.settle_funds(
        &mango_group_pk,
        &mango_group,
        &asker_mango_account_pk,
        &asker_mango_account,
        spot_markets[mint_index],
        &oracle_pks,
        asker_user_index,
        mint_index,
    ).await;

    // Assert
    test.run_keeper(&mango_group, &mango_group_pk, &oracle_pks, &[]).await;
    mango_cache = test.load_account::<MangoCache>(mango_cache_pk).await;
    bidder_mango_account = test.load_account::<MangoAccount>(bidder_mango_account_pk).await;
    asker_mango_account = test.load_account::<MangoAccount>(asker_mango_account_pk).await;

    let bidder_base_deposit = bidder_mango_account.get_native_deposit(&mango_cache.root_bank_cache[0], 0).unwrap();
    let asker_base_deposit = asker_mango_account.get_native_deposit(&mango_cache.root_bank_cache[0], 0).unwrap();

    let bidder_quote_deposit = bidder_mango_account.get_native_deposit(&mango_cache.root_bank_cache[QUOTE_INDEX], QUOTE_INDEX).unwrap();
    let asker_quote_deposit = asker_mango_account.get_native_deposit(&mango_cache.root_bank_cache[QUOTE_INDEX], QUOTE_INDEX).unwrap();

    assert_eq!(bidder_base_deposit, I80F48::from_num(1000000));
    assert_eq!(asker_base_deposit, I80F48::from_num(0));

    // TODO: Figure out if the weird quote deposits should be asserted

}
