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

#[tokio::test]
async fn test_american_options() {
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    let num_precreated_mango_users = 2;
    mango_group_cookie
        .full_setup(&mut test, num_precreated_mango_users, config.num_mints - 1)
        .await;
    println!("Creating option market");
    
    let expiry = { let clock = test.get_clock().await; clock.unix_timestamp as u64 + 30 * solana_program::clock::DEFAULT_TICKS_PER_SECOND }; // after 30 sec
    let (om_key, option_market) = test.create_option_market(&mango_group_cookie,
        0,
        15,
         OptionType::American,
         I80F48::from_num(100_000_000), 
         I80F48::from_num(10_200_000), 
         expiry,
        None).await;

    assert_eq!(option_market.meta_data.data_type, DataType::OptionMarket as u8);
    assert_eq!(option_market.option_type, OptionType::American);
    assert_eq!(option_market.contract_size, I80F48::from_num(100_000_000));
    assert_eq!(option_market.quote_amount, I80F48::from_num(10_200_000));
    assert_eq!(option_market.expiry, expiry);
    assert_eq!(option_market.tokens_in_underlying_pool,  I80F48::from_num(0));
    assert_eq!(option_market.tokens_in_quote_pool,  I80F48::from_num(0));
    mango_group_cookie.run_keeper(&mut test).await;
    mango_group_cookie.run_keeper(&mut test).await;

    println!("Write option");
    // deposit some underlying tokens for user0
    let load_amount = 120_000_000 * 10;
    test.perform_deposit(&mango_group_cookie, 0, option_market.underlying_token_index, load_amount).await;
    // write option with user0
    let (u0_trade_data_pk, option_trade_data) = test.write_option(&mango_group_cookie, om_key, option_market, 0, I80F48::from_num(10_000_000)).await;
    //check write option
    {
        let option_tokens = option_trade_data.number_of_option_tokens;
        let writer_tokens = option_trade_data.number_of_writers_tokens;
        assert_eq!(option_tokens, 10_000_000);
        assert_eq!(writer_tokens, 10_000_000);
        let mango_account_key = mango_group_cookie.mango_accounts[0].address;
        let funds :u64 = test.with_mango_account_deposit(&mango_account_key, option_market.underlying_token_index).await;
        assert_eq!(load_amount - 1_000_000_000, funds );
    
        let option_market_ck = test.load_account::<OptionMarket>(om_key).await;
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

    test.exercise_option(&mango_group_cookie, om_key, option_market, 0, I80F48::from_num(5_000_000)).await;
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
        let option_market_ck = test.load_account::<OptionMarket>(om_key).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 100_000_000 * 5);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 10_200_000 * 5);
        funds_underlying_old= funds_underlying;
        funds_quote_old = funds_quote;
    }
    mango_group_cookie.run_keeper(&mut test).await;
    println!("sleeping for 1 min");
    test.advance_clock_past_timestamp(option_market.expiry as i64 + 1).await;
    mango_group_cookie.run_keeper(&mut test).await;

    println!("exchange writers tokens for quote tokens");
    {
        let clock = test.get_clock().await;
        assert!( (clock.unix_timestamp as u64) > option_market.expiry );
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        let writer_tokens = trade_data.number_of_writers_tokens;
        assert_eq!(writer_tokens, 10_000_000);
    }
    
    test.exchange_writers_tokens(&mango_group_cookie, om_key, option_market, 0, I80F48::from_num(1_000_000), ExchangeFor::ForQuoteTokens).await;
    {
        // check exchange
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        let writers_tokens = trade_data.number_of_writers_tokens;
        assert_eq!(writers_tokens, 9_000_000);

        let funds_underlying :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.underlying_token_index).await;
        let funds_quote :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.quote_token_index).await;
        assert_eq!(funds_underlying, funds_underlying_old);
        assert_eq!(funds_quote, funds_quote_old + 10_199_999); // floating point rounding problem

        let option_market_ck = test.load_account::<OptionMarket>(om_key).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 100_000_000 * 5);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 10_200_000 * 4);
        funds_underlying_old = funds_underlying;
        funds_quote_old =  funds_quote;
    }
    test.exchange_writers_tokens(&mango_group_cookie, om_key, option_market, 0, I80F48::from_num(1_000_000), ExchangeFor::ForUnderlyingTokens).await;
    {
        // check exchange
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        let writers_tokens = trade_data.number_of_writers_tokens;
        assert_eq!(writers_tokens, 8_000_000);

        let funds_underlying :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.underlying_token_index).await;
        let funds_quote :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.quote_token_index).await;
        assert_eq!(funds_underlying, funds_underlying_old + 100_000_000);
        assert_eq!(funds_quote, funds_quote_old); // floating point rounding problem

        let option_market_ck = test.load_account::<OptionMarket>(om_key).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 100_000_000 * 4);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 10_200_000 * 4);
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
//     
    let (om_key, option_market) = test.create_option_market(&mango_group_cookie,
        0,
        15,
         OptionType::European,
         I80F48::from_num(100_000_000), 
         I80F48::from_num(10_200_000), 
         expiry,
        Some(expiry_to_exercise)).await;

    assert_eq!(option_market.meta_data.data_type, DataType::OptionMarket as u8);
    assert_eq!(option_market.option_type, OptionType::European);
    assert_eq!(option_market.contract_size, I80F48::from_num(100_000_000));
    assert_eq!(option_market.quote_amount, I80F48::from_num(10_200_000));
    assert_eq!(option_market.expiry, expiry);
    assert_eq!(option_market.tokens_in_underlying_pool,  I80F48::from_num(0));
    assert_eq!(option_market.tokens_in_quote_pool,  I80F48::from_num(0));
    mango_group_cookie.run_keeper(&mut test).await;
    mango_group_cookie.run_keeper(&mut test).await;

    println!("Write option");
    // deposit some underlying tokens for user0
    let load_amount = 120_000_000 * 10;
    test.perform_deposit(&mango_group_cookie, 0, option_market.underlying_token_index, load_amount).await;
    // write option with user0
    let (u0_trade_data_pk, option_trade_data) = test.write_option(&mango_group_cookie, om_key, option_market, 0, I80F48::from_num(10_000_000)).await;
    //check write option
    {
        let option_tokens = option_trade_data.number_of_option_tokens;
        let writer_tokens = option_trade_data.number_of_writers_tokens;
        assert_eq!(option_tokens, 10_000_000);
        assert_eq!(writer_tokens, 10_000_000);
        let mango_account_key = mango_group_cookie.mango_accounts[0].address;
        let funds :u64 = test.with_mango_account_deposit(&mango_account_key, option_market.underlying_token_index).await;
        assert_eq!(load_amount - 1_000_000_000, funds );
    
        let option_market_ck = test.load_account::<OptionMarket>(om_key).await;
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

    test.exercise_option(&mango_group_cookie, om_key, option_market, 0, I80F48::from_num(5_000_000)).await;
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
        let option_market_ck = test.load_account::<OptionMarket>(om_key).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 100_000_000 * 5);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 10_200_000 * 5);
        funds_underlying_old= funds_underlying;
        funds_quote_old = funds_quote;
    }
    mango_group_cookie.run_keeper(&mut test).await;
    println!("sleeping for 1 min");
    test.advance_clock_past_timestamp(option_market.expiry_to_exercise_european as i64 + 1).await;
    mango_group_cookie.run_keeper(&mut test).await;

    println!("exchange writers tokens for quote tokens");
    {
        let clock = test.get_clock().await;
        assert!( (clock.unix_timestamp as u64) > option_market.expiry );
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        let writer_tokens = trade_data.number_of_writers_tokens;
        assert_eq!(writer_tokens, 10_000_000);
    }
    
    test.exchange_writers_tokens(&mango_group_cookie, om_key, option_market, 0, I80F48::from_num(1_000_000), ExchangeFor::ForQuoteTokens).await;
    {
        // check exchange
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        let writers_tokens = trade_data.number_of_writers_tokens;
        assert_eq!(writers_tokens, 9_000_000);

        let funds_underlying :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.underlying_token_index).await;
        let funds_quote :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.quote_token_index).await;
        assert_eq!(funds_underlying, funds_underlying_old);
        assert_eq!(funds_quote, funds_quote_old + 10_199_999); // floating point rounding problem

        let option_market_ck = test.load_account::<OptionMarket>(om_key).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 100_000_000 * 5);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 10_200_000 * 4);
        funds_underlying_old = funds_underlying;
        funds_quote_old =  funds_quote;
    }
    test.exchange_writers_tokens(&mango_group_cookie, om_key, option_market, 0, I80F48::from_num(1_000_000), ExchangeFor::ForUnderlyingTokens).await;
    {
        // check exchange
        let trade_data = test.load_account::<UserOptionTradeData>(u0_trade_data_pk).await;
        let writers_tokens = trade_data.number_of_writers_tokens;
        assert_eq!(writers_tokens, 8_000_000);

        let funds_underlying :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.underlying_token_index).await;
        let funds_quote :u64 = test.with_mango_account_deposit(&mango_account_u0_key, option_market.quote_token_index).await;
        assert_eq!(funds_underlying, funds_underlying_old + 100_000_000);
        assert_eq!(funds_quote, funds_quote_old); // floating point rounding problem

        let option_market_ck = test.load_account::<OptionMarket>(om_key).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 100_000_000 * 4);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 10_200_000 * 4);
        funds_underlying_old = funds_underlying;
        funds_quote_old =  funds_quote;
    }

}
