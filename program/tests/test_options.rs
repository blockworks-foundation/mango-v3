mod program_test;
use anchor_lang::prelude::SolanaSysvar;
use mango::{matching::*, state::*};
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;
use std::{mem::size_of, mem::size_of_val};
use fixed::types::I80F48;
use anchor_spl::{token};

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
    let (om__key, option_market) = test.create_option_market(&mango_group_cookie,
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

    let load_amount = 120_000_000 * 10;
    test.perform_deposit(&mango_group_cookie, 0, option_market.underlying_token_index, load_amount).await;
    let (mint_key_0, writer_key_0) = test.write_option(&mango_group_cookie, om__key, option_market, 0, I80F48::from_num(10.0)).await;
    let option_tokens = test.get_token_balance(mint_key_0).await;
    let writer_tokens = test.get_token_balance(writer_key_0).await;
    assert_eq!(option_tokens, 10_000_000);
    assert_eq!(writer_tokens, 10_000_000);
    let mango_account_key = mango_group_cookie.mango_accounts[0].address;
    let funds :u64 = test.with_mango_account_deposit(&mango_account_key, option_market.underlying_token_index).await;
    assert_eq!(load_amount - 1_000_000_000, funds );
}