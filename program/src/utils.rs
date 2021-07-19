use bytemuck::{bytes_of, cast_slice_mut, from_bytes_mut, Contiguous, Pod};

use crate::error::MangoResult;
use crate::matching::Side;
use fixed::types::I80F48;
use solana_program::account_info::AccountInfo;
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;
use std::cell::RefMut;
use std::cmp::min;
use std::mem::size_of;

pub fn gen_signer_seeds<'a>(nonce: &'a u64, acc_pk: &'a Pubkey) -> [&'a [u8]; 2] {
    [acc_pk.as_ref(), bytes_of(nonce)]
}

pub fn gen_signer_key(
    nonce: u64,
    acc_pk: &Pubkey,
    program_id: &Pubkey,
) -> Result<Pubkey, ProgramError> {
    let seeds = gen_signer_seeds(&nonce, acc_pk);
    Ok(Pubkey::create_program_address(&seeds, program_id)?)
}

pub fn create_signer_key_and_nonce(program_id: &Pubkey, acc_pk: &Pubkey) -> (Pubkey, u64) {
    for i in 0..=u64::MAX_VALUE {
        if let Ok(pk) = gen_signer_key(i, acc_pk, program_id) {
            return (pk, i);
        }
    }
    panic!("Could not generate signer key");
}

#[inline]
pub fn remove_slop_mut<T: Pod>(bytes: &mut [u8]) -> &mut [T] {
    let slop = bytes.len() % size_of::<T>();
    let new_len = bytes.len() - slop;
    cast_slice_mut(&mut bytes[..new_len])
}

pub fn strip_header_mut<'a, H: Pod, D: Pod>(
    account: &'a AccountInfo,
) -> MangoResult<(RefMut<'a, H>, RefMut<'a, [D]>)> {
    Ok(RefMut::map_split(account.try_borrow_mut_data()?, |data| {
        let (header_bytes, inner_bytes) = data.split_at_mut(size_of::<H>());
        (from_bytes_mut(header_bytes), remove_slop_mut(inner_bytes))
    }))
}

pub fn invert_side(side: Side) -> Side {
    if side == Side::Bid {
        Side::Ask
    } else {
        Side::Bid
    }
}

pub struct FI80F48(i128);
impl FI80F48 {
    fn from_fixed(x: I80F48) -> Self {
        FI80F48(x.to_bits())
    }

    fn mul(&self, x: Self) -> Self {
        Self(0)
    }

    fn add(&self, x: Self) -> Self {
        Self(0)
    }
}

pub fn fmul(a: i128, b: i128) -> i128 {
    let x = a.trailing_zeros();
    if x < 48 {
        let y = min(48 - x, b.trailing_zeros());

        if x + y < 48 {
            ((a >> x) * (b >> y)) >> (48 - x - y)
        } else {
            (a >> x) * (b >> y)
        }
    } else {
        (a >> 48) * b
    }
}

#[test]
fn test_fmul() {
    let b = I80F48::from_num(-100000.12312423534555);
    let a = I80F48::from_num(120002.23412341231);

    println!("{:?}", I80F48::from_bits(fmul(a.to_bits(), b.to_bits())));
    println!("{:?}", a * b);
}
