#![cfg(feature = "test-bpf")]
#![feature(link_llvm_intrinsics)]
extern "C" {
    #[link_name = "llvm.smul.fix.i128"]
    pub fn unsafe_fixmul(a: i128, b: i128, scale: i32) -> i128;
}
pub fn fixmul(a: i128, b: i128) -> i128 {
    unsafe { unsafe_fixmul(a, b, 48) }
}
use fixed::types::I80F48;
use mango::matching::{AnyNode, InnerNode, LeafNode};
use mango::state::{MangoAccount, MangoCache, ONE_I80F48};
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
async fn test_fixmul() {
    let y = I80F48::from_bits(fixmul(ONE_I80F48.to_bits(), ONE_I80F48.to_bits()));
    println!("{}", y);
}
