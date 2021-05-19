// Tests related to borrowing on a MerpsGroup
#![cfg(feature = "test-bpf")]

mod helpers;

use helpers::*;
use merps::{
    entrypoint::process_instruction,
    instruction::{borrow, deposit, init_merps_account},
    state::MerpsAccount,
    state::MerpsGroup,
};
use solana_program::account_info::AccountInfo;
use solana_program_test::*;
use solana_sdk::{
    account::Account,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use std::mem::size_of;

#[tokio::test]
async fn test_borrow_succeeds() {}

#[tokio::test]
async fn test_borrow_fails_overleveraged() {}
