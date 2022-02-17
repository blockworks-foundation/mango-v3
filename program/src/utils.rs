use bytemuck::{bytes_of, cast_slice_mut, from_bytes_mut, Contiguous, Pod};

use crate::error::MangoResult;
use crate::matching::Side;
use crate::state::{RootBank, ONE_I80F48};
use fixed::types::I80F48;
use solana_program::account_info::AccountInfo;
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;
use std::cell::RefMut;
use std::mem::size_of;

use crate::state::{PerpAccount, PerpMarketCache};

use mango_logs::{mango_emit_stack, PerpBalanceLog};

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

/// Return (quote_free, quote_locked, base_free, base_locked) in I80F48
#[inline(always)]
pub fn split_open_orders(
    open_orders: &serum_dex::state::OpenOrders,
) -> (I80F48, I80F48, I80F48, I80F48) {
    (
        I80F48::from_num(open_orders.native_pc_free + open_orders.referrer_rebates_accrued),
        I80F48::from_num(open_orders.native_pc_total - open_orders.native_pc_free),
        I80F48::from_num(open_orders.native_coin_free),
        I80F48::from_num(open_orders.native_coin_total - open_orders.native_coin_free),
    )
}

/// exponentiate by squaring; send in 1 / base if you want neg
pub fn pow_i80f48(mut base: I80F48, mut exp: u8) -> I80F48 {
    let mut result = ONE_I80F48;
    loop {
        if exp & 1 == 1 {
            result = result.checked_mul(base).unwrap();
        }
        exp >>= 1;
        if exp == 0 {
            break result;
        }
        base = base.checked_mul(base).unwrap();
    }
}

/// Warning: This function needs 512+ bytes free on the stack
pub fn emit_perp_balances(
    mango_group: Pubkey,
    mango_account: Pubkey,
    market_index: u64,
    pa: &PerpAccount,
    perp_market_cache: &PerpMarketCache,
) {
    mango_emit_stack::<_, 256>(PerpBalanceLog {
        mango_group: mango_group,
        mango_account: mango_account,
        market_index: market_index,
        base_position: pa.base_position,
        quote_position: pa.quote_position.to_bits(),
        long_settled_funding: pa.long_settled_funding.to_bits(),
        short_settled_funding: pa.short_settled_funding.to_bits(),
        long_funding: perp_market_cache.long_funding.to_bits(),
        short_funding: perp_market_cache.short_funding.to_bits(),
    });
}

/// returns the current interest rate in APR for a given RootBank
#[inline(always)]
pub fn compute_interest_rate(root_bank: &RootBank, utilization: I80F48) -> I80F48 {
    interest_rate_curve_calculator(
        utilization,
        root_bank.optimal_util,
        root_bank.optimal_rate,
        root_bank.max_rate,
    )
}

/// returns a tuple of (deposit_rate, interest_rate) for a given RootBank
/// values are in APR
#[inline(always)]
pub fn compute_deposit_rate(root_bank: &RootBank, utilization: I80F48) -> Option<(I80F48, I80F48)> {
    let interest_rate = compute_interest_rate(root_bank, utilization);
    if let Some(deposit_rate) = interest_rate.checked_mul(utilization) {
        Some((deposit_rate, interest_rate))
    } else {
        None
    }
}

/// calcualtor function that can be used to compute an interest
/// rate based on the given parameters
#[inline(always)]
pub fn interest_rate_curve_calculator(
    utilization: I80F48,
    optimal_util: I80F48,
    optimal_rate: I80F48,
    max_rate: I80F48,
) -> I80F48 {
    if utilization > optimal_util {
        let extra_util = utilization - optimal_util;
        let slope = (max_rate - optimal_rate) / (ONE_I80F48 - optimal_util);
        optimal_rate + slope * extra_util
    } else {
        let slope = optimal_rate / optimal_util;
        slope * utilization
    }
}
