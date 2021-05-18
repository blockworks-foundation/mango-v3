#![cfg(feature = "test-bpf")]
use bytemuck::Zeroable;
use merps::critbit::{AnyNode, InnerNode, LeafNode};
use solana_program_test::tokio;
use std::mem::{align_of, size_of};

#[tokio::test]
async fn test_size() {
    println!("LeafNode: {} {}", size_of::<LeafNode>(), align_of::<LeafNode>());
    println!("InnerNode: {}", size_of::<InnerNode>());
    println!("AnyNode: {}", size_of::<AnyNode>());
}
