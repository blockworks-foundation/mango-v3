#![cfg(feature = "test-bpf")]
use fixed::types::I80F48;
use fixed_macro::types::I80F48;
use merps::matching::{AnyNode, InnerNode, LeafNode};
use merps::state::MerpsAccount;
use solana_program_test::tokio;
use std::mem::{align_of, size_of};

#[tokio::test]
async fn test_size() {
    println!("LeafNode: {} {}", size_of::<LeafNode>(), align_of::<LeafNode>());
    println!("InnerNode: {}", size_of::<InnerNode>());
    println!("AnyNode: {}", size_of::<AnyNode>());
    println!("MerpsAccount: {}", size_of::<MerpsAccount>());
}

#[tokio::test]
async fn test_i80f48() {
    let one: I80F48 = I80F48!(1.25);
    let neg_one: I80F48 = I80F48!(-1.25);
    println!("1.25 -> {:?} ", one.to_le_bytes());
    println!("-1.25 -> {:?} ", neg_one.to_le_bytes());
}
