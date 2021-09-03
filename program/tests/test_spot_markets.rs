mod program_test;
use program_test::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::assertions::*;
use solana_program::pubkey::Pubkey;
use solana_program_test::*;
use fixed::types::I80F48;
use std::collections::HashMap;

#[tokio::test]
async fn test_list_spot_market_on_serum() {
    // === Arrange ===
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
    // === Act ===
    let spot_market_cookie = test.list_spot_market(mint_index).await;
    // === Assert ===
    println!("Serum Market PK: {}", spot_market_cookie.market.to_string());
    // Todo: Figure out how to assert this
}

#[tokio::test]
async fn test_init_spot_markets() {

    // === Arrange ===
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
    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;

    // === Act ===
    test.add_oracles_to_mango_group(&mango_group_cookie.address).await;
    mango_group_cookie.add_spot_markets(&mut test, config.num_mints - 1).await;

    // === Assert ===
    // TODO: Figure out how to assert

}

#[tokio::test]
async fn test_place_spot_order() {
    // === Arrange ===
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

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let user_index: usize = 0;
    let mint_index: usize = 0;
    let base_price: f64 = 10_000.0;
    let base_size: f64 = 1.0;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Deposit amounts
    let user_deposits = vec![
        (user_index, test.quote_index, base_price),
    ];

    // Spot Orders
    let user_spot_orders = vec![
        (user_index, mint_index, serum_dex::matching::Side::Bid, base_size, base_price),
    ];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Place spot orders
    place_spot_order_scenario(&mut test, &mut mango_group_cookie, &user_spot_orders).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;
    let expected_values: HashMap<&str, I80F48> = [("quote_locked", I80F48::from_num(10000000000 as i64))].iter().cloned().collect();
    assert_user_spot_orders(
        &mut test,
        &mango_group_cookie,
        expected_values,
        user_index,
        mint_index,
    ).await;
    let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
    assert_ne!(mango_account.spot_open_orders[mint_index], Pubkey::default());

    let (quote_free, quote_locked, base_free, base_locked) = test.get_oo_info(
        &mango_group_cookie,
        user_index,
        mint_index,
    ).await;

    println!("quote_free: {}", quote_free);
    println!("quote_locked: {}", quote_locked);
    println!("base_free: {}", base_free);
    println!("base_locked: {}", base_locked);

    // TODO: More assertions
}

#[tokio::test]
async fn test_match_spot_order() {
    // === Arrange ===
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

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let bidder_user_index: usize = 0;
    let asker_user_index: usize = 1;
    let mint_index: usize = 0;
    let base_price: f64 = 10_000.0;
    let base_size: f64 = 1.0;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Deposit amounts
    let user_deposits = vec![
        (bidder_user_index, test.quote_index, base_price),
        (asker_user_index, mint_index, 1.0),
    ];

    // Matched Spot Orders
    let matched_spot_orders = vec![
        vec![
            (bidder_user_index, mint_index, serum_dex::matching::Side::Bid, 0.5, base_price),
            (asker_user_index, mint_index, serum_dex::matching::Side::Ask, base_size, base_price),
        ],
    ];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Place and match spot order
    match_spot_order_scenario(&mut test, &mut mango_group_cookie, &matched_spot_orders).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let bidder_base_deposit =
        &mango_group_cookie.mango_accounts[bidder_user_index].mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index).unwrap();
    let asker_base_deposit =
        &mango_group_cookie.mango_accounts[asker_user_index].mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index).unwrap();


    let (bidder_quote_free, bidder_quote_locked, bidder_base_free, bidder_base_locked) = test.get_oo_info(
        &mango_group_cookie,
        bidder_user_index,
        mint_index,
    ).await;
    println!("bidder_quote_free: {}", bidder_quote_free);
    println!("bidder_quote_locked: {}", bidder_quote_locked);
    println!("bidder_base_free: {}", bidder_base_free);
    println!("bidder_base_locked: {}", bidder_base_locked);

    let (asker_quote_free, asker_quote_locked, asker_base_free, asker_base_locked) = test.get_oo_info(
        &mango_group_cookie,
        asker_user_index,
        mint_index,
    ).await;
    println!("asker_quote_free: {}", asker_quote_free);
    println!("asker_quote_locked: {}", asker_quote_locked);
    println!("asker_base_free: {}", asker_base_free);
    println!("asker_base_locked: {}", asker_base_locked);

    // let bidder_quote_deposit =
    //     &mango_group_cookie.mango_accounts[bidder_user_index].mango_account
    //     .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[QUOTE_INDEX], QUOTE_INDEX).unwrap();
    // let asker_quote_deposit =
    //     &mango_group_cookie.mango_accounts[asker_user_index].mango_account
    //     .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[QUOTE_INDEX], QUOTE_INDEX).unwrap();

    assert_eq!(bidder_base_deposit.to_string(), I80F48::from_num(1000000).to_string());
    assert_eq!(asker_base_deposit.to_string(), I80F48::from_num(0).to_string());

    // TODO: Figure out if the weird quote deposits should be asserted

}
