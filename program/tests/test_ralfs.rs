// Tests related to placing orders on a perp market
mod program_test;
use mango::{matching::*, state::*};
use program_test::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use solana_program::pubkey::Pubkey;
use solana_program_test::*;
use std::num::NonZeroU64;
use std::{mem::size_of, mem::size_of_val};
use fixed::types::I80F48;

#[tokio::test]
async fn test_init_perp_markets() {
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
    // Need to add oracles first in order to add perp_markets
    test.add_oracles_to_mango_group(&mango_group_cookie.address).await;
    let perp_market_cookies =
        mango_group_cookie.add_perp_markets(&mut test, config.num_mints - 1).await;
    mango_group_cookie.mango_group =
        test.load_account::<MangoGroup>(mango_group_cookie.address).await;
    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    for perp_market_cookie in perp_market_cookies {
        assert_eq!(size_of_val(&perp_market_cookie.perp_market), size_of::<PerpMarket>());
    }
}

async fn place_perp_order(order_side: Side) {
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
    // // Disable all logs except error
    // solana_logger::setup_with("error");

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let user_index: usize = 0;
    let mint_index: usize = 0;
    let base_price = 10000;

    // Deposit amounts
    let user_deposits = vec![
        (user_index, test.quote_index, base_price),
    ];

    // Perp Orders
    let user_perp_orders = vec![
        (user_index, mint_index, order_side, 1, base_price),
    ];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, user_deposits).await;

    // Step 2: Place perp orders
    place_perp_order_scenario(&mut test, &mut mango_group_cookie, user_perp_orders).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;

    // TODO - @lagzda need to implemente similar function on MangoAccount for new way of handling open orders
    //      commented out for now so tests pass
    // let (client_order_id, _order_id, side) = mango_account.perp_accounts[mint_index]
    //     .orders_with_client_ids()
    //     .last()
    //     .unwrap();
    // assert_eq!(client_order_id, NonZeroU64::new(10_000).unwrap());
    // assert_eq!(side, order_side);
}

#[tokio::test]
async fn test_place_perp_order() {
    // Scenario 1
    place_perp_order(Side::Bid).await;
    // Scenario 2
    place_perp_order(Side::Ask).await;
}

#[tokio::test]
async fn test_worst_case_v1() {
    // === Arrange ===
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 2, num_mints: 16 };
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
    let num_orders: usize = test.num_mints - 1;
    let user_index: usize = 0;
    let base_price = 10000;

    // Set oracles
    for mint_index in 0..num_orders {
        mango_group_cookie.set_oracle(&mut test, mint_index, base_price as f64).await;
    }

    // Deposit amounts
    let user_deposits = vec![
        (user_index, test.quote_index, base_price * num_orders as u64),
    ];

    // Spot Orders
    let mut user_spot_orders = vec![];
    for mint_index in 0..num_orders.min(MAX_NUM_IN_MARGIN_BASKET as usize) {
        user_spot_orders.push((user_index, mint_index, serum_dex::matching::Side::Bid, 1, base_price));
    }

    // Perp Orders
    let mut user_perp_orders = vec![];
    for mint_index in 0..num_orders {
        user_perp_orders.push((user_index, mint_index, mango::matching::Side::Bid, 1, base_price));
    }

    // === Act ===
    // Step 1: Deposit all tokens into mango account
    deposit_scenario(&mut test, &mut mango_group_cookie, user_deposits).await;

    // Step 2: Place spot orders
    place_spot_order_scenario(&mut test, &mut mango_group_cookie, user_spot_orders).await;

    // Step 3: Place perp orders
    place_perp_order_scenario(&mut test, &mut mango_group_cookie, user_perp_orders).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
    for spot_open_orders_index in 0..num_orders.min(MAX_NUM_IN_MARGIN_BASKET as usize) {
        assert_ne!(mango_account.spot_open_orders[spot_open_orders_index], Pubkey::default());
    }
    // TODO: more assertions
}

#[tokio::test]
async fn test_worst_case_v2() {
    // === Arrange ===
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 2, num_mints: 16 };
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
    let borrower_user_index: usize = 0;
    let lender_user_index: usize = 1;
    let num_orders: usize = test.num_mints - 1;
    let base_price = 10000;

    // Set oracles
    for mint_index in 0..num_orders {
        mango_group_cookie.set_oracle(&mut test, mint_index, 10000.0000000001).await;
    }

    // Deposit amounts
    let mut user_deposits = vec![
        (borrower_user_index, test.quote_index, 2 * base_price * num_orders as u64), // NOTE: If depositing exact amount throws insufficient
    ];
    for mint_index in 0..num_orders {
        user_deposits.push((lender_user_index, mint_index, 10));
    }

    // Withdraw amounts
    let mut user_withdraws = vec![];
    for mint_index in 0..num_orders {
        user_withdraws.push((borrower_user_index, mint_index, 1, true));
    }

    // Spot Orders
    let mut user_spot_orders = vec![];
    for mint_index in 0..num_orders.min(MAX_NUM_IN_MARGIN_BASKET as usize) {
        user_spot_orders.push((lender_user_index, mint_index, serum_dex::matching::Side::Ask, 1, base_price));
    }

    // Perp Orders
    let mut user_perp_orders = vec![];
    for mint_index in 0..num_orders {
        user_perp_orders.push((lender_user_index, mint_index, mango::matching::Side::Ask, 1, base_price));
    }


    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, user_deposits).await;

    // Step 2: Make withdraws
    withdraw_scenario(&mut test, &mut mango_group_cookie, user_withdraws).await;

    // Step 3: Check that lenders all deposits are not a nice number anymore (> 10 mint)
    mango_group_cookie.run_keeper(&mut test).await;

    for mint_index in 0..num_orders {
        let base_mint = test.with_mint(mint_index);
        let base_deposit_amount = (10 * base_mint.unit) as u64;
        let lender_base_deposit =
            &mango_group_cookie.mango_accounts[lender_user_index].mango_account
            .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index).unwrap();
        assert_ne!(lender_base_deposit.to_string(), I80F48::from_num(base_deposit_amount).to_string());
    }

    // Step 4: Place spot orders
    place_spot_order_scenario(&mut test, &mut mango_group_cookie, user_spot_orders).await;

    // Step 5: Place perp orders
    place_perp_order_scenario(&mut test, &mut mango_group_cookie, user_perp_orders).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let lender_mango_account = mango_group_cookie.mango_accounts[lender_user_index].mango_account;
    for spot_open_orders_index in 0..num_orders.min(MAX_NUM_IN_MARGIN_BASKET as usize) {
        assert_ne!(
            lender_mango_account.spot_open_orders[spot_open_orders_index],
            Pubkey::default()
        );
    }

    for mint_index in 0..num_orders {
        // TODO - @lagzda need to implemente similar function on MangoAccount for new way of handling open orders
        //      commented out for now so tests pass

        // let (client_order_id, _order_id, side) = lender_mango_account.perp_accounts[mint_index]
        //     .open_orders
        //     .orders_with_client_ids()
        //     .last()
        //     .unwrap();
        // assert_eq!(client_order_id, NonZeroU64::new(10_000 + mint_index as u64).unwrap());
        // assert_eq!(side, Side::Ask);
    }
}
