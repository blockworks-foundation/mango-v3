use std::collections::HashMap;
use std::time::Duration;

use fixed::types::I80F48;
use solana_program::program_option::COption;
use solana_program::pubkey::Pubkey;
use solana_program_test::processor;
use solana_program_test::*;
use solana_sdk::signature::Keypair;
use solana_sdk::signature::Signer;
use spl_token::state::{Account, AccountState};

use mango::instruction::flash_loan;
use mango::state::{QUOTE_INDEX, ZERO_I80F48};
use program_test::assertions::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use program_test::*;

use crate::tokio::time::sleep;

mod program_test;
#[tokio::test]
async fn test_flash_loan() {
    // === Arrange ===
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 2, num_mints: 2 };
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let user_index: usize = 0;
    let mint_index: usize = 1;
    let base_price: f64 = 10_000.0;

    // Deposit amounts
    let user_deposits = vec![(user_index, test.quote_index, base_price * 3.)];

    let (_, root_bank) = test.with_root_bank(&mango_group_cookie.mango_group, mint_index).await;
    let (_, node_bank) = test.with_node_bank(&root_bank, 0).await;

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &user_deposits).await;
    let balance_before = test.get_token_balance(node_bank.vault).await;

    // Step 2: Make flash loan
    flash_loan_scenario(&mut test, &mut mango_group_cookie, mint_index, 100_000_000).await.unwrap();
    let balance_after = test.get_token_balance(node_bank.vault).await;

    assert!(balance_before >= balance_after)
}
//
// #[tokio::test]
// async fn test_margin_trade() {
//     // === Arrange ===
//     let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 2, num_mints: 2 };
//     let mut test = MangoProgramTest::start_new(&config).await;
//
//     let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
//     mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;
//
//     // General parameters
//     let user_index: usize = 0;
//     let mint_index: usize = 1;
//     let base_price: f64 = 10_000.0;
//
//     // Deposit amounts
//     let user_deposits = vec![(user_index, test.quote_index, base_price * 3.)];
// }
