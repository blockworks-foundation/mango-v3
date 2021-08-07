use crate::*;

#[allow(dead_code)]
pub async fn deposit_scenario(
    test: &mut MangoProgramTest,
    mango_group_cookie: &mut MangoGroupCookie,
    deposits: Vec<(usize, usize, u64)>,
) {

    mango_group_cookie.run_keeper(test).await;

    for deposit in deposits {
        let (user_index, mint_index, amount) = deposit;
        let mint = test.with_mint(mint_index);
        let deposit_amount = (amount * mint.unit) as u64;
        test.perform_deposit(
            &mango_group_cookie,
            user_index,
            mint_index,
            deposit_amount,
        ).await;
    }
}

#[allow(dead_code)]
pub async fn withdraw_scenario(
    test: &mut MangoProgramTest,
    mango_group_cookie: &mut MangoGroupCookie,
    withdraws: Vec<(usize, usize, u64, bool)>,
) {

    mango_group_cookie.run_keeper(test).await;

    for withdraw in withdraws {
        let (user_index, mint_index, amount, allow_borrow) = withdraw;
        let mint = test.with_mint(mint_index);
        let withdraw_amount = (amount * mint.unit) as u64;
        test.perform_withdraw(
            &mango_group_cookie,
            user_index,
            mint_index,
            withdraw_amount,
            allow_borrow,
        ).await;
    }
}

#[allow(dead_code)]
pub async fn place_spot_order_scenario(
    test: &mut MangoProgramTest,
    mango_group_cookie: &mut MangoGroupCookie,
    spot_orders: Vec<(usize, usize, serum_dex::matching::Side, u64, u64)>,
) {

    mango_group_cookie.run_keeper(test).await;

    for spot_order in spot_orders {
        let (user_index, market_index, order_side, order_size, order_price) = spot_order;
        let mut spot_market_cookie = mango_group_cookie.spot_markets[market_index];
        spot_market_cookie.place_order(
            test,
            mango_group_cookie,
            user_index,
            order_side,
            order_size,
            order_price,
        ).await;
    }

}

#[allow(dead_code)]
pub async fn place_perp_order_scenario(
    test: &mut MangoProgramTest,
    mango_group_cookie: &mut MangoGroupCookie,
    perp_orders: Vec<(usize, usize, mango::matching::Side, u64, u64)>,
) {

    mango_group_cookie.run_keeper(test).await;

    for perp_order in perp_orders {
        let (user_index, market_index, order_side, order_size, order_price) = perp_order;
        let mut perp_market_cookie = mango_group_cookie.perp_markets[market_index];
        perp_market_cookie.place_order(
            test,
            mango_group_cookie,
            user_index,
            order_side,
            order_size,
            order_price,
        ).await;
    }

}



#[allow(dead_code)]
pub async fn match_single_spot_order_scenario(
    test: &mut MangoProgramTest,
    mango_group_cookie: &mut MangoGroupCookie,
    bidder_user_index: usize,
    asker_user_index: usize,
    mint_index: usize,
    price: u64,
    // TODO: Allow order size selection
) {

    // Step 2: Place a bid for 1 BTC @ 10_000 USDC
    mango_group_cookie.run_keeper(test).await;

    let mut spot_market_cookie = mango_group_cookie.spot_markets[mint_index];
    spot_market_cookie.place_order(
        test,
        mango_group_cookie,
        bidder_user_index,
        serum_dex::matching::Side::Bid,
        1,
        price,
    ).await;


    // Step 3: Place an ask for 1 BTC @ 10_000 USDC
    mango_group_cookie.run_keeper(test).await;

    spot_market_cookie.place_order(
        test,
        mango_group_cookie,
        asker_user_index,
        serum_dex::matching::Side::Ask,
        1,
        price,
    ).await;

    // Step 4: Consume events
    mango_group_cookie.run_keeper(test).await;

    test.consume_events(
        &spot_market_cookie,
        vec![
            &mango_group_cookie.mango_accounts[bidder_user_index].mango_account.spot_open_orders[0],
            &mango_group_cookie.mango_accounts[asker_user_index].mango_account.spot_open_orders[0],
        ],
        bidder_user_index,
        mint_index,
    ).await;

    // Step 5: Settle funds so that deposits get updated
    mango_group_cookie.run_keeper(test).await;

    // Settling bidder
    spot_market_cookie.settle_funds(
        test,
        mango_group_cookie,
        bidder_user_index,
    ).await;

    // Settling asker
    spot_market_cookie.settle_funds(
        test,
        mango_group_cookie,
        asker_user_index,
    ).await;

}
