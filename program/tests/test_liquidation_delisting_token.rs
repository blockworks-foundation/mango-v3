mod program_test;

use mango::state::*;
use program_test::cookies::*;
use program_test::*;
use solana_program_test::*;

#[tokio::test]
async fn test_liquidation_delisting_token_only_deposits() {
    let config = MangoProgramTestConfig::default_two_mints();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    let market_index: usize = 0;
    let price: u64 = 5;

    let liqee_index: usize = 0;
    let liqor_index: usize = 1;

    let liqee_deposit = 100;

    // Set asset price to expected value
    mango_group_cookie.set_oracle(&mut test, market_index, price as f64).await;

    // Deposit some asset to be delisted
    mango_group_cookie.run_keeper(&mut test).await;
    test.perform_deposit(&mango_group_cookie, liqee_index, market_index, liqee_deposit)
        .await
        .unwrap();

    // Set market to force close
    test.perform_set_market_mode(
        &mango_group_cookie,
        market_index,
        MarketMode::CloseOnly,
        AssetType::Token,
    )
    .await
    .unwrap();
    test.perform_set_market_mode(
        &mango_group_cookie,
        market_index,
        MarketMode::ForceCloseOnly,
        AssetType::Token,
    )
    .await
    .unwrap();

    // Expect deposit to be completely withdrawn to the liqee ATA
    test.perform_liquidate_delisting_token(
        &mango_group_cookie,
        liqee_index,
        liqor_index,
        market_index,
        test.quote_index,
    )
    .await
    .unwrap();
    let deposit_post = test
        .with_mango_account_deposit_I80F48(
            &mango_group_cookie.mango_accounts[liqee_index].address,
            market_index,
        )
        .await;

    assert!(deposit_post == ZERO_I80F48);
    // TODO: check the correct ATA gets the balance
}

#[tokio::test]
async fn test_liquidation_delisting_token_deposits_as_collateral() {
    let config = MangoProgramTestConfig::default_two_mints();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    let market_index: usize = 0;
    let price: u64 = 5;

    let liqee_index: usize = 0;
    let liqor_index: usize = 1;

    let liqee_deposit = 100;
    let liqee_borrow = 20;

    // Set asset price to expected value
    mango_group_cookie.set_oracle(&mut test, market_index, price as f64).await;

    // Deposit some asset to be delisted, withdraw some quote to make a borrow
    test.update_all_root_banks(&mango_group_cookie, &mango_group_cookie.address).await;
    test.cache_all_prices(
        &mango_group_cookie.mango_group,
        &mango_group_cookie.address,
        &mango_group_cookie.mango_group.oracles[0..mango_group_cookie.mango_group.num_oracles],
    )
    .await;
    test.perform_deposit(&mango_group_cookie, liqee_index, market_index, liqee_deposit)
        .await
        .unwrap();
    test.perform_deposit(&mango_group_cookie, liqor_index, test.quote_index, liqee_borrow * 5)
        .await
        .unwrap();
    test.perform_withdraw(&mango_group_cookie, liqee_index, test.quote_index, liqee_borrow, true)
        .await
        .unwrap();

    // Set market to force close
    test.perform_set_market_mode(
        &mango_group_cookie,
        market_index,
        MarketMode::CloseOnly,
        AssetType::Token,
    )
    .await
    .unwrap();
    test.perform_set_market_mode(
        &mango_group_cookie,
        market_index,
        MarketMode::ForceCloseOnly,
        AssetType::Token,
    )
    .await
    .unwrap();

    // Expect deposit to be completely withdrawn to the liqor ATA
    // Expect
    test.perform_liquidate_delisting_token(
        &mango_group_cookie,
        liqee_index,
        liqor_index,
        market_index,
        test.quote_index,
    )
    .await
    .unwrap();
    let deposit_post = test
        .with_mango_account_deposit(
            &mango_group_cookie.mango_accounts[liqee_index].address,
            market_index,
        )
        .await;
    println!("{}", deposit_post);
    assert!(deposit_post == 0);
    // TODO: check the correct ATA gets the balance
}

#[tokio::test]
async fn test_liquidation_delisting_token_borrows() {
    let config = MangoProgramTestConfig::default_two_mints();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    let market_index: usize = 0;
    let price: u64 = 5;

    let liqee_index: usize = 0;
    let liqor_index: usize = 1;

    let liqee_deposit = 100;
    let liqee_borrow = 10;

    // Set asset price to expected value
    mango_group_cookie.set_oracle(&mut test, market_index, price as f64).await;

    // Deposit some asset to be delisted, withdraw some quote to make a borrow
    test.update_all_root_banks(&mango_group_cookie, &mango_group_cookie.address).await;
    test.cache_all_prices(
        &mango_group_cookie.mango_group,
        &mango_group_cookie.address,
        &mango_group_cookie.mango_group.oracles[0..mango_group_cookie.mango_group.num_oracles],
    )
    .await;
    test.perform_deposit(&mango_group_cookie, liqee_index, test.quote_index, liqee_deposit)
        .await
        .unwrap();
    test.perform_deposit(&mango_group_cookie, liqor_index, market_index, liqee_borrow * 2)
        .await
        .unwrap();
    test.perform_deposit(&mango_group_cookie, liqor_index, test.quote_index, liqee_borrow * 10)
        .await
        .unwrap();
    test.perform_withdraw(&mango_group_cookie, liqee_index, market_index, liqee_borrow, true)
        .await
        .unwrap();

    let liqor_deposit_pre = test
        .with_mango_account_deposit_I80F48(
            &mango_group_cookie.mango_accounts[liqor_index].address,
            market_index,
        )
        .await;
    let borrow_pre = test
        .with_mango_account_borrow_I80F48(
            &mango_group_cookie.mango_accounts[liqee_index].address,
            market_index,
        )
        .await;

    // Set market to force close
    test.perform_set_market_mode(
        &mango_group_cookie,
        market_index,
        MarketMode::CloseOnly,
        AssetType::Token,
    )
    .await
    .unwrap();
    test.perform_set_market_mode(
        &mango_group_cookie,
        market_index,
        MarketMode::ForceCloseOnly,
        AssetType::Token,
    )
    .await
    .unwrap();

    // Expect deposit to be completely withdrawn to the liqor ATA
    // Expect
    test.perform_liquidate_delisting_token(
        &mango_group_cookie,
        liqee_index,
        liqor_index,
        market_index,
        test.quote_index,
    )
    .await
    .unwrap();
    // TODO: check i80f48 for dust
    let deposit_post = test
        .with_mango_account_deposit_I80F48(
            &mango_group_cookie.mango_accounts[liqee_index].address,
            market_index,
        )
        .await;
    assert!(deposit_post == ZERO_I80F48);
    let borrow_post = test
        .with_mango_account_borrow_I80F48(
            &mango_group_cookie.mango_accounts[liqee_index].address,
            market_index,
        )
        .await;
    let liqor_deposit_post = test
        .with_mango_account_deposit_I80F48(
            &mango_group_cookie.mango_accounts[liqor_index].address,
            market_index,
        )
        .await;

    assert!(borrow_post == ZERO_I80F48);
    println!("{} {} {}", liqor_deposit_post, liqor_deposit_pre, borrow_pre);
    // assert!(liqor_deposit_post == liqor_deposit_pre - borrow_pre);
    // TODO: check the correct ATA gets the balance
}
