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

#[derive(Copy, Clone)]
pub struct FI80F48(i128);
impl FI80F48 {
    pub fn from_fixed(x: I80F48) -> Self {
        FI80F48(x.to_bits())
    }

    pub fn from_u64(x: u64) -> Self {
        Self((x << 48) as i128)
    }

    pub fn to_fixed(&self) -> I80F48 {
        I80F48::from_bits(self.0)
    }

    pub fn add(&self, x: Self) -> Self {
        Self(self.0 + x.0)
    }

    pub fn sub(&self, x: Self) -> Self {
        Self(self.0 - x.0)
    }

    pub fn mul(&self, x: Self) -> Self {
        let n = self.0.trailing_zeros();
        Self(if n < 48 {
            let m = min(48 - n, x.0.trailing_zeros());

            if n + m < 48 {
                let (r, over) = (self.0 >> n).overflowing_mul(x.0 >> m);
                if over {
                    let (ah, al) = self.split();
                    let (bh, bl) = x.split();
                    mul_hi_lo(ah, al, bh, bl)
                } else {
                    r >> (48 - m - n)
                }
            } else {
                (self.0 >> n) * (x.0 >> m)
            }
        } else {
            (self.0 >> 48) * x.0
        })
    }

    pub fn div(&self, x: Self) -> Self {
        Self((self.0 / x.0) << 48)
    }
    fn split(&self) -> (i128, i128) {
        (self.0 >> 64, 0xffffffffffffffffi128 & self.0)
    }
    pub fn is_positive(&self) -> bool {
        self.0.is_positive()
    }
    pub fn is_negative(&self) -> bool {
        self.0.is_negative()
    }
    pub fn min(&self, x: Self) -> Self {
        if self.0 < x.0 {
            *self
        } else {
            x
        }
    }
}

fn mul_hi_lo(ah: i128, al: i128, bh: i128, bl: i128) -> i128 {
    let ah_bh = (ah * bh).checked_shl(80).unwrap();
    let ah_bl = (ah * bl).checked_shl(16).unwrap();
    let al_bh = (al * bh).checked_shl(16).unwrap();
    let al_bl = (al * bl) >> 48;
    ah_bh.checked_add(ah_bl).unwrap().checked_add(al_bh).unwrap().checked_add(al_bl).unwrap()
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
    let b = I80F48::from_bits((1i128 << 64) + 1);
    let a = I80F48::from_bits((1i128 << 64) + 1);

    println!("{:?}", a * b);
    println!("{:?}", FI80F48::from_fixed(a).mul(FI80F48::from_fixed(b)).to_fixed());
}
