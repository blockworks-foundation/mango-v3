mod program_test;
use anchor_lang::prelude::SolanaSysvar;
use mango::{matching::*, state::*};
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;
use std::{mem::size_of, mem::size_of_val};

#[tokio::test]
async fn test_init_option_market() {
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
    let option_market = test.create_option_market(&mango_group_cookie, OptionType::American, 0, 100000000, 10000000, expiry).await;
    assert_eq!(option_market.meta_data.data_type, DataType::OptionMarket as u8);
    assert_eq!(option_market.option_type, OptionType::American);
    assert_eq!(option_market.contract_size, 100000000);
    assert_eq!(option_market.quote_amount, 10000000);
    assert_eq!(option_market.expiry, expiry);
}