mod program_test;
use anchor_lang::prelude::SolanaSysvar;
use mango::{matching::*, state::*};
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;
use solana_sdk::signer::Signer;
use std::{mem::size_of, mem::size_of_val};
use fixed::types::I80F48;
use solana_sdk::signature::Keypair;

#[tokio::test]
async fn test_options() {
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    let num_precreated_mango_users = 2;
    mango_group_cookie
        .full_setup(&mut test, num_precreated_mango_users, config.num_mints - 1)
        .await;
    println!("Creating option market");
    let clock = test.get_clock().await;
    let expiry = clock.unix_timestamp as u64 + 60 * 60 * 10 * solana_program::clock::DEFAULT_TICKS_PER_SECOND; // after 10 hrs
    let (om_key, option_market) = test.create_option_market(&mango_group_cookie,
        0,
        15,
         OptionType::American,
         I80F48::from_num(100_000_000), 
         I80F48::from_num(10_200_000), 
         expiry).await;
    assert_eq!(option_market.meta_data.data_type, DataType::OptionMarket as u8);
    assert_eq!(option_market.option_type, OptionType::American);
    assert_eq!(option_market.contract_size, I80F48::from_num(100_000_000));
    assert_eq!(option_market.quote_amount, I80F48::from_num(10_200_000));
    assert_eq!(option_market.expiry, expiry);
    assert_eq!(option_market.tokens_in_underlying_pool,  I80F48::from_num(0));
    assert_eq!(option_market.tokens_in_quote_pool,  I80F48::from_num(0));
    mango_group_cookie.run_keeper(&mut test).await;
    mango_group_cookie.run_keeper(&mut test).await;

    // deposit some underlying tokens for user0
    let load_amount = 120_000_000 * 10;
    test.perform_deposit(&mango_group_cookie, 0, option_market.underlying_token_index, load_amount).await;
    // write option with user0
    let (mint_key_acc_0, writer_key_acc_0) = test.write_option(&mango_group_cookie, om_key, option_market, 0, I80F48::from_num(10_000_000)).await;
    //check write option
    {
        let option_tokens = test.get_token_balance(mint_key_acc_0).await;
        let writer_tokens = test.get_token_balance(writer_key_acc_0).await;
        assert_eq!(option_tokens, 10_000_000);
        assert_eq!(writer_tokens, 10_000_000);
        let mango_account_key = mango_group_cookie.mango_accounts[0].address;
        let funds :u64 = test.with_mango_account_deposit(&mango_account_key, option_market.underlying_token_index).await;
        assert_eq!(load_amount - 1_000_000_000, funds );
    
        let option_market_ck = test.load_account::<OptionMarket>(om_key).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 100_000_000 * 10);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 0);
    }

    let user0 = Keypair::from_base58_string(&test.users[0].to_base58_string());;
    let mint_account_key_u1 = test.create_token_account(&test.users[1].pubkey(), &option_market.option_mint).await;
    let load_amount = 10_000_000 * 10;
    test.perform_deposit(&mango_group_cookie, 1, option_market.quote_token_index, load_amount).await;
    test.transfer_tokens(&user0, &mint_key_acc_0, &mint_account_key_u1, 9_000_000).await;
    test.excercise_option(&mango_group_cookie, om_key, option_market, 1, mint_account_key_u1, I80F48::from_num(5_000_000)).await;
    //check excercise
    {
        let option_tokens = test.get_token_balance(mint_account_key_u1).await;
        assert_eq!(option_tokens, 4_000_000);
        let mango_account_key = mango_group_cookie.mango_accounts[1].address;
        let funds_underlying :u64 = test.with_mango_account_deposit(&mango_account_key, option_market.underlying_token_index).await;
        let funds_quote :u64 = test.with_mango_account_deposit(&mango_account_key, option_market.quote_token_index).await;
        assert_eq!(funds_underlying, 500_000_000);
        assert_eq!(funds_quote, load_amount - 10_200_000 * 5);
        let option_market_ck = test.load_account::<OptionMarket>(om_key).await;
        assert_eq!(option_market_ck.tokens_in_underlying_pool, 100_000_000 * 5);
        assert_eq!(option_market_ck.tokens_in_quote_pool, 10_200_000 * 5);
    }
}