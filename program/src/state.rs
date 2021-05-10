use std::cell::{Ref, RefMut};

use solana_program::account_info::AccountInfo;
use solana_program::program_error::ProgramError;
use bytemuck::{cast_slice, cast_slice_mut, from_bytes, from_bytes_mut, Pod, try_from_bytes, try_from_bytes_mut, Zeroable};
use solana_program::pubkey::Pubkey;
use fixed::types::{U64F64, I64F64};
use crate::error::MangoResult;
use solana_program::entrypoint::ProgramResult;

// TODO: all unit numbers are just place holders. make decisions on each unit number
macro_rules! check {
    ($cond:expr, $err:expr) => {
        check_assert($cond, $err, line!(), SourceFileId::State)
    }
}

macro_rules! check_eq {
    ($x:expr, $y:expr, $err:expr) => {
        check_assert($x == $y, $err, line!(), SourceFileId::State)
    }
}


pub trait Loadable: Pod {
    fn load_mut<'a>(account: &'a AccountInfo) -> Result<RefMut<'a, Self>, ProgramError> {
        // TODO verify if this checks for size
        Ok(RefMut::map(account.try_borrow_mut_data()?, |data| from_bytes_mut(data)))
    }
    fn load<'a>(account: &'a AccountInfo) -> Result<Ref<'a, Self>, ProgramError> {
        Ok(Ref::map(account.try_borrow_data()?, |data| from_bytes(data)))
    }

    fn load_from_bytes(data: &[u8]) -> Result<&Self, ProgramError> {
        Ok(from_bytes(data))
    }
}

macro_rules! impl_loadable {
    ($type_name:ident) => {
        unsafe impl Zeroable for $type_name {}
        unsafe impl Pod for $type_name {}
        impl Loadable for $type_name {}
    }
}


pub const MAX_TOKENS: usize = 8;
pub const MAX_PAIRS: usize = MAX_TOKENS - 1;
pub const MAX_NODE_BANKS: usize = 8;

#[derive(Copy, Clone)]
#[repr(C)]
pub struct MerpsGroup {
    pub account_flags: u64,  // TODO think about adding versioning here
    pub num_tokens: usize,
    pub num_spot_markets: usize,
    pub num_perp_markets: usize,

    pub tokens: [Pubkey; MAX_TOKENS],
    pub spot_oracles: [Pubkey; MAX_PAIRS],
    pub mark_oracles: [Pubkey; MAX_PAIRS],  // oracles for the perp mark price, might be same as spot oracle

    pub contract_sizes: [u128; MAX_PAIRS],  // [10, ... 1]

    // Right now Serum dex spot markets. TODO make this general to an interface
    pub spot_markets: [Pubkey; MAX_PAIRS],

    // Pubkeys of different perpetuals markets
    pub perp_markets: [Pubkey; MAX_PAIRS],

    pub root_banks: [Pubkey; MAX_TOKENS],

    // TODO store risk params (collateral weighting, liability weighting, perp weighting, liq weighting (?))
    // TODO consider storing oracle prices here
    //      it makes this more single threaded if cranks are writing to merps group constantly with oracle prices
    //


    pub last_updated: [u64; MAX_TOKENS]
}
impl_loadable!(MerpsGroup);


/// This is the root bank for one token's lending and borrowing info
#[derive(Copy, Clone)]
#[repr(C)]
pub struct RootBank {
    pub account_flags: u64,
    pub num_node_banks: usize,
    pub node_banks: [Pubkey; MAX_NODE_BANKS],
    pub deposit_index: U64F64,
    pub borrow_index: U64F64,
    pub last_updated: u64,
}
impl_loadable!(RootBank);


#[derive(Copy, Clone)]
#[repr(C)]
pub struct NodeBank {
    pub account_flags: u64,
    pub deposits: U64F64,
    pub borrows: U64F64,
    pub vault: Pubkey,
}
impl_loadable!(NodeBank);


#[derive(Copy, Clone)]
#[repr(C)]
pub struct MerpsAccount {
    pub account_flags: u64,
    pub merps_group: Pubkey,
    pub owner: Pubkey,

    pub deposits: [U64F64; MAX_TOKENS],
    pub borrows: [U64F64; MAX_TOKENS],

    // perp positions in base qty and quote qty
    pub base_positions: [i128; MAX_PAIRS],
    pub quote_positions: [i128; MAX_PAIRS],

    pub funding_settled: [u128; MAX_PAIRS],

    // TODO hold open orders in here as well
}
impl_loadable!(MerpsAccount);


impl MerpsAccount {

    pub fn get_account_value(

    ) {

    }

}


/// This will hold top level info about the perps market
/// Likely all perps transactions on a market will be locked on this one because this will be passed in as writable
#[derive(Copy, Clone)]
#[repr(C)]
pub struct PerpMarket {
    pub account_flags: u64,

    pub bids: Pubkey,
    pub asks: Pubkey,
    pub event_queue: Pubkey,
    pub matching_queue: Pubkey,
    pub funding_paid: u128,
    pub open_interest: u128,

    pub mark_price: u128,
    pub mark_oracle: u128,
    pub last_updated: u64
    // admin key
    // insurance fund?
}
impl_loadable!(PerpMarket);
