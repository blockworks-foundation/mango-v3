use std::collections::HashMap;
use std::num::NonZeroU64;
use solana_program::pubkey::Pubkey;
use crate::*;

#[allow(dead_code)]
pub fn assert_open_spot_orders(
    mango_group_cookie: &MangoGroupCookie,
    user_spot_orders: &Vec<(usize, usize, serum_dex::matching::Side, f64, f64)>,
    // TODO: Can we get order_id to assert too?
) {

    for i in 0..user_spot_orders.len() {
        let (user_index, mint_index, arranged_order_side, _, _) = user_spot_orders[i];
        let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
        assert_ne!(
            mango_account.spot_open_orders[mint_index],
            Pubkey::default()
        );
    }

}

#[allow(dead_code)]
pub fn assert_open_perp_orders(
    mango_group_cookie: &MangoGroupCookie,
    user_perp_orders: &Vec<(usize, usize, mango::matching::Side, f64, f64)>,
    starting_order_id: u64,
) {

    let mut perp_orders_map: HashMap<String, usize> = HashMap::new();

    for i in 0..user_perp_orders.len() {

        let (user_index, mint_index, arranged_order_side, _, _) = user_perp_orders[i];
        let perp_orders_map_key = format!("{}_{}", user_index, mint_index);
        if let Some(x) = perp_orders_map.get_mut(&perp_orders_map_key) {
            *x += 1;
        } else {
            perp_orders_map.insert(perp_orders_map_key.clone(), 0);
        }

        let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
        let perp_open_orders = mango_account.perp_accounts[mint_index]
            .open_orders
            .orders_with_client_ids()
            .collect::<Vec<(NonZeroU64, i128, mango::matching::Side)>>();

        let (client_order_id, _order_id, side) = perp_open_orders[perp_orders_map[&perp_orders_map_key]];
        assert_eq!(
            client_order_id,
            NonZeroU64::new(starting_order_id + i as u64).unwrap()
        );
        assert_eq!(side, arranged_order_side);

    }

}


// #[allow(dead_code)]
// pub fn assert_perp_accounts(
//     mango_group_cookie: &MangoGroupCookie,
//     user_perp_orders: &Vec<(usize, usize, mango::matching::Side, f64, f64)>,
// )
