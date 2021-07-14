#![cfg(feature = "test-bpf")]
use fixed::types::I80F48;
use fixed_macro::types::I80F48;
use mango::matching::{AnyNode, InnerNode, LeafNode};
use mango::state::{MangoAccount, MangoCache};
use solana_program_test::tokio;
use std::mem::{align_of, size_of};

#[tokio::test]
async fn test_size() {
    println!("LeafNode: {} {}", size_of::<LeafNode>(), align_of::<LeafNode>());
    println!("InnerNode: {}", size_of::<InnerNode>());
    println!("AnyNode: {}", size_of::<AnyNode>());
    println!("MangoAccount: {}", size_of::<MangoAccount>());
    println!("MangoCache: {}", size_of::<MangoCache>());
}

#[tokio::test]
async fn test_i80f48() {
    let hundred = I80F48::from_num(100);
    let million = I80F48::from_num(1_000_000);
    let r: I80F48 = hundred / million;
    println!("{:#0128b}", r.to_bits())
}

#[tokio::test]
async fn test_bit_shift() {
    let n = 126;
    println!("{}", 1i128 << (127 - n) != (1i128 << 127) >> n);
}
