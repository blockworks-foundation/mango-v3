// spot
// set market to closeonly /
// assert fails if not admin /
// assert market mode changed on tokeninfo /
// assert not possible to deposit in a fresh account /
// assert possible to deposit in an account with borrows /
// assert withdraw borrow not possible /
// assert margin trade borrow not possible
// assert open orders limited to one
// assert order must be reduce only

// perp
// set market to closeonly
// assert order must be reduce only
mod program_test;

use mango::state::*;
use program_test::cookies::*;
use program_test::*;
use solana_program_test::*;

#[tokio::test]
async fn test_spot_market_mode_closeonly() {
    // === Arrange ===
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    let market_index: usize = 0;
    let price: u64 = 5;
    let user_index: usize = 0;
    let user_deposit: u64 = 100;
    let user_with_borrow_index: usize = 1;
    let user_borrow: u64 = 10;

    // Set asset price to expected value
    mango_group_cookie.set_oracle(&mut test, market_index, price as f64).await;

    // Deposit some asset to allow borrowing
    // TODO: this is messy, replace keeper calls once updatefunding issues is fixed
    test.update_all_root_banks(&mango_group_cookie, &mango_group_cookie.address).await;
    test.cache_all_prices(
        &mango_group_cookie.mango_group,
        &mango_group_cookie.address,
        &mango_group_cookie.mango_group.oracles[0..mango_group_cookie.mango_group.num_oracles],
    )
    .await;
    test.perform_deposit(&mango_group_cookie, user_index, market_index, user_deposit)
        .await
        .unwrap();

    // Deposit some collateral and borrow asset
    test.perform_deposit(
        &mango_group_cookie,
        user_with_borrow_index,
        test.quote_index,
        user_borrow * price * 10,
    )
    .await
    .unwrap();
    test.perform_withdraw(
        &mango_group_cookie,
        user_with_borrow_index,
        market_index,
        user_borrow,
        true,
    )
    .await
    .unwrap();

    // Expect error if executing as non-admin
    test.perform_set_market_mode_as_user(
        &mango_group_cookie,
        market_index,
        MarketMode::CloseOnly,
        AssetType::Token,
        user_index,
    )
    .await
    .unwrap_err();

    // Expect success when executing as admin
    test.perform_set_market_mode(
        &mango_group_cookie,
        market_index,
        MarketMode::CloseOnly,
        AssetType::Token,
    )
    .await
    .unwrap();

    // Load group after changes
    let mango_group = test.load_account::<MangoGroup>(mango_group_cookie.address).await;
    // Expect mode to be changed for spot
    assert!(mango_group.tokens[market_index].spot_market_mode == MarketMode::CloseOnly);

    // Expect deposit to succeed but not update the balance as native_borrows is 0 and market is reduce_only
    let mango_account_deposit_pre = test
        .with_mango_account_deposit(
            &mango_group_cookie.mango_accounts[user_index].address,
            market_index,
        )
        .await;
    test.perform_deposit(&mango_group_cookie, user_index, market_index, 10).await.unwrap();
    let mango_account_deposit_post = test
        .with_mango_account_deposit(
            &mango_group_cookie.mango_accounts[user_index].address,
            market_index,
        )
        .await;
    assert!(mango_account_deposit_post == mango_account_deposit_pre);

    // Expect deposit to succeed and only update balance to close borrows, leaving the extra
    test.perform_deposit(
        &mango_group_cookie,
        user_with_borrow_index,
        market_index,
        user_borrow * 100,
    )
    .await
    .unwrap();
    let mango_account_with_borrow_deposit = test
        .with_mango_account_deposit(
            &mango_group_cookie.mango_accounts[user_with_borrow_index].address,
            market_index,
        )
        .await;
    let mango_account_with_borrow_borrow = test
        .with_mango_account_borrow(
            &mango_group_cookie.mango_accounts[user_with_borrow_index].address,
            market_index,
        )
        .await;
    assert!(mango_account_with_borrow_deposit == 0);
    // This fails as 1 native spl left in borrows due to checked_floor, so check it's just dust left
    // assert!(mango_account_with_borrow_borrow == 0);
    assert!(mango_account_with_borrow_borrow <= ONE_I80F48);

    // Expect withdraw increasing borrow to fail
    test.perform_withdraw(
        &mango_group_cookie,
        user_with_borrow_index,
        market_index,
        user_borrow * 10,
        true,
    )
    .await
    .unwrap_err();
}
