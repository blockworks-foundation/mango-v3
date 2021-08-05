// // Tests related to liquidations
// mod program_test;
// use mango::{matching::*, state::*};
// use program_test::*;
// use solana_program::pubkey::Pubkey;
// use solana_program_test::*;
// use fixed::types::I80F48;
// use std::num::NonZeroU64;
// use std::{mem::size_of, mem::size_of_val};
//
// use serum_dex::instruction::{NewOrderInstructionV3, SelfTradeBehavior};
//
// #[tokio::test]
// async fn test_spot_liquidation() {
//     // Arrange
//     let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 3, num_mints: 2 };
//     let mut test = MangoProgramTest::start_new(&config).await;
//     // Supress some of the logs
//     solana_logger::setup_with_default(
//         "solana_rbpf::vm=info,\
//              solana_runtime::message_processor=debug,\
//              solana_runtime::system_instruction_processor=info,\
//              solana_program_test=info",
//     );
//
//     let (mango_group_pk, mut mango_group) = test.with_mango_group().await;
//     let oracle_pks = test.add_oracles_to_mango_group(&mango_group_pk).await;
//     let spot_markets = test.add_spot_markets_to_mango_group(&mango_group_pk).await;
//     // Need to reload mango group because `add_spot_markets` adds tokens in to mango_group
//     mango_group = test.load_account::<MangoGroup>(mango_group_pk).await;
//     let (mango_cache_pk, mut mango_cache) = test.with_mango_cache(&mango_group).await;
//
//     let bidder_user_index: usize = 0;
//     let asker_user_index: usize = 1;
//     let liqor_user_index: usize = 2;
//     let mint_index: usize = 0;
//     let base_mint = test.with_mint(mint_index);
//     let base_price = 15_000;
//     let order_amount = 1;
//     let mut oracle_price = test.with_oracle_price(&base_mint, base_price as u64);
//     test.set_oracle(&mango_group, &mango_group_pk, &oracle_pks[mint_index], oracle_price).await;
//
//     let (bidder_mango_account_pk, mut bidder_mango_account) =
//         test.with_mango_account(&mango_group_pk, bidder_user_index).await;
//     let (asker_mango_account_pk, mut asker_mango_account) =
//         test.with_mango_account(&mango_group_pk, asker_user_index).await;
//     let (liqor_mango_account_pk, mut liqor_mango_account) =
//         test.with_mango_account(&mango_group_pk, liqor_user_index).await;
//
//     // Act
//     // Step 1: Make deposits from 3 accounts (Bidder / Asker / Liqor)
//     // Deposit 10_000 USDC as the bidder
//     let quote_deposit_amount = (10_000 * test.quote_mint.unit) as u64;
//     test.perform_deposit(
//         &mango_group,
//         &mango_group_pk,
//         &bidder_mango_account_pk,
//         bidder_user_index,
//         test.quote_index,
//         quote_deposit_amount,
//     )
//     .await;
//
//     // Deposit 1 BTC as the asker
//     let base_deposit_amount = (order_amount * base_mint.unit) as u64;
//     test.perform_deposit(
//         &mango_group,
//         &mango_group_pk,
//         &asker_mango_account_pk,
//         asker_user_index,
//         mint_index,
//         base_deposit_amount,
//     )
//     .await;
//
//     // Deposit 10_000 USDC as the asker too (so we have someone to borrow from)
//     let quote_deposit_amount = (10_000 * test.quote_mint.unit) as u64;
//     test.perform_deposit(
//         &mango_group,
//         &mango_group_pk,
//         &asker_mango_account_pk,
//         asker_user_index,
//         test.quote_index,
//         quote_deposit_amount,
//     )
//     .await;
//
//     // Deposit 10_000 USDC as the liqor
//     test.perform_deposit(
//         &mango_group,
//         &mango_group_pk,
//         &liqor_mango_account_pk,
//         liqor_user_index,
//         test.quote_index,
//         quote_deposit_amount,
//     )
//     .await;
//
//
//     // Step 2: Place a bid for 1 BTC @ 10_000 USDC
//     test.run_keeper(&mango_group, &mango_group_pk, &oracle_pks, &[]).await;
//
//     let starting_order_id = 1000;
//
//     let limit_price = test.price_number_to_lots(&base_mint, base_price) as i64;
//     let max_coin_qty = test.base_size_number_to_lots(&base_mint, order_amount) as u64;
//     let max_native_pc_qty_including_fees = test.quote_size_number_to_lots(&base_mint, order_amount * limit_price) as u64;
//
//     let order = NewOrderInstructionV3 {
//         side: serum_dex::matching::Side::Bid,
//         limit_price: NonZeroU64::new(limit_price as u64).unwrap(),
//         max_coin_qty: NonZeroU64::new(max_coin_qty).unwrap(),
//         max_native_pc_qty_including_fees: NonZeroU64::new(max_native_pc_qty_including_fees).unwrap(),
//         self_trade_behavior: SelfTradeBehavior::DecrementTake,
//         order_type: serum_dex::matching::OrderType::Limit,
//         client_order_id: starting_order_id as u64,
//         limit: u16::MAX,
//     };
//     test.place_spot_order(
//         &mango_group_pk,
//         &mango_group,
//         &bidder_mango_account_pk,
//         &bidder_mango_account,
//         spot_markets[mint_index],
//         &oracle_pks,
//         bidder_user_index,
//         mint_index,
//         order,
//     )
//     .await;
//
//
//     // Step 3: Place an ask for 1 BTC @ 10_000 USDC
//     test.run_keeper(&mango_group, &mango_group_pk, &oracle_pks, &[]).await;
//
//     let order = NewOrderInstructionV3 {
//         side: serum_dex::matching::Side::Ask,
//         limit_price: NonZeroU64::new(limit_price as u64).unwrap(),
//         max_coin_qty: NonZeroU64::new(max_coin_qty).unwrap(),
//         max_native_pc_qty_including_fees: NonZeroU64::new(max_native_pc_qty_including_fees).unwrap(),
//         self_trade_behavior: SelfTradeBehavior::DecrementTake,
//         order_type: serum_dex::matching::OrderType::Limit,
//         client_order_id: starting_order_id + 1 as u64,
//         limit: u16::MAX,
//     };
//     test.place_spot_order(
//         &mango_group_pk,
//         &mango_group,
//         &asker_mango_account_pk,
//         &asker_mango_account,
//         spot_markets[mint_index],
//         &oracle_pks,
//         asker_user_index,
//         mint_index,
//         order,
//     )
//     .await;
//
//     // Step 4: Consume events
//     test.run_keeper(&mango_group, &mango_group_pk, &oracle_pks, &[]).await;
//     bidder_mango_account = test.load_account::<MangoAccount>(bidder_mango_account_pk).await;
//     asker_mango_account = test.load_account::<MangoAccount>(asker_mango_account_pk).await;
//
//     // NOTE: Dunno why, but sometimes this fails...
//     test.consume_events(
//         spot_markets[mint_index],
//         vec![
//             &bidder_mango_account.spot_open_orders[0],
//             &asker_mango_account.spot_open_orders[0],
//         ],
//         bidder_user_index,
//         mint_index,
//     ).await;
//
//     // Step 5: Settle funds so that deposits get updated
//     test.run_keeper(&mango_group, &mango_group_pk, &oracle_pks, &[]).await;
//     bidder_mango_account = test.load_account::<MangoAccount>(bidder_mango_account_pk).await;
//     asker_mango_account = test.load_account::<MangoAccount>(asker_mango_account_pk).await;
//     // Settling bidder
//     test.settle_funds(
//         &mango_group_pk,
//         &mango_group,
//         &bidder_mango_account_pk,
//         &bidder_mango_account,
//         spot_markets[mint_index],
//         &oracle_pks,
//         bidder_user_index,
//         mint_index,
//     ).await;
//     // Settling asker
//     test.settle_funds(
//         &mango_group_pk,
//         &mango_group,
//         &asker_mango_account_pk,
//         &asker_mango_account,
//         spot_markets[mint_index],
//         &oracle_pks,
//         asker_user_index,
//         mint_index,
//     ).await;
//
//     // Step 5: Assert that the order has been matched and the bidder has 1 BTC in deposits
//     test.run_keeper(&mango_group, &mango_group_pk, &oracle_pks, &[]).await;
//     mango_cache = test.load_account::<MangoCache>(mango_cache_pk).await;
//     bidder_mango_account = test.load_account::<MangoAccount>(bidder_mango_account_pk).await;
//     asker_mango_account = test.load_account::<MangoAccount>(asker_mango_account_pk).await;
//
//     let bidder_base_deposit = bidder_mango_account.get_native_deposit(&mango_cache.root_bank_cache[0], 0).unwrap();
//     let asker_base_deposit = asker_mango_account.get_native_deposit(&mango_cache.root_bank_cache[0], 0).unwrap();
//
//     let bidder_quote_deposit = bidder_mango_account.get_native_deposit(&mango_cache.root_bank_cache[QUOTE_INDEX], QUOTE_INDEX).unwrap();
//     let asker_quote_deposit = asker_mango_account.get_native_deposit(&mango_cache.root_bank_cache[QUOTE_INDEX], QUOTE_INDEX).unwrap();
//
//     assert_eq!(bidder_base_deposit, I80F48::from_num(1000000));
//     assert_eq!(asker_base_deposit, I80F48::from_num(0));
//
//     // Step 6: Change the oracle price so that bidder becomes liqee
//     oracle_price = test.with_oracle_price(&base_mint, (base_price / 15) as u64);
//     test.set_oracle(&mango_group, &mango_group_pk, &oracle_pks[mint_index], oracle_price).await;
//
//     // Step 7: Perform liquidation
//     test.run_keeper(&mango_group, &mango_group_pk, &oracle_pks, &[]).await;
//
//     for x in 0..5 {
//         test.run_keeper(&mango_group, &mango_group_pk, &oracle_pks, &[]).await;
//         test.perform_liquidate_token_and_token(
//             &mango_group,
//             &mango_group_pk,
//             &bidder_mango_account_pk,
//             &bidder_mango_account,
//             &liqor_mango_account_pk,
//             &liqor_mango_account,
//             liqor_user_index,
//             0, // Asset index
//             QUOTE_INDEX, // Liab index
//         ).await;
//     }
//
//     // Assert
//     mango_cache = test.load_account::<MangoCache>(mango_cache_pk).await;
//     bidder_mango_account = test.load_account::<MangoAccount>(bidder_mango_account_pk).await;
//     liqor_mango_account = test.load_account::<MangoAccount>(liqor_mango_account_pk).await;
//
//     let bidder_base_deposit = bidder_mango_account.get_native_deposit(&mango_cache.root_bank_cache[0], 0).unwrap();
//     let liqor_base_deposit = liqor_mango_account.get_native_deposit(&mango_cache.root_bank_cache[0], 0).unwrap();
//
//     let bidder_base_borrows = bidder_mango_account.get_native_borrow(&mango_cache.root_bank_cache[0], 0).unwrap();
//     let liqor_base_borrows = liqor_mango_account.get_native_borrow(&mango_cache.root_bank_cache[0], 0).unwrap();
//
//     // TODO: Actually assert here
//
// }
