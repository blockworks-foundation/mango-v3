mod program_test;
use anchor_lang::prelude::SolanaSysvar;
use mango::{matching::*, state::*};
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;
use solana_sdk::signer::Signer;
use std::time::Duration;
use std::{mem::size_of, mem::size_of_val};
use fixed::types::I80F48;
use solana_sdk::signature::Keypair;
use solana_program::pubkey::Pubkey;

#[tokio::test]
async fn test_american_options() {
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    let num_precreated_mango_users = 2;
    mango_group_cookie
        .full_setup(&mut test, num_precreated_mango_users, config.num_mints - 1)
        .await;
    println!("Creating option market american CALL MNGO of 100@10.2");
    
    let expiry = { let clock = test.get_clock().await; clock.unix_timestamp as u64 + 30 * solana_program::clock::DEFAULT_TICKS_PER_SECOND }; // after 30 sec
    let (option_market_pk, option_market) = test.create_option_market(&mango_group_cookie,
        0,
        15,
         OptionType::American,
         100_000_000, 
         10_200_000, 
         expiry,
        None).await;

    assert_eq!(option_market.meta_data.data_type, DataType::OptionMarket as u8);
    assert_eq!(option_market.option_type, OptionType::American);
    assert_eq!(option_market.contract_size, 100_000_000);
    assert_eq!(option_market.strike_price, 10_200_000);
    assert_eq!(option_market.expiry, expiry);
    assert_eq!(option_market.tokens_in_underlying_pool,  0);
    assert_eq!(option_market.tokens_in_quote_pool, 0);
    mango_group_cookie.run_keeper(&mut test).await;
    mango_group_cookie.run_keeper(&mut test).await;

    println!("Write option 10 Options");
    // deposit some underlying tokens for user0
    let load_amount = 120_000_000 * 10;
    test.perform_deposit(&mango_group_cookie, 0, option_market.underlying_token_index, load_amount).await;
    // write option with user0
    let (u0_trade_data_pk, option_trade_data) = test.write_option(&mango_group_cookie, option_market_pk, &option_market, 0, 10_000_000).await;
    //check write option
    {
        let option_tokens = option_trade_data.number_of_option_tokens;
        let writer_tokens = option_trade_data.number_of_writers_tokens;
        assert_eq!(option_tokens, 10_000_000);
        assert_eq!(writer_tokens, 10_000_000);
        let mango_account_key = mango_group_cookie.mango_accounts[0].address;
        let funds :u64 = test.with_mango_account_deposit(&mango_account_key, option_market.underlying_token_index).await;
        assert_eq!(load_amount - 1_000_000_000, funds );
        // check balances in account
        let option_market_ck = test.load_account::<OptionMarket>(option_market_pk).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 100_000_000 * 10);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 0);
    }
    println!("exercise option 5 option tokens");
    
    let mango_account_u0_key = mango_group_cookie.mango_accounts[0].address;
    let mut funds_underlying_old :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.underlying_token_index).await;
    let user0 = Keypair::from_base58_string(&test.users[0].to_base58_string());;
    let load_amount = 10_000_000 * 10;
    test.perform_deposit(&mango_group_cookie, 0, option_market.quote_token_index, load_amount).await;
    let mut funds_quote_old :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.quote_token_index).await;

    test.exercise_option(&mango_group_cookie, option_market_pk, &option_market, 0, 5_000_000).await;
    //check exercise
    {
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        let option_tokens = trade_data.number_of_option_tokens;
        assert_eq!(option_tokens, 5_000_000);
        let mango_account_key = mango_group_cookie.mango_accounts[0].address;
        let funds_underlying :u64 = test.with_mango_account_deposit(&mango_account_key, option_market.underlying_token_index).await;
        let funds_quote :u64 = test.with_mango_account_deposit(&mango_account_key, option_market.quote_token_index).await;
        assert_eq!(funds_underlying, funds_underlying_old + 100_000_000 * 5);
        assert_eq!(funds_quote, funds_quote_old -  10_200_000 * 5);
        let option_market_ck = test.load_account::<OptionMarket>(option_market_pk).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 100_000_000 * 5);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 10_200_000 * 5);
        funds_underlying_old= funds_underlying;
        funds_quote_old = funds_quote;
    }
    mango_group_cookie.run_keeper(&mut test).await;
    println!("sleeping for 1 min");
    test.advance_clock_past_timestamp(option_market.expiry as i64 + 1).await;
    mango_group_cookie.run_keeper(&mut test).await;

    println!("exchange writers 2 tokens");
    {
        let clock = test.get_clock().await;
        assert!( (clock.unix_timestamp as u64) > option_market.expiry );
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        let writer_tokens = trade_data.number_of_writers_tokens;
        assert_eq!(writer_tokens, 10_000_000);
    }
    
    test.exchange_writers_tokens(&mango_group_cookie, option_market_pk, &option_market, 0, 2_000_000,).await;
    {
        // check exchange
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        let writers_tokens = trade_data.number_of_writers_tokens;
        assert_eq!(writers_tokens, 8_000_000);

        let funds_underlying :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.underlying_token_index).await;
        let funds_quote :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.quote_token_index).await;
        assert_eq!(funds_underlying, funds_underlying_old + 100_000_000);
        assert_eq!(funds_quote, funds_quote_old + 10_199_999); // floating point rounding problem

        let option_market_ck = test.load_account::<OptionMarket>(option_market_pk).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 100_000_000 * 4);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 10_200_000 * 4);
        funds_underlying_old = funds_underlying;
        funds_quote_old =  funds_quote;
    }
    println!("exchange writers 8 tokens");
    test.exchange_writers_tokens(&mango_group_cookie, option_market_pk, &option_market, 0, 8_000_000,).await;
    {
        // check exchange
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        let writers_tokens = trade_data.number_of_writers_tokens;
        assert_eq!(writers_tokens, 0);

        let funds_underlying :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.underlying_token_index).await;
        let funds_quote :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.quote_token_index).await;
        assert_eq!(funds_underlying, funds_underlying_old + 100_000_000 * 4);
        assert_eq!(funds_quote, funds_quote_old + 10_200_000 * 4);

        let option_market_ck = test.load_account::<OptionMarket>(option_market_pk).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 0);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 0);
        funds_underlying_old = funds_underlying;
        funds_quote_old =  funds_quote;
    }

}


#[tokio::test]
async fn test_european_options() {
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    let num_precreated_mango_users = 2;
    mango_group_cookie
        .full_setup(&mut test, num_precreated_mango_users, config.num_mints - 1)
        .await;
    println!("Creating option market");
    
    let expiry = { let clock = test.get_clock().await; clock.unix_timestamp as u64 + 30 * solana_program::clock::DEFAULT_TICKS_PER_SECOND }; // after 30 sec
    let expiry_to_exercise = { let clock = test.get_clock().await; clock.unix_timestamp as u64 + 60 * solana_program::clock::DEFAULT_TICKS_PER_SECOND }; // after 30 sec
    // create a european option     
    let (option_market_pk, option_market) = test.create_option_market(&mango_group_cookie,
        0,
        15,
         OptionType::European,
         100_000_000, 
         10_200_000, 
         expiry,
        Some(expiry_to_exercise)).await;

    assert_eq!(option_market.meta_data.data_type, DataType::OptionMarket as u8);
    assert_eq!(option_market.option_type, OptionType::European);
    assert_eq!(option_market.contract_size, 100_000_000);
    assert_eq!(option_market.strike_price, 10_200_000);
    assert_eq!(option_market.expiry, expiry);
    assert_eq!(option_market.expiry_to_exercise_european, expiry_to_exercise);
    assert_eq!(option_market.tokens_in_underlying_pool,  0);
    assert_eq!(option_market.tokens_in_quote_pool,  0);
    mango_group_cookie.run_keeper(&mut test).await;
    mango_group_cookie.run_keeper(&mut test).await;

    println!("Write option");
    // deposit some underlying tokens for user0
    let load_amount = 120_000_000 * 10;
    test.perform_deposit(&mango_group_cookie, 0, option_market.underlying_token_index, load_amount).await;
    // write option with user0
    let (u0_trade_data_pk, option_trade_data) = test.write_option(&mango_group_cookie, option_market_pk, &option_market, 0, 10_000_000).await;
    //check write option
    {
        let option_tokens = option_trade_data.number_of_option_tokens;
        let writer_tokens = option_trade_data.number_of_writers_tokens;
        assert_eq!(option_tokens, 10_000_000);
        assert_eq!(writer_tokens, 10_000_000);
        let mango_account_key = mango_group_cookie.mango_accounts[0].address;
        let funds :u64 = test.with_mango_account_deposit(&mango_account_key, option_market.underlying_token_index).await;
        assert_eq!(load_amount - 1_000_000_000, funds );
    
        let option_market_ck = test.load_account::<OptionMarket>(option_market_pk).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 100_000_000 * 10);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 0);
    }
    println!("exercise option");
    
    let mango_account_u0_key = mango_group_cookie.mango_accounts[0].address;
    let mut funds_underlying_old :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.underlying_token_index).await;
    let user0 = Keypair::from_base58_string(&test.users[0].to_base58_string());;
    let load_amount = 10_000_000 * 10;
    test.perform_deposit(&mango_group_cookie, 0, option_market.quote_token_index, load_amount).await;
    let mut funds_quote_old :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.quote_token_index).await;
    test.advance_clock_past_timestamp(option_market.expiry as i64 + 1).await;

    test.exercise_option(&mango_group_cookie, option_market_pk, &option_market, 0, 5_000_000).await;
    //check exercise
    {
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        let option_tokens = trade_data.number_of_option_tokens;
        assert_eq!(option_tokens, 5_000_000);
        let mango_account_key = mango_group_cookie.mango_accounts[0].address;
        let funds_underlying :u64 = test.with_mango_account_deposit(&mango_account_key, option_market.underlying_token_index).await;
        let funds_quote :u64 = test.with_mango_account_deposit(&mango_account_key, option_market.quote_token_index).await;
        assert_eq!(funds_underlying, funds_underlying_old + 100_000_000 * 5);
        assert_eq!(funds_quote, funds_quote_old -  10_200_000 * 5);
        let option_market_ck = test.load_account::<OptionMarket>(option_market_pk).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 100_000_000 * 5);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 10_200_000 * 5);
        funds_underlying_old= funds_underlying;
        funds_quote_old = funds_quote;
    }
    mango_group_cookie.run_keeper(&mut test).await;
    println!("sleeping for 1 min");
    test.advance_clock_past_timestamp(option_market.expiry_to_exercise_european as i64 + 1).await;
    mango_group_cookie.run_keeper(&mut test).await;

    println!("exchange writers 2 tokens");
    {
        let clock = test.get_clock().await;
        assert!( (clock.unix_timestamp as u64) > option_market.expiry );
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        let writer_tokens = trade_data.number_of_writers_tokens;
        assert_eq!(writer_tokens, 10_000_000);
    }
    
    test.exchange_writers_tokens(&mango_group_cookie, option_market_pk, &option_market, 0, 2_000_000,).await;
    {
        // check exchange
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        let writers_tokens = trade_data.number_of_writers_tokens;
        assert_eq!(writers_tokens, 8_000_000);

        let funds_underlying :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.underlying_token_index).await;
        let funds_quote :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.quote_token_index).await;
        assert_eq!(funds_underlying, funds_underlying_old + 100_000_000);
        assert_eq!(funds_quote, funds_quote_old + 10_199_999); // floating point rounding problem

        let option_market_ck = test.load_account::<OptionMarket>(option_market_pk).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 100_000_000 * 4);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 10_200_000 * 4);
        funds_underlying_old = funds_underlying;
        funds_quote_old =  funds_quote;
    }
    println!("exchange writers 8 tokens");
    test.exchange_writers_tokens(&mango_group_cookie, option_market_pk, &option_market, 0, 8_000_000,).await;
    {
        // check exchange
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        let writers_tokens = trade_data.number_of_writers_tokens;
        assert_eq!(writers_tokens, 0);

        let funds_underlying :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.underlying_token_index).await;
        let funds_quote :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.quote_token_index).await;
        assert_eq!(funds_underlying, funds_underlying_old + 100_000_000 * 4);
        assert_eq!(funds_quote, funds_quote_old + 10_200_000 * 4);

        let option_market_ck = test.load_account::<OptionMarket>(option_market_pk).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 0);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 0);
        funds_underlying_old = funds_underlying;
        funds_quote_old =  funds_quote;
    }
}


#[tokio::test]
async fn test_placing_orders_for_options() {

    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    let num_precreated_mango_users = 2;
    mango_group_cookie
        .full_setup(&mut test, num_precreated_mango_users, config.num_mints - 1)
        .await;
    println!("Creating option market");
    
    let expiry = { let clock = test.get_clock().await; clock.unix_timestamp as u64 + 30 * solana_program::clock::DEFAULT_TICKS_PER_SECOND }; // after 30 sec
    let expiry_to_exercise = { let clock = test.get_clock().await; clock.unix_timestamp as u64 + 60 * solana_program::clock::DEFAULT_TICKS_PER_SECOND }; // after 30 sec
    // MNGO/USDC put option     
    let (option_market_pk, option_market) = test.create_option_market(&mango_group_cookie,
        15, // USDC
        0,    // MNGO
         OptionType::American,
         10_000_000, //10 USDC FOR
         100_000_000,  //100 MNGO 
         expiry,
        Some(expiry_to_exercise)).await;

    mango_group_cookie.run_keeper(&mut test).await;

    println!("Write option");
    // deposit some underlying tokens for user0
    let load_amount = 100_000_000 * 2;
    test.perform_deposit(&mango_group_cookie, 1, option_market.quote_token_index, load_amount).await;
    test.perform_deposit(&mango_group_cookie, 0, QUOTE_INDEX, 100_000_000).await;
    test.perform_deposit(&mango_group_cookie, 1, QUOTE_INDEX, 100_000_000).await;
    // write option with user0
    let (u0_trade_data_pk, option_trade_data) = test.write_option(&mango_group_cookie, option_market_pk, &option_market, 0, 10_000_000).await;
    println!("User0 places ask of 1 option @0.5 USDC");
    // place trades with user0
    // place ask of 50 cents USDC for 1 option contract
    test.place_options_order(&mango_group_cookie, option_market_pk, &option_market, 0, 1_000_000, 500_000, Side::Ask, 0).await;
    {
        // check order in the asks
        let asks = test.load_account::<BookSide>(option_market.asks).await;
        let mut iter = BookSideIter::new(&asks);
        let leaf = iter.next().unwrap();
        assert_eq!(leaf.order_type, OrderType::Limit);
        assert_eq!(leaf.price(), 500_000);
        assert_eq!(leaf.quantity, 1_000_000);
        assert_eq!(leaf.client_order_id, 0);
        assert_eq!(leaf.owner, u0_trade_data_pk);

        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        assert_eq!(trade_data.number_of_option_tokens, 9_000_000);
        assert_eq!(trade_data.number_of_writers_tokens, 10_000_000);
        assert_eq!(trade_data.order_filled[0], true);
        assert_eq!(trade_data.order_side[0], Side::Ask);
        assert_eq!(trade_data.client_order_ids[0], 0);
        assert_eq!(trade_data.number_of_usdc_locked, 0 );
        assert_eq!(trade_data.number_of_usdc_to_settle, 0 );
        assert_eq!(trade_data.number_of_locked_options_tokens, 1_000_000 );
    }
    println!("User0 places ask of 1 option @0.1 USDC by mistake");
    test.place_options_order(&mango_group_cookie, option_market_pk, &option_market, 0, 1_000_000, 100_000, Side::Ask, 1).await;
    println!("User0 cancels last order");
    test.cancel_option_order_by_client_order_id(&mango_group_cookie, option_market_pk, &option_market, 0, 1).await;
    println!("User1 places bid of 0.5 option @0.6 USDC");
    // user 1 places order for bid
    let mango_account_u0 = mango_group_cookie.mango_accounts[0].address;
    let mango_account_u1 = mango_group_cookie.mango_accounts[1].address;
    let (u1_trade_data_pk, _) = Pubkey::find_program_address( &[b"mango_option_user_data", option_market_pk.as_ref(), mango_account_u1.as_ref()], &test.mango_program_id );
        
    let mut usdc_with_u1 :u64 = test.with_mango_account_deposit(&mango_account_u1, QUOTE_INDEX).await;
    test.place_options_order(&mango_group_cookie, option_market_pk, &option_market, 1, 500_000, 600_000, Side::Bid, 0).await;
    {
        // Should be executed at 0.5 USDC
        let trade_data = test.load_account::<UserOptionTradeData>(u1_trade_data_pk).await;
        assert_eq!(trade_data.number_of_option_tokens, 500_000);
        assert_eq!(trade_data.number_of_writers_tokens, 0);
        assert_eq!(trade_data.number_of_usdc_locked, 0 );
        assert_eq!(trade_data.number_of_usdc_to_settle, 0 );
        assert_eq!(trade_data.number_of_locked_options_tokens, 0 );

        let usdc_with_u1_after :u64 = test.with_mango_account_deposit(&mango_account_u1, QUOTE_INDEX).await;
        assert_eq!(usdc_with_u1 - usdc_with_u1_after, 250_000);
        usdc_with_u1 = usdc_with_u1_after;
    }
    println!("Consume Event will update User0 trade data and deposit USDC recieved from User1");
    let mut usdc_with_u0 :u64 = test.with_mango_account_deposit(&mango_account_u0, QUOTE_INDEX).await;
    // comsume events to update user0
    test.consume_events_for_options(&mango_group_cookie, option_market_pk, &option_market, Vec::from([0,1])).await;

    {
        // Should be executed at 0.5 USDC
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        assert_eq!(trade_data.number_of_option_tokens, 9_000_000);
        assert_eq!(trade_data.number_of_writers_tokens, 10_000_000);
        assert_eq!(trade_data.order_filled[0], true); // all order not yet matched
        assert_eq!(trade_data.number_of_usdc_locked, 0 );
        assert_eq!(trade_data.number_of_usdc_to_settle, 0 );
        assert_eq!(trade_data.number_of_locked_options_tokens, 500_000 );

        let usdc_with_u0_after :u64 = test.with_mango_account_deposit(&mango_account_u0, QUOTE_INDEX).await;
        assert_eq!(usdc_with_u0 + 250_000, usdc_with_u0_after); // user0 recieved 250_000 USDC 25 cents
        usdc_with_u0 = usdc_with_u0_after;
    }
    test.advance_clock_by_slots(20).await;

    println!("User1 places bid of 2 option @1 USDC");
    test.place_options_order(&mango_group_cookie, option_market_pk, &option_market, 1, 2_000_000, 1_000_000, Side::Bid, 0).await;
    {
        let trade_data = test.load_account::<UserOptionTradeData>(u1_trade_data_pk).await;
        assert_eq!(trade_data.number_of_option_tokens, 1_000_000);
        assert_eq!(trade_data.number_of_writers_tokens, 0);
        assert_eq!(trade_data.order_filled[0], true);
        assert_eq!(trade_data.order_filled[1], false);
        assert_eq!(trade_data.order_side[0], Side::Bid);
        assert_eq!(trade_data.client_order_ids[0], 0);
        assert_eq!(trade_data.number_of_usdc_locked, 1_500_000 );
        assert_eq!(trade_data.number_of_usdc_to_settle, 0 );
        assert_eq!(trade_data.number_of_locked_options_tokens, 0 );

        let usdc_with_u1_after :u64 = test.with_mango_account_deposit(&mango_account_u1, QUOTE_INDEX).await;
        assert_eq!(usdc_with_u1 - usdc_with_u1_after, 1_750_000); // 500_000 matched at 500_000
        usdc_with_u1 = usdc_with_u1_after;
    }
    test.advance_clock().await;

    println!("Consume Event will update User0 trade data and deposit USDC recieved from User1");
    // comsume events to update user0
    test.consume_events_for_options(&mango_group_cookie, option_market_pk, &option_market, Vec::from([0,1])).await;
    {
        // Should be executed at 0.5 USDC
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        assert_eq!(trade_data.number_of_option_tokens, 9_000_000);
        assert_eq!(trade_data.number_of_writers_tokens, 10_000_000);
        assert_eq!(trade_data.number_of_usdc_locked, 0 );
        assert_eq!(trade_data.number_of_usdc_to_settle, 0 );
        assert_eq!(trade_data.number_of_locked_options_tokens, 0 );
        assert_eq!(trade_data.order_filled[0], false); // all order are matched and no order is pending

        let usdc_with_u0_after :u64 = test.with_mango_account_deposit(&mango_account_u0, QUOTE_INDEX).await;
        assert_eq!(usdc_with_u0 + 250_000, usdc_with_u0_after); // user0 recieved 250_000 USDC 25 cents
        usdc_with_u0 = usdc_with_u0_after;
    }

    test.advance_clock_by_slots(20).await;

    println!("User1 places bid of 2 option @0.9 USDC");
    usdc_with_u1 = test.with_mango_account_deposit(&mango_account_u1, QUOTE_INDEX).await;
    test.place_options_order(&mango_group_cookie, option_market_pk, &option_market, 1, 2_000_000, 900_000, Side::Bid, 1).await;
    {
        let trade_data = test.load_account::<UserOptionTradeData>(u1_trade_data_pk).await;
        assert_eq!(trade_data.number_of_option_tokens, 1_000_000);
        assert_eq!(trade_data.number_of_writers_tokens, 0);
        assert_eq!(trade_data.order_filled[1], true);
        assert_eq!(trade_data.order_side[1], Side::Bid);
        assert_eq!(trade_data.client_order_ids[1], 1);
        assert_eq!(trade_data.number_of_usdc_locked, 3_300_000 );
        assert_eq!(trade_data.number_of_usdc_to_settle, 0 );
        assert_eq!(trade_data.number_of_locked_options_tokens, 0 );
    }
    test.advance_clock().await;
    println!("User1 cancels bid of 2 option @0.9 USDC");
    test.cancel_option_order_by_client_order_id(&mango_group_cookie, option_market_pk, &option_market, 1, 1).await;
    {
        // data remains unchanged
        let trade_data = test.load_account::<UserOptionTradeData>(u1_trade_data_pk).await;
        assert_eq!(trade_data.number_of_option_tokens, 1_000_000);
        assert_eq!(trade_data.number_of_writers_tokens, 0);
        assert_eq!(trade_data.order_filled[0], true);
        assert_eq!(trade_data.order_side[0], Side::Bid);
        assert_eq!(trade_data.client_order_ids[0], 0);
        assert_eq!(trade_data.number_of_usdc_locked, 1_500_000 );
        assert_eq!(trade_data.number_of_usdc_to_settle, 0 );
        assert_eq!(trade_data.number_of_locked_options_tokens, 0 );

        let usdc_with_u1_after :u64 = test.with_mango_account_deposit(&mango_account_u1, QUOTE_INDEX).await;
        assert_eq!(usdc_with_u1, usdc_with_u1_after);
    }

    println!("User0 places ask of 0.5 option @0.9 USDC");
    test.place_options_order(&mango_group_cookie, option_market_pk, &option_market,0, 900_000, 900_000, Side::Ask, 0).await;
    {
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        assert_eq!(trade_data.number_of_option_tokens, 8_100_000);
        assert_eq!(trade_data.number_of_writers_tokens, 10_000_000);
        assert_eq!(trade_data.order_filled[0], false);
        assert_eq!(trade_data.number_of_usdc_locked, 0 );
        assert_eq!(trade_data.number_of_usdc_to_settle, 0 );
        assert_eq!(trade_data.number_of_locked_options_tokens, 0 );

        let usdc_with_u0_after :u64 = test.with_mango_account_deposit(&mango_account_u0, QUOTE_INDEX).await;
        assert_eq!(usdc_with_u0_after - usdc_with_u0, 899_999); // 900_000 matched at 1_000_000, float rounding error
        usdc_with_u0 = usdc_with_u0_after;
    }
    test.advance_clock().await;
    println!("Consume Event will update User1 trade data and deposit Opion tokens recieved from User0");
    // comsume events to update user0
    test.consume_events_for_options(&mango_group_cookie, option_market_pk, &option_market, Vec::from([1,0])).await;
    {
        // Should be executed at 0.5 USDC
        let trade_data = test.load_account::<UserOptionTradeData>(u1_trade_data_pk).await;
        assert_eq!(trade_data.number_of_option_tokens, 1_900_000);
        assert_eq!(trade_data.number_of_writers_tokens, 0);
        assert_eq!(trade_data.number_of_usdc_locked, 600_000 );
        assert_eq!(trade_data.number_of_usdc_to_settle, 0 );
        assert_eq!(trade_data.number_of_locked_options_tokens, 0 );
        assert_eq!(trade_data.order_filled[0], true);
        // check the account balance
        let usdc_with_u1_after :u64 = test.with_mango_account_deposit(&mango_account_u1, QUOTE_INDEX).await;
        assert_eq!(usdc_with_u1, usdc_with_u1_after); // no usdc transfered
        usdc_with_u1 = usdc_with_u1_after;
    }
    // user1 excesices the option
    let funds_underlying_u1 :u64 = test.with_mango_account_deposit(&mango_account_u1, option_market.underlying_token_index).await;
    let funds_quote_u1 :u64 = test.with_mango_account_deposit(&mango_account_u1, option_market.quote_token_index).await;
    {
        test.exercise_option(&mango_group_cookie, option_market_pk, &option_market, 1, 1_000_000).await;
        let funds_underlying_u1_a :u64 = test.with_mango_account_deposit(&mango_account_u1, option_market.underlying_token_index).await;
        let funds_quote_u1_a :u64 = test.with_mango_account_deposit(&mango_account_u1, option_market.quote_token_index).await;
        assert_eq!( funds_underlying_u1_a - funds_underlying_u1, 10_000_000 );
        assert_eq!( funds_quote_u1 - funds_quote_u1_a, 100_000_000 );

        let trade_data = test.load_account::<UserOptionTradeData>(u1_trade_data_pk).await;
        assert_eq!(trade_data.number_of_option_tokens, 900_000);
        assert_eq!(trade_data.number_of_writers_tokens, 0);
        assert_eq!(trade_data.number_of_usdc_locked, 600_000 );
        assert_eq!(trade_data.number_of_usdc_to_settle, 0 );
        assert_eq!(trade_data.number_of_locked_options_tokens, 0 );
        assert_eq!(trade_data.order_filled[0], true);
    }
}