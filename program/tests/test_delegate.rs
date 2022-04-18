use std::collections::HashMap;

use fixed::types::I80F48;
use solana_program_test::*;

use mango::state::ZERO_I80F48;
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;

mod program_test;

#[tokio::test]
async fn test_delegate() {
    // === Arrange ===
    let config = MangoProgramTestConfig::default_two_mints();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let user_index: usize = 0;
    let delegate_user_index: usize = 1;
    let mint_index: usize = 0;
    let base_price: f64 = 10_000.0;
    let base_size: f64 = 1.0;
    let quote_mint = test.quote_mint;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price).await;

    // Deposit amounts
    let user_deposits = vec![(user_index, test.quote_index, base_price * 3.)];

    // Withdraw amounts
    let user_withdraw_with_delegate =
        (user_index, delegate_user_index, test.quote_index, base_price, false);

    // Spot Orders
    let user_spot_orders = (
        user_index,
        delegate_user_index,
        mint_index,
        serum_dex::matching::Side::Bid,
        base_size,
        base_price,
    );

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;

    // Step2: Setup delegate authority which can place orders on behalf
    delegate_scenario(&mut test, &mut mango_group_cookie, user_index, delegate_user_index).await;

    // Step 3: Place spot orders
    place_spot_order_scenario_with_delegate(&mut test, &mut mango_group_cookie, &user_spot_orders)
        .await
        .unwrap();

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let expected_values_vec: Vec<(usize, usize, HashMap<&str, I80F48>)> = vec![(
        mint_index, // Mint index
        user_index, // User index
        [
            ("quote_free", ZERO_I80F48),
            ("quote_locked", test.to_native(&quote_mint, base_price * base_size)),
            ("base_free", ZERO_I80F48),
            ("base_locked", ZERO_I80F48),
        ]
        .iter()
        .cloned()
        .collect(),
    )];

    for expected_values in expected_values_vec {
        assert_user_spot_orders(&mut test, &mango_group_cookie, expected_values).await;
    }

    // Step 4: Withdraw, should fail
    withdraw_scenario_with_delegate(
        &mut test,
        &mut mango_group_cookie,
        &user_withdraw_with_delegate,
    )
    .await
    .unwrap_err();

    // Step5: Reset delegate
    reset_delegate_scenario(&mut test, &mut mango_group_cookie, user_index).await;

    // Step6: Test placing orders again, should fail
    place_spot_order_scenario_with_delegate(&mut test, &mut mango_group_cookie, &user_spot_orders)
        .await
        .unwrap_err();
}
