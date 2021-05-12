use std::cell::{Ref, RefMut};

use bytemuck::{
    cast_slice, cast_slice_mut, from_bytes, from_bytes_mut, try_from_bytes, try_from_bytes_mut,
    Pod, Zeroable,
};
use enumflags2::BitFlags;
use fixed::types::{I64F64, U64F64};
use fixed_macro::types::U64F64;
use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;

use crate::error::{check_assert, MerpsError, MerpsErrorCode, MerpsResult, SourceFileId};

// TODO: all unit numbers are just place holders. make decisions on each unit number
// TODO: add prop tests for nums
// TODO add GUI hoster fee discount

macro_rules! check {
    ($cond:expr, $err:expr) => {
        check_assert($cond, $err, line!(), SourceFileId::State)
    };
}

macro_rules! check_eq {
    ($x:expr, $y:expr, $err:expr) => {
        check_assert($x == $y, $err, line!(), SourceFileId::State)
    };
}

macro_rules! check_eq_default {
    ($x:expr, $y:expr) => {
        check_assert($x == $y, MerpsErrorCode::Default, line!(), SourceFileId::Processor)
    };
}

macro_rules! throw {
    () => {
        MerpsError::MerpsErrorCode {
            merps_error_code: MerpsErrorCode::Default,
            line: line!(),
            source_file_id: SourceFileId::State,
        }
    };
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
    };
}

pub const MAX_TOKENS: usize = 64;
pub const MAX_PAIRS: usize = MAX_TOKENS - 1;
pub const MAX_NODE_BANKS: usize = 8;
pub const ZERO_U64F64: U64F64 = U64F64!(0);

#[derive(Copy, Clone, BitFlags, Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum AccountFlag {
    Initialized = 1u64 << 0,
    MerpsGroup = 1u64 << 1,
    MerpsAccount = 1u64 << 2,
}

#[derive(Copy, Clone)]
#[repr(C)]
pub struct MerpsGroup {
    pub account_flags: u64, // TODO think about adding versioning here
    pub num_tokens: usize,
    pub num_markets: usize, // Note: does not increase if there is a spot and perp market for same base token

    pub tokens: [Pubkey; MAX_TOKENS],
    pub oracles: [Pubkey; MAX_PAIRS],
    // Note: oracle used for perps mark price is same as the one for spot. This is not ideal so it may change
    pub contract_sizes: [u128; MAX_PAIRS], // [10, ... 1]

    // Right now Serum dex spot markets. TODO make this general to an interface
    pub spot_markets: [Pubkey; MAX_PAIRS],

    // Pubkeys of different perpetuals markets
    pub perp_markets: [Pubkey; MAX_PAIRS],

    pub root_banks: [Pubkey; MAX_TOKENS],

    // TODO store risk params (collateral weighting, liability weighting, perp weighting, liq weighting (?))
    // TODO consider storing oracle prices here
    //      it makes this more single threaded if cranks are writing to merps group constantly with oracle prices
    pub last_updated: [u64; MAX_TOKENS], // this only exists for the test_multi_tx thing
    pub valid_interval: u8,
}
impl_loadable!(MerpsGroup);
impl MerpsGroup {
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MerpsResult<RefMut<'a, Self>> {
        // TODO
        Ok(Self::load_mut(account)?)
    }
    pub fn load_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MerpsResult<Ref<'a, Self>> {
        check_eq_default!(account.owner, program_id)?;

        let merps_group = Self::load(account)?;
        check_eq_default!(
            merps_group.account_flags,
            (AccountFlag::Initialized | AccountFlag::MerpsGroup).bits()
        )?;

        Ok(merps_group)
    }

    pub fn find_oracle_index(&self, oracle_pk: &Pubkey) -> Option<usize> {
        self.oracles.iter().position(|pk| pk == oracle_pk) // TODO profile and optimize
    }
    pub fn find_root_bank_index(&self, root_bank_pk: &Pubkey) -> Option<usize> {
        self.root_banks.iter().position(|pk| pk == root_bank_pk) // TODO profile and optimize
    }
}

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

impl RootBank {
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MerpsResult<RefMut<'a, Self>> {
        // TODO
        Ok(Self::load_mut(account)?)
    }
    pub fn load_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MerpsResult<Ref<'a, Self>> {
        // TODO
        Ok(Self::load(account)?)
    }
    pub fn find_node_bank_index(&self, node_bank_pk: &Pubkey) -> Option<usize> {
        self.node_banks.iter().position(|pk| pk == node_bank_pk)
    }
}

#[derive(Copy, Clone)]
#[repr(C)]
pub struct NodeBank {
    pub account_flags: u64,
    pub deposits: U64F64,
    pub borrows: U64F64,
    pub vault: Pubkey,
}
impl_loadable!(NodeBank);
impl NodeBank {
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MerpsResult<RefMut<'a, Self>> {
        // TODO
        Ok(Self::load_mut(account)?)
    }
    pub fn load_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MerpsResult<Ref<'a, Self>> {
        // TODO
        Ok(Self::load(account)?)
    }
    pub fn checked_add_borrow(&mut self, v: U64F64) -> MerpsResult<()> {
        Ok(self.borrows = self.borrows.checked_add(v).ok_or(throw!())?)
    }
    pub fn checked_sub_borrow(&mut self, v: U64F64) -> MerpsResult<()> {
        Ok(self.borrows = self.borrows.checked_sub(v).ok_or(throw!())?)
    }
    pub fn checked_add_deposit(&mut self, v: U64F64) -> MerpsResult<()> {
        Ok(self.deposits = self.deposits.checked_add(v).ok_or(throw!())?)
    }
    pub fn checked_sub_deposit(&mut self, v: U64F64) -> MerpsResult<()> {
        Ok(self.deposits = self.deposits.checked_sub(v).ok_or(throw!())?)
    }
}

#[derive(Copy, Clone)]
#[repr(C)]
pub struct PriceCache {
    pub price: U64F64,
    pub last_update: u64,
}
unsafe impl Zeroable for PriceCache {}
unsafe impl Pod for PriceCache {}

#[derive(Copy, Clone)]
#[repr(C)]
pub struct RootBankCache {
    pub deposit_index: U64F64,
    pub borrow_index: U64F64,
    pub last_update: u64,
}
unsafe impl Zeroable for RootBankCache {}
unsafe impl Pod for RootBankCache {}

#[derive(Copy, Clone)]
#[repr(C)]
pub struct OpenOrdersCache {
    pub base_total: u64,
    pub quote_total: u64,
    pub last_update: u64,
}
unsafe impl Zeroable for OpenOrdersCache {}
unsafe impl Pod for OpenOrdersCache {}

#[derive(Copy, Clone)]
#[repr(C)]
pub struct PerpMarketCache {
    pub funding_paid: u128,
    pub last_update: u64,
}
unsafe impl Zeroable for PerpMarketCache {}
unsafe impl Pod for PerpMarketCache {}

#[derive(Copy, Clone)]
#[repr(C)]
pub struct MerpsAccount {
    pub account_flags: u64,
    pub merps_group: Pubkey,
    pub owner: Pubkey,

    pub in_basket: [bool; MAX_PAIRS], // this can be done with u64 and bit shifting to save space
    pub price_cache: [PriceCache; MAX_PAIRS], // TODO consider only having enough space for those in basket
    pub root_bank_cache: [RootBankCache; MAX_TOKENS],
    pub open_orders_cache: [OpenOrdersCache; MAX_PAIRS],
    pub perp_market_cache: [PerpMarketCache; MAX_PAIRS],

    // Spot and Margin related data
    pub deposits: [U64F64; MAX_TOKENS],
    pub borrows: [U64F64; MAX_TOKENS],
    pub open_orders: [Pubkey; MAX_PAIRS],

    // Perps related data
    pub base_positions: [i128; MAX_PAIRS],
    pub quote_positions: [i128; MAX_PAIRS],

    pub funding_earned: [u128; MAX_PAIRS],
    pub funding_settled: [u128; MAX_PAIRS],
    // TODO hold perps open orders in here
}
impl_loadable!(MerpsAccount);

impl MerpsAccount {
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        merps_group_pk: &Pubkey,
    ) -> MerpsResult<RefMut<'a, Self>> {
        // load_mut checks for size already
        // merps account must be rent exempt to even be initialized
        check_eq!(account.owner, program_id, MerpsErrorCode::InvalidOwner)?;
        let merps_account = Self::load_mut(account)?;

        let valid_flags: u64 = (AccountFlag::Initialized | AccountFlag::MerpsAccount).bits();
        check_eq!(merps_account.account_flags, valid_flags, MerpsErrorCode::Default)?;
        check_eq!(&merps_account.merps_group, merps_group_pk, MerpsErrorCode::Default)?;

        Ok(merps_account)
    }
    pub fn checked_add_borrow(&mut self, token_i: usize, v: U64F64) -> MerpsResult<()> {
        Ok(self.borrows[token_i] = self.borrows[token_i].checked_add(v).ok_or(throw!())?)
    }
    pub fn checked_sub_borrow(&mut self, token_i: usize, v: U64F64) -> MerpsResult<()> {
        Ok(self.borrows[token_i] = self.borrows[token_i].checked_sub(v).ok_or(throw!())?)
    }
    pub fn checked_add_deposit(&mut self, token_i: usize, v: U64F64) -> MerpsResult<()> {
        Ok(self.deposits[token_i] = self.deposits[token_i].checked_add(v).ok_or(throw!())?)
    }
    pub fn checked_sub_deposit(&mut self, token_i: usize, v: U64F64) -> MerpsResult<()> {
        Ok(self.deposits[token_i] = self.deposits[token_i].checked_sub(v).ok_or(throw!())?)
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
    pub last_updated: u64, // admin key
                           // insurance fund?
}
impl_loadable!(PerpMarket);
