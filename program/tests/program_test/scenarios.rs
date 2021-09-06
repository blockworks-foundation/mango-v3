use crate::*;

#[allow(dead_code)]
pub fn arrange_deposit_all_scenario(
    test: &mut MangoProgramTest,
    user_index: usize,
    mint_amount: f64,
    quote_amount: f64,
) -> Vec<(usize, usize, f64)> {
    let mut user_deposits = Vec::new();
    for mint_index in 0..test.num_mints - 1 {
        user_deposits.push((user_index, mint_index, mint_amount));
    }
    user_deposits.push((user_index, test.quote_index, quote_amount));
    return user_deposits;
}

#[allow(dead_code)]
pub async fn deposit_scenario(
    test: &mut MangoProgramTest,
    mango_group_cookie: &mut MangoGroupCookie,
    deposits: &Vec<(usize, usize, f64)>,
) {
    mango_group_cookie.run_keeper(test).await;

    for deposit in deposits {
        let (user_index, mint_index, amount) = deposit;
        let mint = test.with_mint(*mint_index);
        let deposit_amount = (*amount * mint.unit) as u64;
        test.perform_deposit(&mango_group_cookie, *user_index, *mint_index, deposit_amount).await;
    }
}

#[allow(dead_code)]
pub async fn withdraw_scenario(
    test: &mut MangoProgramTest,
    mango_group_cookie: &mut MangoGroupCookie,
    withdraws: &Vec<(usize, usize, f64, bool)>,
) {
    mango_group_cookie.run_keeper(test).await;

    for withdraw in withdraws {
        let (user_index, mint_index, amount, allow_borrow) = withdraw;
        let mint = test.with_mint(*mint_index);
        let withdraw_amount = (*amount * mint.unit) as u64;
        test.perform_withdraw(
            &mango_group_cookie,
            *user_index,
            *mint_index,
            withdraw_amount,
            *allow_borrow,
        )
        .await;
    }
}

#[allow(dead_code)]
pub async fn place_spot_order_scenario(
    test: &mut MangoProgramTest,
    mango_group_cookie: &mut MangoGroupCookie,
    spot_orders: &Vec<(usize, usize, serum_dex::matching::Side, f64, f64)>,
) {
    mango_group_cookie.run_keeper(test).await;

    for spot_order in spot_orders {
        let (user_index, market_index, order_side, order_size, order_price) = *spot_order;
        let mut spot_market_cookie = mango_group_cookie.spot_markets[market_index];
        spot_market_cookie
            .place_order(test, mango_group_cookie, user_index, order_side, order_size, order_price)
            .await;

        mango_group_cookie.users_with_spot_event[market_index].push(user_index);
    }
}

#[allow(dead_code)]
pub async fn place_perp_order_scenario(
    test: &mut MangoProgramTest,
    mango_group_cookie: &mut MangoGroupCookie,
    perp_orders: &Vec<(usize, usize, mango::matching::Side, f64, f64)>,
) {
    mango_group_cookie.run_keeper(test).await;

    for perp_order in perp_orders {
        let (user_index, market_index, order_side, order_size, order_price) = *perp_order;
        let mut perp_market_cookie = mango_group_cookie.perp_markets[market_index];
        perp_market_cookie
            .place_order(test, mango_group_cookie, user_index, order_side, order_size, order_price)
            .await;

        mango_group_cookie.users_with_perp_event[market_index].push(user_index);
    }
}

#[allow(dead_code)]
pub async fn match_spot_order_scenario(
    test: &mut MangoProgramTest,
    mango_group_cookie: &mut MangoGroupCookie,
    matched_spot_orders: &Vec<Vec<(usize, usize, serum_dex::matching::Side, f64, f64)>>,
) {
    for matched_spot_order in matched_spot_orders {
        place_spot_order_scenario(test, mango_group_cookie, matched_spot_order).await;
        mango_group_cookie.run_keeper(test).await;
        mango_group_cookie.consume_spot_events(test).await;
        mango_group_cookie.run_keeper(test).await;
    }
}

#[allow(dead_code)]
pub async fn match_perp_order_scenario(
    test: &mut MangoProgramTest,
    mango_group_cookie: &mut MangoGroupCookie,
    matched_perp_orders: &Vec<Vec<(usize, usize, mango::matching::Side, f64, f64)>>,
) {
    for matched_perp_order in matched_perp_orders {
        place_perp_order_scenario(test, mango_group_cookie, matched_perp_order).await;
        mango_group_cookie.run_keeper(test).await;
        mango_group_cookie.consume_perp_events(test).await;
        mango_group_cookie.run_keeper(test).await;
    }
}
