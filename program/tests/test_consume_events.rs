mod program_test;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;
use solana_program_test::*;

#[tokio::test]
async fn test_consume_events() {
    // === Arrange ===
    let config = MangoProgramTestConfig {
        consume_perp_events_count: 10,
        ..MangoProgramTestConfig::default_two_mints()
    };

    let mut test = MangoProgramTest::start_new(&config).await;

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
        (bidder_user_index, test.quote_index, 100.0 * base_price),
        (asker_user_index, mint_index, 100.0),
    ];

    let perp_orders = vec![
        (asker_user_index, mint_index, mango::matching::Side::Ask, base_size, base_price),
        (bidder_user_index, mint_index, mango::matching::Side::Bid, base_size, base_price),
    ];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step 2: Place matching spot orders several times
    for _ in 0..10 {
        place_perp_order_scenario(&mut test, &mut mango_group_cookie, &perp_orders).await;
    }

    // Step 3: Check that the call to consume events does not fail
    mango_group_cookie.consume_perp_events(&mut test).await;
}
