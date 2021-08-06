// Tests related to spot markets
mod program_test;
use program_test::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use solana_program::pubkey::Pubkey;
use solana_program_test::*;
use fixed::types::I80F48;

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
    let base_price = 10000;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price as f64).await;

    // Deposit amounts
    let user_deposits = vec![
        (user_index, test.quote_index, base_price),
    ];

    // Spot Orders
    let user_spot_orders = vec![
        (user_index, mint_index, serum_dex::matching::Side::Bid, 1, base_price),
    ];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, user_deposits).await;

    // Step 2: Place spot orders
    place_spot_order_scenario(&mut test, &mut mango_group_cookie, user_spot_orders).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
    assert_ne!(mango_account.spot_open_orders[mint_index], Pubkey::default());
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
    let base_price = 10_000;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price as f64).await;

    // Deposit amounts
    let user_deposits = vec![
        (bidder_user_index, test.quote_index, base_price),
        (asker_user_index, mint_index, 1),
    ];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(
        &mut test,
        &mut mango_group_cookie,
        user_deposits,
    ).await;

    // Step 2: Place and match spot order
    match_single_spot_order_scenario(
        &mut test,
        &mut mango_group_cookie,
        bidder_user_index,
        asker_user_index,
        mint_index,
        base_price,
    ).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let bidder_base_deposit =
        &mango_group_cookie.mango_accounts[bidder_user_index].mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index).unwrap();
    let asker_base_deposit =
        &mango_group_cookie.mango_accounts[asker_user_index].mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index).unwrap();

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
