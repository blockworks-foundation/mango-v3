use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use fixed::types::I80F48;
use fixed_macro::types::I80F48;
use safe_transmute::to_bytes;
use safe_transmute::transmute_one_to_bytes;
use safe_transmute::transmute_to_bytes;
use solana_program::clock::Epoch;
use solana_program_test::*;
use solana_sdk::account::AccountSharedData;
use solana_sdk::account::WritableAccount;

use crate::tokio::time::sleep;
use mango::state::{QUOTE_INDEX, ZERO_I80F48};
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;

mod program_test;

#[tokio::test]
async fn test_delegate() {
    // === Arrange ===
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 2, num_mints: 16 };
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let liqor_index: usize = 0;
    let liqee_index: usize = 1;
    let btc_index = 2;
    let eth_index = 3;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, btc_index, 50_000.0).await;
    mango_group_cookie.set_oracle(&mut test, eth_index, 2_500.0).await;

    // Deposit amounts
    let liqor_deposits = vec![
        (liqor_index, btc_index, 10.0),
        (liqor_index, eth_index, 100.0),
        (liqor_index, test.quote_index, 1_000_000.0),
    ];

    let mut liqee_account_override = mango_group_cookie.mango_accounts[liqee_index].mango_account;
    liqee_account_override.deposits[btc_index] = I80F48!(1_000_000);
    liqee_account_override.deposits[test.quote_index] = I80F48!(100_000_000_000);
    liqee_account_override.perp_accounts[btc_index].base_position = -20_000;
    liqee_account_override.perp_accounts[btc_index].quote_position = I80F48!(50_000);
    liqee_account_override.perp_accounts[eth_index].quote_position = I80F48!(-50_000);

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &liqor_deposits).await;

    let acc = WritableAccount::create(
        10_000_000,
        transmute_one_to_bytes(&liqee_account_override).to_vec(),
        test.mango_program_id,
        bool::default(),
        Epoch::default(),
    );
    // AccountSharedData::new_data(10_000_000, &liqee_account_override, &test.mango_program_id);
    test.context.set_account(&mango_group_cookie.mango_accounts[liqee_index].address, &acc);

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;
}
