use std::collections::HashMap;
use solana_program::pubkey::Pubkey;
use fixed::types::I80F48;
use mango::state::*;
use crate::*;

#[allow(dead_code)]
pub fn assert_open_spot_orders(
    mango_group_cookie: &MangoGroupCookie,
    user_spot_orders: &Vec<(usize, usize, serum_dex::matching::Side, f64, f64)>,
) {

    for i in 0..user_spot_orders.len() {
        let (user_index, mint_index, _, _, _) = user_spot_orders[i];
        let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
        assert_ne!(
            mango_account.spot_open_orders[mint_index],
            Pubkey::default()
        );
    }

}

#[allow(dead_code)]
pub async fn assert_user_spot_orders(
    test: &mut MangoProgramTest,
    mango_group_cookie: &MangoGroupCookie,
    expected_values: HashMap<&str, I80F48>,
    user_index: usize,
    mint_index: usize,
) {
    let (actual_quote_free, actual_quote_locked, actual_base_free, actual_base_locked) = test.get_oo_info(
        &mango_group_cookie,
        user_index,
        mint_index,
    ).await;
    if let Some(quote_free) = expected_values.get("quote_free") {
        println!("quote_free: {}", quote_free.to_string());
        println!("actual_quote_free: {}", actual_quote_free.to_string());
        assert!(*quote_free == actual_quote_free);
    }
    if let Some(quote_locked) = expected_values.get("quote_locked") {
        println!("quote_locked: {}", quote_locked.to_string());
        println!("actual_quote_locked: {}", actual_quote_locked.to_string());
        assert!(*quote_locked == actual_quote_locked);
    }
    if let Some(base_free) = expected_values.get("base_free") {
        println!("base_free: {}", base_free.to_string());
        println!("actual_base_free: {}", actual_base_free.to_string());
        assert!(*base_free == actual_base_free);
    }
    if let Some(base_locked) = expected_values.get("base_locked") {
        println!("base_locked: {}", base_locked.to_string());
        println!("actual_base_locked: {}", actual_base_locked.to_string());
        assert!(*base_locked == actual_base_locked);
    }
}

// #[allow(dead_code)]
// pub fn assert_matched_spot_orders(
//     mango_group_cookie: &MangoGroupCookie,
//     user_spot_orders: &Vec<(usize, usize, serum_dex::matching::Side, f64, f64)>,
// ) {
//     let mut balances_map: HashMap<String, (f64, f64)> = HashMap::new();
//     for i in 0..user_spot_orders.len() {
//         let (user_index, _, arranged_order_side, arranged_order_size, arranged_order_price) = user_spot_orders[i];
//         let balances_map_key = format!("{}", user_index);
//         let sign = match arranged_order_side {
//             serum_dex::matching::Side::Bid => 1.0,
//             serum_dex::matching::Side::Ask => -1.0,
//         }
//         if let Some((base_balance, quote_balance)) = balances_map.get_mut(&balances_map_key) {
//             *base_balance += arranged_order_size * arranged_order_price * sign;
//             *quote_balance += arranged_order_size * arranged_order_price * (sign * -1.0);
//         } else {
//             perp_orders_map.insert(perp_orders_map_key.clone(), 0);
//         }
//     }
// }

#[allow(dead_code)]
pub fn assert_open_perp_orders(
    mango_group_cookie: &MangoGroupCookie,
    user_perp_orders: &Vec<(usize, usize, mango::matching::Side, f64, f64)>,
    starting_order_id: u64,
) {

    let mut perp_orders_map: HashMap<String, usize> = HashMap::new();

    for i in 0..user_perp_orders.len() {
        let (user_index, _, arranged_order_side, _, _) = user_perp_orders[i];
        let perp_orders_map_key = format!("{}", user_index);
        if let Some(x) = perp_orders_map.get_mut(&perp_orders_map_key) {
            *x += 1;
        } else {
            perp_orders_map.insert(perp_orders_map_key.clone(), 0);
        }
        let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
        let client_order_id = mango_account.client_order_ids[perp_orders_map[&perp_orders_map_key]];
        let order_side = mango_account.order_side[perp_orders_map[&perp_orders_map_key]];
        assert_eq!(
            client_order_id,
            starting_order_id + i as u64,
        );
        assert_eq!(order_side, arranged_order_side);
    }

}

// #[allow(dead_code)]
// pub fn assert_matched_perp_orders(
//     test: &mut MangoProgramTest,
//     mango_group_cookie: &MangoGroupCookie,
//     user_perp_orders: &Vec<(usize, usize, mango::matching::Side, f64, f64)>,
// ) {
//     let mut matched_perp_orders_map: HashMap<String, I80F48> = HashMap::new();
//     let (_, _, _, maker_side, _) = user_perp_orders[0];
//     for i in 0..user_perp_orders.len() {
//         let (user_index, mint_index, arranged_order_side, arranged_order_size, arranged_order_price) = user_perp_orders[i];
//         let mango_group = mango_group_cookie.mango_group;
//         let perp_market_info = mango_group.perp_markets[mint_index];
//
//         let mint = test.with_mint(mint_index);
//
//         let order_size = test.base_size_number_to_lots(&self.mint, arranged_order_size);
//         let order_price = test.price_number_to_lots(&self.mint, arranged_order_price);
//
//         let mut taker = None;
//         let mut base_position: I80F48;
//         let mut quote_position: I80F48;
//
//         let fee = maker_side
//
//         if arranged_order_side == mango::matching::Side::Bid {
//             base_position = order_size;
//             quote_position = -order_size * order_price - (order_size * order_price * perp_market_info.maker_fee);
//         } else {
//             base_position = -order_size;
//             quote_position = order_size * order_price - (order_size * order_price * perp_market_info.taker_fee);
//         }
//
//         let perp_orders_map_key = format!("{}_{}", user_index, mint_index);
//
//         if let Some(x) = perp_orders_map.get_mut(&perp_orders_map_key) {
//
//             *x += 1;
//         } else {
//             perp_orders_map.insert(perp_orders_map_key.clone(), 0);
//         }
//     }
// }

fn get_net(
    mango_account: &MangoAccount,
    bank_cache: &RootBankCache,
    mint_index: usize
) -> I80F48 {
    if mango_account.deposits[mint_index].is_positive() {
        mango_account.deposits[mint_index].checked_mul(bank_cache.deposit_index).unwrap()
    } else if mango_account.borrows[mint_index].is_positive() {
        -mango_account.borrows[mint_index].checked_mul(bank_cache.borrow_index).unwrap()
    } else {
        ZERO_I80F48
    }
}

#[allow(dead_code)]
pub async fn assert_vault_net_deposit_diff(
    test: &mut MangoProgramTest,
    mango_group_cookie: &MangoGroupCookie,
    user_index: usize,
    mint_index: usize,
) {

  let mango_cache = mango_group_cookie.mango_cache;
  let root_bank_cache = mango_cache.root_bank_cache[mint_index];
  let (_root_bank_pk, root_bank) = test.with_root_bank(
      &mango_group_cookie.mango_group, mint_index).await;

  let net = get_net(
      &mango_group_cookie.mango_accounts[user_index].mango_account,
      &root_bank_cache,
      mint_index,
  );

  let mut vault_amount = ZERO_I80F48;
  for node_bank_pk in root_bank.node_banks {
      if node_bank_pk != Pubkey::default() {
          let node_bank = test.load_account::<NodeBank>(node_bank_pk).await;
          let balance = test.get_token_balance(node_bank.vault).await;
          vault_amount += I80F48::from_num(balance);
      }
  }

  println!("net: {}", net.to_string());
  println!("vault_amount: {}", vault_amount.to_string());

  assert!(net == vault_amount);

}
