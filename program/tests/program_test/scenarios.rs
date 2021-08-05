use crate::*;

pub async fn deposit_scenario(
    test: &mut MangoProgramTest,
    mango_group_cookie: &mut MangoGroupCookie,
    deposits: Vec<(usize, &Vec<u64>)>,
) {

    mango_group_cookie.run_keeper(test).await;

    for deposit in deposits {
        let (user_index, user_deposits) = deposit;
        for (mint_index, amount) in user_deposits.iter().enumerate() {
            if *amount > 0 {
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
    }

}

pub async fn match_single_spot_order_scenario(
    test: &mut MangoProgramTest,
    mango_group_cookie: &mut MangoGroupCookie,
    bidder_user_index: usize,
    asker_user_index: usize,
    mint_index: usize,
    price: u64,
) {

    // Step 2: Place a bid for 1 BTC @ 10_000 USDC
    mango_group_cookie.run_keeper(test).await;

    let mut spot_market_cookie = mango_group_cookie.spot_markets[mint_index];
    let starting_spot_order_id = 1000;
    spot_market_cookie.place_order(
        test,
        mango_group_cookie,
        bidder_user_index,
        starting_spot_order_id as u64,
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
        starting_spot_order_id + 1 as u64,
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
