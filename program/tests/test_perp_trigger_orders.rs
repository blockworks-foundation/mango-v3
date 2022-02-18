#![cfg(feature = "test-bpf")]

mod program_test;
use fixed::types::I80F48;
use mango::matching::{OrderType, Side};
use mango::state::{TriggerCondition, ADVANCED_ORDER_FEE};
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;

#[tokio::test]
async fn test_perp_trigger_orders_basic() {
    // === Arrange ===
    let config = MangoProgramTestConfig::default_two_mints();
    let mut test = MangoProgramTest::start_new(&config).await;
    // Disable all logs except error
    // solana_logger::setup_with("error");
    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let user_index: usize = 0;
    let user2_index: usize = 1;
    let mint_index: usize = 0;
    let base_price: f64 = 10_000.0;
    let base_size: f64 = 1.0;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Deposit
    let user_deposits = vec![
        (user_index, test.quote_index, base_price * base_size),
        (user_index, mint_index, base_size),
        (user2_index, test.quote_index, base_price * base_size),
        (user2_index, mint_index, base_size),
    ];
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Make an advanced orders account
    let mango_account_cookie = &mango_group_cookie.mango_accounts[user_index];
    let mut advanced_orders_cookie =
        AdvancedOrdersCookie::init(&mut test, mango_account_cookie).await;
    assert!(!advanced_orders_cookie.advanced_orders.orders[0].is_active);
    assert!(!advanced_orders_cookie.advanced_orders.orders[1].is_active);
    let advanced_orders_initial_lamports =
        test.get_account(advanced_orders_cookie.address).await.lamports;

    // Add two advanced orders
    let mut perp_market = mango_group_cookie.perp_markets[0];
    perp_market
        .add_trigger_order(
            &mut test,
            &mut mango_group_cookie,
            &mut advanced_orders_cookie,
            user_index,
            OrderType::Limit,
            Side::Bid,
            TriggerCondition::Above,
            base_price,
            base_size,
            I80F48::from_num(base_price * 1.1),
        )
        .await;
    assert!(advanced_orders_cookie.advanced_orders.orders[0].is_active);
    assert!(!advanced_orders_cookie.advanced_orders.orders[1].is_active);
    assert!(
        test.get_account(advanced_orders_cookie.address).await.lamports
            - advanced_orders_initial_lamports
            == ADVANCED_ORDER_FEE
    );
    perp_market
        .add_trigger_order(
            &mut test,
            &mut mango_group_cookie,
            &mut advanced_orders_cookie,
            user_index,
            OrderType::Limit,
            Side::Bid,
            TriggerCondition::Below,
            base_price * 0.91,
            base_size,
            I80F48::from_num(base_price * 0.9),
        )
        .await;
    assert!(advanced_orders_cookie.advanced_orders.orders[0].is_active);
    assert!(advanced_orders_cookie.advanced_orders.orders[1].is_active);
    assert!(
        test.get_account(advanced_orders_cookie.address).await.lamports
            - advanced_orders_initial_lamports
            == 2 * ADVANCED_ORDER_FEE
    );

    // Remove the first advanced order
    advanced_orders_cookie
        .remove_advanced_order(&mut test, &mut mango_group_cookie, user_index, 0)
        .await
        .expect("deletion succeeds");
    assert!(!advanced_orders_cookie.advanced_orders.orders[0].is_active);
    assert!(advanced_orders_cookie.advanced_orders.orders[1].is_active);
    assert!(
        test.get_account(advanced_orders_cookie.address).await.lamports
            - advanced_orders_initial_lamports
            == ADVANCED_ORDER_FEE
    );
    // advance slots, since we want to send the same tx a second time
    test.advance_clock_by_slots(2).await;
    advanced_orders_cookie
        .remove_advanced_order(&mut test, &mut mango_group_cookie, user_index, 0)
        .await
        .expect("deletion of inactive is ok");
    advanced_orders_cookie
        .remove_advanced_order(&mut test, &mut mango_group_cookie, user_index, 2)
        .await
        .expect("deletion of unused is ok");

    // Trigger the second advanced order
    let agent_user_index = user2_index;
    perp_market
        .execute_trigger_order(
            &mut test,
            &mut mango_group_cookie,
            &mut advanced_orders_cookie,
            user_index,
            agent_user_index,
            1,
        )
        .await
        .expect_err("order trigger condition should not be met");
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price * 0.89).await;
    mango_group_cookie.run_keeper(&mut test).await;
    perp_market
        .execute_trigger_order(
            &mut test,
            &mut mango_group_cookie,
            &mut advanced_orders_cookie,
            user_index,
            agent_user_index,
            1,
        )
        .await
        .expect("order executed");
    assert!(!advanced_orders_cookie.advanced_orders.orders[1].is_active);
    assert!(
        test.get_account(advanced_orders_cookie.address).await.lamports
            - advanced_orders_initial_lamports
            == 0
    );

    // Check that order is in book now
    mango_group_cookie.run_keeper(&mut test).await;
    let user_perp_orders = vec![(user_index, mint_index, Side::Bid, base_size, base_price)];
    assert_open_perp_orders(&mango_group_cookie, &user_perp_orders, STARTING_ADVANCED_ORDER_ID + 1);
}

#[tokio::test]
async fn test_perp_trigger_orders_health() {
    // === Arrange ===
    let config = MangoProgramTestConfig::default_two_mints();
    let mut test = MangoProgramTest::start_new(&config).await;
    // Disable all logs except error
    // solana_logger::setup_with("error");
    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let user_index: usize = 0;
    let user2_index: usize = 1;
    let agent_user_index = user2_index;
    let mint_index: usize = 0;
    let base_price: f64 = 10_000.0;
    let base_size: f64 = 1.0;
    let mint = test.with_mint(mint_index);

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Deposit
    let user_deposits = vec![
        (user_index, test.quote_index, base_price * base_size),
        //(user_index, mint_index, base_size),
        (user2_index, test.quote_index, base_price * base_size),
        (user2_index, mint_index, base_size),
    ];
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Make an advanced orders account
    let mango_account_cookie = &mango_group_cookie.mango_accounts[user_index];
    let mut advanced_orders_cookie =
        AdvancedOrdersCookie::init(&mut test, mango_account_cookie).await;

    // Add trigger orders
    let mut perp_market = mango_group_cookie.perp_markets[0];
    perp_market
        .add_trigger_order(
            &mut test,
            &mut mango_group_cookie,
            &mut advanced_orders_cookie,
            user_index,
            OrderType::Limit,
            Side::Ask,
            TriggerCondition::Above,
            base_price,
            11.0 * base_size,
            I80F48::from_num(base_price * 0.01),
        )
        .await;
    perp_market
        .add_trigger_order(
            &mut test,
            &mut mango_group_cookie,
            &mut advanced_orders_cookie,
            user_index,
            OrderType::Limit,
            Side::Ask,
            TriggerCondition::Above,
            base_price,
            9.0 * base_size,
            I80F48::from_num(base_price * 0.01),
        )
        .await;
    perp_market
        .add_trigger_order(
            &mut test,
            &mut mango_group_cookie,
            &mut advanced_orders_cookie,
            user_index,
            OrderType::Limit,
            Side::Ask,
            TriggerCondition::Above,
            base_price,
            0.001 * base_size,
            I80F48::from_num(base_price * 0.01),
        )
        .await;
    perp_market
        .add_trigger_order(
            &mut test,
            &mut mango_group_cookie,
            &mut advanced_orders_cookie,
            user_index,
            OrderType::Market,
            Side::Bid,
            TriggerCondition::Above,
            0.99 * base_price,
            0.001 * base_size,
            I80F48::from_num(base_price * 0.01),
        )
        .await;

    // Triggering order 0 would drop health too much returns ok, but doesn't add
    // the order to the book due to health
    perp_market
        .execute_trigger_order(
            &mut test,
            &mut mango_group_cookie,
            &mut advanced_orders_cookie,
            user_index,
            agent_user_index,
            0,
        )
        .await
        .expect("order triggered, but not added to book");
    assert!(
        mango_group_cookie.mango_accounts[user_index].mango_account.perp_accounts[0].asks_quantity
            == 0
    );

    // Triggering order 1 is acceptable but brings health to the brink
    perp_market
        .execute_trigger_order(
            &mut test,
            &mut mango_group_cookie,
            &mut advanced_orders_cookie,
            user_index,
            agent_user_index,
            1,
        )
        .await
        .expect("order triggered, added to book");
    assert!(
        mango_group_cookie.mango_accounts[user_index].mango_account.perp_accounts[0].asks_quantity
            == 90_000
    );

    // Change the price oracle to make the account unhealthy
    mango_group_cookie.set_oracle(&mut test, mint_index, 2.0 * base_price).await;
    mango_group_cookie.run_keeper(&mut test).await;

    // Triggering order 2 would decrease health a tiny bit - not allowed
    perp_market
        .execute_trigger_order(
            &mut test,
            &mut mango_group_cookie,
            &mut advanced_orders_cookie,
            user_index,
            agent_user_index,
            2,
        )
        .await
        .expect("order triggered, but not added to book");
    assert!(
        mango_group_cookie.mango_accounts[user_index].mango_account.perp_accounts[0].bids_quantity
            == 0
    );
    assert!(
        mango_group_cookie.mango_accounts[user_index].mango_account.perp_accounts[0].asks_quantity
            == 90_000
    );

    // Add an order for user1 to trade against
    perp_market
        .place_order(
            &mut test,
            &mut mango_group_cookie,
            user2_index,
            Side::Ask,
            base_size,
            0.99 * base_price,
            None,
        )
        .await;

    // Triggering order 3 improves health and is allowed
    perp_market
        .execute_trigger_order(
            &mut test,
            &mut mango_group_cookie,
            &mut advanced_orders_cookie,
            user_index,
            agent_user_index,
            3,
        )
        .await
        .expect("order triggered");
    assert!(
        mango_group_cookie.mango_accounts[user_index].mango_account.perp_accounts[0].taker_base
            == test.base_size_number_to_lots(&mint, 0.001 * base_size) as i64
    );
}
