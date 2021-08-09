mod program_test;
use mango::{matching::*, state::*};
use program_test::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use solana_program_test::*;
use std::num::NonZeroU64;
use std::{mem::size_of, mem::size_of_val};

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

#[tokio::test]
async fn test_place_perp_order() {
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
    let base_price: f64 = 10_000.0;
    let base_size: f64 = 1.0;

    // Deposit amounts
    let user_deposits = vec![
        (user_index, test.quote_index, base_price * base_size),
        (user_index, mint_index, base_size),
    ];

    // Perp Orders
    let user_perp_orders = vec![
        (user_index, mint_index, Side::Bid, 1.0, base_price),
        (user_index, mint_index, Side::Ask, 1.0, base_price * 2.0),
    ];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, user_deposits).await;

    // Step 2: Place perp orders
    place_perp_order_scenario(&mut test, &mut mango_group_cookie, &user_perp_orders).await;

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
    let perp_open_orders =
        mango_account.perp_accounts[mint_index].open_orders.orders_with_client_ids().collect::<Vec<(NonZeroU64, i128, Side)>>();

    assert_eq!(&perp_open_orders.len(), &user_perp_orders.len());

    for i in 0..user_perp_orders.len() {
        let (_, _, arranged_order_side, _, _) = user_perp_orders[i];
        let (client_order_id, _order_id, side) = perp_open_orders[i];
        assert_eq!(client_order_id, NonZeroU64::new(STARTING_PERP_ORDER_ID + i as u64).unwrap());
        assert_eq!(side, arranged_order_side);
    }

}
