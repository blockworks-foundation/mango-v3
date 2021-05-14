use std::cell::{Ref, RefMut};

use bytemuck::{
    cast_slice, cast_slice_mut, from_bytes, from_bytes_mut, try_from_bytes, try_from_bytes_mut,
    Pod, Zeroable,
};
use enumflags2::BitFlags;
use fixed::types::{I64F64, I80F48, U64F64};
use fixed_macro::types::{I80F48, U64F64};
use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;

use crate::error::{check_assert, MerpsError, MerpsErrorCode, MerpsResult, SourceFileId};

pub const MAX_TOKENS: usize = 64;
pub const MAX_PAIRS: usize = MAX_TOKENS - 1;
pub const MAX_NODE_BANKS: usize = 8;
pub const ZERO_U64F64: U64F64 = U64F64!(0);
pub const ZERO_I80F48: I80F48 = I80F48!(0);
pub const ONE_I80F48: I80F48 = I80F48!(1);

declare_check_assert_macros!(SourceFileId::State);

// TODO: all unit numbers are just place holders. make decisions on each unit number
// TODO: add prop tests for nums
// TODO add GUI hoster fee discount

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

#[repr(u8)]
pub enum DataType {
    MerpsGroup = 0,
    MerpsAccount,
    RootBank,
    NodeBank,
    PerpMarket,
}

#[derive(Copy, Clone)]
#[repr(C)]
pub struct MerpsGroup {
    pub data_type: u8,
    pub version: u8,
    pub is_initialized: bool,
    pub padding: [u8; 5],

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

    pub asset_weights: [I80F48; MAX_TOKENS],

    pub signer_nonce: u64,
    // TODO determine liquidation incentives for each token
    // TODO determine maint weight and init weight

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
        check_eq!(account.owner, program_id, MerpsErrorCode::InvalidOwner)?;

        let merps_group = Self::load(account)?;
        check!(merps_group.is_initialized, MerpsErrorCode::Default)?;
        check_eq!(merps_group.data_type, DataType::MerpsGroup as u8, MerpsErrorCode::Default)?;

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
    pub data_type: u8,
    pub version: u8,
    pub is_initialized: bool,
    pub padding: [u8; 5],

    pub account_flags: u64,
    pub num_node_banks: usize,
    pub node_banks: [Pubkey; MAX_NODE_BANKS],
    pub deposit_index: I80F48,
    pub borrow_index: I80F48,
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
    pub data_type: u8,
    pub version: u8,
    pub is_initialized: bool,
    pub padding: [u8; 5],
    pub deposits: I80F48,
    pub borrows: I80F48,
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
    pub fn checked_add_borrow(&mut self, v: I80F48) -> MerpsResult<()> {
        Ok(self.borrows = self.borrows.checked_add(v).ok_or(throw!())?)
    }
    pub fn checked_sub_borrow(&mut self, v: I80F48) -> MerpsResult<()> {
        Ok(self.borrows = self.borrows.checked_sub(v).ok_or(throw!())?)
    }
    pub fn checked_add_deposit(&mut self, v: I80F48) -> MerpsResult<()> {
        Ok(self.deposits = self.deposits.checked_add(v).ok_or(throw!())?)
    }
    pub fn checked_sub_deposit(&mut self, v: I80F48) -> MerpsResult<()> {
        Ok(self.deposits = self.deposits.checked_sub(v).ok_or(throw!())?)
    }
}

#[derive(Copy, Clone)]
#[repr(C)]
pub struct PriceCache {
    pub price: I80F48,
    pub last_update: u64,
}
unsafe impl Zeroable for PriceCache {}
unsafe impl Pod for PriceCache {}

#[derive(Copy, Clone)]
#[repr(C)]
pub struct RootBankCache {
    pub deposit_index: I80F48,
    pub borrow_index: I80F48,
    pub last_update: u64,
}
unsafe impl Zeroable for RootBankCache {}
unsafe impl Pod for RootBankCache {}

#[derive(Copy, Clone)]
#[repr(C)]
pub struct OpenOrdersCache {
    pub base_total: I80F48,
    pub quote_total: I80F48,
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
    pub data_type: u8,
    pub version: u8,
    pub is_initialized: bool,
    pub padding: [u8; 5],

    pub merps_group: Pubkey,
    pub owner: Pubkey,

    pub in_basket: [bool; MAX_PAIRS], // this can be done with u64 and bit shifting to save space
    pub price_cache: [PriceCache; MAX_PAIRS], // TODO consider only having enough space for those in basket
    pub root_bank_cache: [RootBankCache; MAX_TOKENS],
    pub open_orders_cache: [OpenOrdersCache; MAX_PAIRS],
    pub perp_market_cache: [PerpMarketCache; MAX_PAIRS],

    // Spot and Margin related data
    pub deposits: [I80F48; MAX_TOKENS],
    pub borrows: [I80F48; MAX_TOKENS],
    pub open_orders: [Pubkey; MAX_PAIRS],

    // Perps related data
    pub base_positions: [I80F48; MAX_PAIRS],
    pub quote_positions: [I80F48; MAX_PAIRS],

    pub funding_earned: [I80F48; MAX_PAIRS],
    pub funding_settled: [I80F48; MAX_PAIRS],
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

        check_eq!(merps_account.data_type, DataType::MerpsAccount as u8, MerpsErrorCode::Default)?;
        check!(merps_account.is_initialized, MerpsErrorCode::Default)?;
        check_eq!(&merps_account.merps_group, merps_group_pk, MerpsErrorCode::Default)?;

        Ok(merps_account)
    }
    pub fn checked_add_borrow(&mut self, token_i: usize, v: I80F48) -> MerpsResult<()> {
        Ok(self.borrows[token_i] = self.borrows[token_i].checked_add(v).ok_or(throw!())?)
    }
    pub fn checked_sub_borrow(&mut self, token_i: usize, v: I80F48) -> MerpsResult<()> {
        Ok(self.borrows[token_i] = self.borrows[token_i].checked_sub(v).ok_or(throw!())?)
    }
    pub fn checked_add_deposit(&mut self, token_i: usize, v: I80F48) -> MerpsResult<()> {
        Ok(self.deposits[token_i] = self.deposits[token_i].checked_add(v).ok_or(throw!())?)
    }
    pub fn checked_sub_deposit(&mut self, token_i: usize, v: I80F48) -> MerpsResult<()> {
        Ok(self.deposits[token_i] = self.deposits[token_i].checked_sub(v).ok_or(throw!())?)
    }

    pub fn check_caches_valid(&self, merps_group: &MerpsGroup, now_ts: u64) -> bool {
        let valid_interval = merps_group.valid_interval as u64;
        if now_ts > self.root_bank_cache[MAX_TOKENS - 1].last_update + valid_interval {
            return false;
        }

        for i in 0..merps_group.num_markets {
            // If this asset is not in user basket, then there are no deposits, borrows or perp positions to calculate value of
            if !self.in_basket[i] {
                continue;
            }
            if now_ts > self.price_cache[i].last_update + valid_interval {
                return false;
            }
            if now_ts > self.root_bank_cache[i].last_update + valid_interval {
                return false;
            }
            if self.open_orders[i] != Pubkey::default() {
                if now_ts > self.open_orders_cache[i].last_update + valid_interval {
                    return false;
                }
            }
            if merps_group.perp_markets[i] != Pubkey::default() {
                if now_ts > self.perp_market_cache[i].last_update + valid_interval {
                    return false;
                }
            }
        }

        true
    }

    // TODO need a new name for this as it's not exactly collateral ratio
    pub fn get_coll_ratio(&self, merps_group: &MerpsGroup) -> MerpsResult<I80F48> {
        // Value of all assets and liabs in quote currency
        let quote_i = MAX_TOKENS - 1;
        let mut assets_val = self.root_bank_cache[quote_i]
            .deposit_index
            .checked_mul(self.deposits[quote_i])
            .ok_or(throw_err!(MerpsErrorCode::MathError))?;

        let mut liabs_val = self.root_bank_cache[quote_i]
            .borrow_index
            .checked_mul(self.borrows[quote_i])
            .ok_or(throw_err!(MerpsErrorCode::MathError))?;

        for i in 0..merps_group.num_markets {
            // If this asset is not in user basket, then there are no deposits, borrows or perp positions to calculate value of
            if !self.in_basket[i] {
                continue;
            }
            let price_cache = &self.price_cache[i];
            let root_bank_cache = &self.root_bank_cache[i];
            let open_orders_cache = &self.open_orders_cache[i];

            let mut base_assets = root_bank_cache
                .deposit_index
                .checked_mul(self.deposits[i])
                .ok_or(throw_err!(MerpsErrorCode::MathError))?;

            let mut base_liabs = root_bank_cache
                .borrow_index
                .checked_mul(self.borrows[i])
                .ok_or(throw_err!(MerpsErrorCode::MathError))?;

            if self.open_orders[i] != Pubkey::default() {
                assets_val = open_orders_cache
                    .quote_total
                    .checked_add(assets_val)
                    .ok_or(throw_err!(MerpsErrorCode::MathError))?;

                base_assets = open_orders_cache
                    .base_total
                    .checked_add(base_assets)
                    .ok_or(throw_err!(MerpsErrorCode::MathError))?;
            }

            if merps_group.perp_markets[i] != Pubkey::default() {
                // TODO fill this in once perp logic is a little bit more clear
            }

            let asset_weight = merps_group.asset_weights[i];
            let liab_weight = ONE_I80F48 / asset_weight;
            assets_val = base_assets
                .checked_mul(price_cache.price)
                .ok_or(throw_err!(MerpsErrorCode::MathError))?
                .checked_mul(asset_weight)
                .ok_or(throw_err!(MerpsErrorCode::MathError))?
                .checked_add(assets_val)
                .ok_or(throw_err!(MerpsErrorCode::MathError))?;

            liabs_val = base_liabs
                .checked_mul(price_cache.price)
                .ok_or(throw_err!(MerpsErrorCode::MathError))?
                .checked_mul(liab_weight)
                .ok_or(throw_err!(MerpsErrorCode::MathError))?
                .checked_add(liabs_val)
                .ok_or(throw_err!(MerpsErrorCode::MathError))?;
        }

        if liabs_val == ZERO_I80F48 {
            Ok(I80F48::MAX)
        } else {
            assets_val.checked_div(liabs_val).ok_or(throw!())
        }
    }
}

/// This will hold top level info about the perps market
/// Likely all perps transactions on a market will be locked on this one because this will be passed in as writable
#[derive(Copy, Clone)]
#[repr(C)]
pub struct PerpMarket {
    pub data_type: u8,
    pub version: u8,
    pub is_initialized: bool,
    pub padding: [u8; 5],

    pub bids: Pubkey,
    pub asks: Pubkey,
    pub event_queue: Pubkey,
    pub matching_queue: Pubkey,
    pub funding_paid: I80F48,
    pub open_interest: I80F48,

    pub mark_price: I80F48,
    pub index_oracle: Pubkey,
    pub last_updated: u64,
    // mark_price = used to liquidate and calculate value of positions; function of index and some moving average of basis
    // index_price = some function of centralized exchange spot prices
    // book_price = average of impact bid and impact ask; used to calculate basis
    // basis = book_price / index_price - 1; some moving average of this is used for mark price
}
impl_loadable!(PerpMarket);
