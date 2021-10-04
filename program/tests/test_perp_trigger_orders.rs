#![cfg(feature = "test-bpf")]

mod program_test;
use fixed::types::I80F48;
use mango::matching::{OrderType, Side};
use mango::state::TriggerCondition;
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;

#[tokio::test]
async fn test_init_perp_trigger_orders() {
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
    perp_market
        .add_trigger_order(
            &mut test,
            &mut mango_group_cookie,
            &mut advanced_orders_cookie,
            user_index,
            OrderType::Limit,
            Side::Bid,
            TriggerCondition::Below,
            base_price,
            base_size,
            I80F48::from_num(base_price * 0.9),
        )
        .await;
    assert!(advanced_orders_cookie.advanced_orders.orders[0].is_active);
    assert!(advanced_orders_cookie.advanced_orders.orders[1].is_active);

    // Remove the first advanced order
    advanced_orders_cookie
        .remove_advanced_order(&mut test, &mut mango_group_cookie, user_index, 0)
        .await
        .expect("deletion succeeds");
    assert!(!advanced_orders_cookie.advanced_orders.orders[0].is_active);
    assert!(advanced_orders_cookie.advanced_orders.orders[1].is_active);
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
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price * 0.8).await;
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

    // Check that order is in book now
    mango_group_cookie.run_keeper(&mut test).await;
    let user_perp_orders = vec![(user_index, mint_index, Side::Bid, base_size, base_price)];
    assert_open_perp_orders(&mango_group_cookie, &user_perp_orders, STARTING_ADVANCED_ORDER_ID + 1);
}
