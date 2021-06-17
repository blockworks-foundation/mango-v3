use std::cell::{Ref, RefMut};
use std::mem::size_of;
use std::num::NonZeroU64;

use bytemuck::{from_bytes, from_bytes_mut};
use fixed::types::I80F48;
use fixed_macro::types::I80F48;
use solana_program::account_info::AccountInfo;
use solana_program::msg;
use solana_program::pubkey::Pubkey;

use crate::error::{check_assert, MerpsError, MerpsErrorCode, MerpsResult, SourceFileId};
use crate::matching::{Book, LeafNode, Side};

use mango_common::Loadable;
use mango_macro::{Loadable, Pod};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use serum_dex::state::ToAlignedBytes;
use solana_program::program_error::ProgramError;
use solana_program::sysvar::{clock::Clock, rent::Rent, Sysvar};
use std::convert::identity;
pub const MAX_TOKENS: usize = 32;
pub const MAX_PAIRS: usize = MAX_TOKENS - 1;
pub const MAX_NODE_BANKS: usize = 8;
pub const QUOTE_INDEX: usize = MAX_TOKENS - 1;
pub const ZERO_I80F48: I80F48 = I80F48!(0);
pub const ONE_I80F48: I80F48 = I80F48!(1);
pub const DAY: I80F48 = I80F48!(86400);

const OPTIMAL_UTIL: I80F48 = I80F48!(0.7);
const OPTIMAL_R: I80F48 = I80F48!(6.3419583967529173008625e-09); // 20% APY -> 0.1 / YEAR
const MAX_R: I80F48 = I80F48!(9.5129375951293759512937e-08); // max 300% APY -> 1 / YEAR

declare_check_assert_macros!(SourceFileId::State);

// TODO: all unit numbers are just place holders. make decisions on each unit number
// TODO: add prop tests for nums
// TODO add GUI hoster fee discount
// TODO double check all the

// units
// long_funding: I80F48 - native quote currency per contract
// short_funding: I80F48 - native quote currency per contract
// long_funding_settled: I80F48 - native quote currency per contract
// short_funding_settled: I80F48 - native quote currency per contract
// base_positions: i64 - number of contracts
// quote_positions: I80F48 - native quote currency
// price: I80F48 - native quote per native base
// price: i64 - quote lots per base lot

#[repr(u8)]
#[derive(IntoPrimitive, TryFromPrimitive)]
pub enum DataType {
    MerpsGroup = 0,
    MerpsAccount,
    RootBank,
    NodeBank,
    PerpMarket,
    Bids,
    Asks,
    MerpsCache,
    EventQueue,
}

#[derive(Copy, Clone, Pod, Default)]
#[repr(C)]
pub struct MetaData {
    pub data_type: u8,
    pub version: u8,
    pub is_initialized: bool,
    pub padding: [u8; 5], // This makes explicit the 8 byte alignment padding
}

impl MetaData {
    pub fn new(data_type: DataType, version: u8, is_initialized: bool) -> Self {
        Self { data_type: data_type as u8, version, is_initialized, padding: [0u8; 5] }
    }
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct TokenInfo {
    pub mint: Pubkey,
    pub root_bank: Pubkey,
    pub decimals: u8,
    pub padding: [u8; 7],
}

impl TokenInfo {
    pub fn is_empty(&self) -> bool {
        self.mint == Pubkey::default()
    }
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct SpotMarketInfo {
    pub spot_market: Pubkey,
    pub maint_asset_weight: I80F48,
    pub init_asset_weight: I80F48,
    pub maint_liab_weight: I80F48,
    pub init_liab_weight: I80F48,
}

impl SpotMarketInfo {
    pub fn is_empty(&self) -> bool {
        self.spot_market == Pubkey::default()
    }
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct PerpMarketInfo {
    pub perp_market: Pubkey, // One of these may be empty
    pub maint_asset_weight: I80F48,
    pub init_asset_weight: I80F48,
    pub maint_liab_weight: I80F48,
    pub init_liab_weight: I80F48,
    pub liquidation_fee: I80F48,
    pub base_lot_size: i64,  // The lot size of the underlying
    pub quote_lot_size: i64, // min tick
}

impl PerpMarketInfo {
    pub fn is_empty(&self) -> bool {
        self.perp_market == Pubkey::default()
    }
}

#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct MerpsGroup {
    pub meta_data: MetaData,
    pub num_oracles: usize, // incremented every time add_oracle is called

    pub tokens: [TokenInfo; MAX_TOKENS],
    pub spot_markets: [SpotMarketInfo; MAX_PAIRS],
    pub perp_markets: [PerpMarketInfo; MAX_PAIRS],

    pub oracles: [Pubkey; MAX_PAIRS],

    pub signer_nonce: u64,
    pub signer_key: Pubkey,
    pub admin: Pubkey,          // Used to add new markets and adjust risk params
    pub dex_program_id: Pubkey, // Consider allowing more
    pub merps_cache: Pubkey,
    // TODO determine liquidation incentives for each token
    // TODO store risk params (collateral weighting, liability weighting, perp weighting, liq weighting (?))
    pub valid_interval: u64,
}

impl MerpsGroup {
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MerpsResult<RefMut<'a, Self>> {
        check_eq!(account.data_len(), size_of::<Self>(), MerpsErrorCode::Default)?;
        check_eq!(account.owner, program_id, MerpsErrorCode::InvalidOwner)?;

        let merps_group = Self::load_mut(account)?;
        check_eq!(
            merps_group.meta_data.data_type,
            DataType::MerpsGroup as u8,
            MerpsErrorCode::Default
        )?;

        Ok(merps_group)
    }
    pub fn load_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MerpsResult<Ref<'a, Self>> {
        check_eq!(account.owner, program_id, MerpsErrorCode::InvalidOwner)?;

        let merps_group = Self::load(account)?;
        check!(merps_group.meta_data.is_initialized, MerpsErrorCode::Default)?;
        check_eq!(
            merps_group.meta_data.data_type,
            DataType::MerpsGroup as u8,
            MerpsErrorCode::Default
        )?;

        Ok(merps_group)
    }

    pub fn find_oracle_index(&self, oracle_pk: &Pubkey) -> Option<usize> {
        self.oracles.iter().position(|pk| pk == oracle_pk) // TODO profile and optimize
    }
    pub fn find_root_bank_index(&self, root_bank_pk: &Pubkey) -> Option<usize> {
        // TODO profile and optimize
        self.tokens.iter().position(|token_info| &token_info.root_bank == root_bank_pk)
    }
    pub fn find_spot_market_index(&self, spot_market_pk: &Pubkey) -> Option<usize> {
        self.spot_markets
            .iter()
            .position(|spot_market_info| &spot_market_info.spot_market == spot_market_pk)
    }
    pub fn find_perp_market_index(&self, perp_market_pk: &Pubkey) -> Option<usize> {
        self.perp_markets
            .iter()
            .position(|perp_market_info| &perp_market_info.perp_market == perp_market_pk)
    }
}

/// This is the root bank for one token's lending and borrowing info
#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct RootBank {
    pub meta_data: MetaData,

    pub num_node_banks: usize,
    pub node_banks: [Pubkey; MAX_NODE_BANKS],
    pub deposit_index: I80F48,
    pub borrow_index: I80F48,
    pub last_updated: u64,
}

impl RootBank {
    pub fn load_and_init<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        node_bank_ai: &'a AccountInfo,

        rent: &Rent,
    ) -> MerpsResult<RefMut<'a, Self>> {
        let mut root_bank = Self::load_mut(account)?;
        check_eq!(account.owner, program_id, MerpsErrorCode::InvalidOwner)?;
        check!(
            rent.is_exempt(account.lamports(), size_of::<Self>()),
            MerpsErrorCode::AccountNotRentExempt
        )?;
        check!(!root_bank.meta_data.is_initialized, MerpsErrorCode::Default)?;

        root_bank.meta_data = MetaData::new(DataType::RootBank, 0, true);
        root_bank.node_banks[0] = *node_bank_ai.key;
        root_bank.num_node_banks = 1;
        root_bank.deposit_index = ONE_I80F48;
        root_bank.borrow_index = ONE_I80F48;

        Ok(root_bank)
    }
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MerpsResult<RefMut<'a, Self>> {
        check_eq!(account.data_len(), size_of::<Self>(), MerpsErrorCode::Default)?;
        check_eq!(account.owner, program_id, MerpsErrorCode::Default)?;

        let root_bank = Self::load_mut(account)?;

        check!(root_bank.meta_data.is_initialized, MerpsErrorCode::Default)?;
        check_eq!(
            root_bank.meta_data.data_type,
            DataType::RootBank as u8,
            MerpsErrorCode::Default
        )?;

        Ok(root_bank)
    }
    pub fn load_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MerpsResult<Ref<'a, Self>> {
        check_eq!(account.data_len(), size_of::<Self>(), MerpsErrorCode::Default)?;
        check_eq!(account.owner, program_id, MerpsErrorCode::InvalidOwner)?;

        let root_bank = Self::load(account)?;

        check!(root_bank.meta_data.is_initialized, MerpsErrorCode::Default)?;
        check_eq!(
            root_bank.meta_data.data_type,
            DataType::RootBank as u8,
            MerpsErrorCode::Default
        )?;

        Ok(root_bank)
    }
    pub fn find_node_bank_index(&self, node_bank_pk: &Pubkey) -> Option<usize> {
        self.node_banks.iter().position(|pk| pk == node_bank_pk)
    }

    pub fn update_index(
        &mut self,
        node_bank_ais: &[AccountInfo],
        program_id: &Pubkey,
    ) -> MerpsResult<()> {
        let clock = Clock::get()?;
        let curr_ts = clock.unix_timestamp as u64;
        let mut native_deposits = ZERO_I80F48;
        let mut native_borrows = ZERO_I80F48;

        for node_bank_ai in node_bank_ais.iter() {
            let node_bank = NodeBank::load_checked(node_bank_ai, program_id)?;
            native_deposits = native_deposits
                .checked_add(node_bank.deposits.checked_mul(self.deposit_index).unwrap())
                .unwrap();
            native_borrows = native_borrows
                .checked_add(node_bank.borrows.checked_mul(self.borrow_index).unwrap())
                .unwrap();
        }

        // TODO - is this a good assumption?
        let utilization = native_borrows.checked_div(native_deposits).unwrap_or(ZERO_I80F48);

        // Calculate interest rate
        // TODO: Review interest rate calculation
        let interest_rate = if utilization > OPTIMAL_UTIL {
            let extra_util = utilization - OPTIMAL_UTIL;
            let slope = (MAX_R - OPTIMAL_R) / (ONE_I80F48 - OPTIMAL_UTIL);
            OPTIMAL_R + slope * extra_util
        } else {
            let slope = OPTIMAL_R / OPTIMAL_UTIL;
            slope * utilization
        };

        let borrow_interest =
            interest_rate.checked_mul(I80F48::from_num(curr_ts - self.last_updated)).unwrap();
        let deposit_interest = borrow_interest.checked_mul(utilization).unwrap();

        self.last_updated = curr_ts;
        self.borrow_index = self
            .borrow_index
            .checked_mul(borrow_interest)
            .unwrap()
            .checked_add(self.borrow_index)
            .unwrap();
        self.deposit_index = self
            .deposit_index
            .checked_mul(deposit_interest)
            .unwrap()
            .checked_add(self.deposit_index)
            .unwrap();

        Ok(())
    }
}

#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct NodeBank {
    pub meta_data: MetaData,

    pub deposits: I80F48,
    pub borrows: I80F48,
    pub vault: Pubkey,
}

impl NodeBank {
    pub fn load_and_init<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        vault_ai: &'a AccountInfo,

        rent: &Rent,
    ) -> MerpsResult<RefMut<'a, Self>> {
        let mut node_bank = Self::load_mut(account)?;
        check_eq!(account.owner, program_id, MerpsErrorCode::InvalidOwner)?;
        check!(
            rent.is_exempt(account.lamports(), size_of::<Self>()),
            MerpsErrorCode::AccountNotRentExempt
        )?;
        check!(!node_bank.meta_data.is_initialized, MerpsErrorCode::Default)?;

        node_bank.meta_data = MetaData::new(DataType::NodeBank, 0, true);
        node_bank.deposits = ZERO_I80F48;
        node_bank.borrows = ZERO_I80F48;
        node_bank.vault = *vault_ai.key;

        Ok(node_bank)
    }
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MerpsResult<RefMut<'a, Self>> {
        check_eq!(account.owner, program_id, MerpsErrorCode::InvalidOwner)?;

        // TODO verify if size check necessary. We know load_mut fails if account size is too small for struct,
        //  does it also fail if it's too big?
        check_eq!(account.data_len(), size_of::<Self>(), MerpsErrorCode::Default)?;
        let node_bank = Self::load_mut(account)?;

        check!(node_bank.meta_data.is_initialized, MerpsErrorCode::Default)?;
        check_eq!(
            node_bank.meta_data.data_type,
            DataType::NodeBank as u8,
            MerpsErrorCode::Default
        )?;

        Ok(node_bank)
    }
    #[allow(unused)]
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
    pub fn has_valid_deposits_borrows(&self, root_bank: &RootBank) -> bool {
        self.get_total_native_deposit(root_bank) >= self.get_total_native_borrow(root_bank)
    }
    pub fn get_total_native_borrow(&self, root_bank: &RootBank) -> u64 {
        let native: I80F48 = self.borrows * root_bank.borrow_index;
        native.checked_ceil().unwrap().to_num() // rounds toward +inf
    }
    pub fn get_total_native_deposit(&self, root_bank: &RootBank) -> u64 {
        let native: I80F48 = self.deposits * root_bank.deposit_index;
        native.checked_floor().unwrap().to_num() // rounds toward -inf
    }
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct PriceCache {
    pub price: I80F48, // unit is interpreted as how many quote native tokens for 1 base native token
    pub last_update: u64,
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct RootBankCache {
    pub deposit_index: I80F48,
    pub borrow_index: I80F48,
    pub last_update: u64,
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct PerpMarketCache {
    pub long_funding: I80F48,
    pub short_funding: I80F48,
    pub last_update: u64,
}

#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct MerpsCache {
    pub meta_data: MetaData,

    pub price_cache: [PriceCache; MAX_PAIRS],
    pub root_bank_cache: [RootBankCache; MAX_TOKENS],
    pub perp_market_cache: [PerpMarketCache; MAX_PAIRS],
}

impl MerpsCache {
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        merps_group: &MerpsGroup,
    ) -> MerpsResult<RefMut<'a, Self>> {
        // merps account must be rent exempt to even be initialized
        check_eq!(account.owner, program_id, MerpsErrorCode::InvalidOwner)?;
        let merps_cache = Self::load_mut(account)?;

        check_eq!(
            merps_cache.meta_data.data_type,
            DataType::MerpsCache as u8,
            MerpsErrorCode::Default
        )?;
        check!(merps_cache.meta_data.is_initialized, MerpsErrorCode::Default)?;
        check_eq!(&merps_group.merps_cache, account.key, MerpsErrorCode::Default)?;

        Ok(merps_cache)
    }

    pub fn load_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        merps_group: &MerpsGroup,
    ) -> MerpsResult<Ref<'a, Self>> {
        check_eq!(account.data_len(), size_of::<Self>(), MerpsErrorCode::Default)?;
        check_eq!(account.owner, program_id, MerpsErrorCode::InvalidOwner)?;

        let merps_cache = Self::load(account)?;

        check_eq!(
            merps_cache.meta_data.data_type,
            DataType::MerpsCache as u8,
            MerpsErrorCode::Default
        )?;
        check!(merps_cache.meta_data.is_initialized, MerpsErrorCode::Default)?;
        check_eq!(&merps_group.merps_cache, account.key, MerpsErrorCode::Default)?;

        Ok(merps_cache)
    }

    pub fn check_caches_valid(
        &self,
        merps_group: &MerpsGroup,
        merps_account: &MerpsAccount,
        now_ts: u64,
    ) -> bool {
        let valid_interval = merps_group.valid_interval;
        if now_ts > self.root_bank_cache[QUOTE_INDEX].last_update + valid_interval {
            return false;
        }

        for i in 0..merps_group.num_oracles {
            // If this asset is not in user basket, then there are no deposits, borrows or perp positions to calculate value of
            if !merps_account.in_basket[i] {
                continue;
            }

            if now_ts > self.price_cache[i].last_update + valid_interval {
                return false;
            }

            if !merps_group.spot_markets[i].is_empty() {
                if now_ts > self.root_bank_cache[i].last_update + valid_interval {
                    return false;
                }
            }

            if !merps_group.perp_markets[i].is_empty() {
                if now_ts > self.perp_market_cache[i].last_update + valid_interval {
                    return false;
                }
            }
        }

        true
    }
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct PerpOpenOrders {
    pub bids_quantity: i64, // total contracts in sell orders
    pub asks_quantity: i64, // total quote currency in buy orders
    pub is_free_bits: u32,
    pub is_bid_bits: u32,
    pub orders: [i128; 32],
    pub client_order_ids: [u64; 32],
}

impl PerpOpenOrders {
    pub fn next_order_slot(self) -> u8 {
        self.is_free_bits.trailing_zeros() as u8
    }

    pub fn remove_order(&mut self, side: Side, slot: u8, quantity: i64) -> MerpsResult<()> {
        let slot_mask = 1u32 << slot;
        check_eq!(Some(side), self.slot_side(slot), MerpsErrorCode::Default)?;

        // accounting
        match side {
            Side::Bid => {
                self.bids_quantity -= quantity;
            }
            Side::Ask => {
                self.asks_quantity -= quantity;
            }
        }

        // release space
        self.is_free_bits |= slot_mask;
        self.orders[slot as usize] = 0i128;
        self.client_order_ids[slot as usize] = 0u64;
        Ok(())
    }
    pub fn add_order(&mut self, side: Side, order: &LeafNode) -> MerpsResult<()> {
        check!(self.is_free_bits != 0, MerpsErrorCode::TooManyOpenOrders)?;
        let slot = self.next_order_slot();
        let slot_mask = 1u32 << slot;
        self.is_free_bits &= !slot_mask;
        match side {
            Side::Bid => {
                // TODO make checked
                self.is_bid_bits |= slot_mask;
                self.bids_quantity += order.quantity;
            }
            Side::Ask => {
                self.is_bid_bits &= !slot_mask;
                self.asks_quantity += order.quantity;
            }
        };

        self.orders[slot as usize] = order.key;
        self.client_order_ids[slot as usize] = order.client_order_id;
        Ok(())
    }

    pub fn cancel_order(
        &mut self,
        order: &LeafNode,
        order_id: i128,
        side: Side,
    ) -> MerpsResult<()> {
        // input verification
        let slot = order.owner_slot;
        let slot_mask = 1u32 << slot;
        check_eq!(0u32, slot_mask & self.is_free_bits, MerpsErrorCode::Default)?;
        check_eq!(Some(side), self.slot_side(slot), MerpsErrorCode::Default)?;
        check_eq!(order_id, self.orders[slot as usize], MerpsErrorCode::Default)?;

        // accounting
        match side {
            Side::Bid => {
                self.bids_quantity -= order.quantity;
            }
            Side::Ask => {
                self.asks_quantity -= order.quantity;
            }
        }

        // release space
        self.is_free_bits |= slot_mask;
        self.orders[slot as usize] = 0i128;
        self.client_order_ids[slot as usize] = 0u64;

        Ok(())
    }

    #[inline]
    fn iter_filled_slots(&self) -> impl Iterator<Item = u8> {
        struct Iter {
            bits: u32,
        }
        impl Iterator for Iter {
            type Item = u8;
            #[inline(always)]
            fn next(&mut self) -> Option<Self::Item> {
                if self.bits == 0 {
                    None
                } else {
                    let next = self.bits.trailing_zeros();
                    let mask = 1u32 << next;
                    self.bits &= !mask;
                    Some(next as u8)
                }
            }
        }
        Iter { bits: !self.is_free_bits }
    }

    #[inline]
    pub fn slot_side(&self, slot: u8) -> Option<Side> {
        let slot_mask = 1u32 << slot;
        if self.is_free_bits & slot_mask != 0 {
            None
        } else if self.is_bid_bits & slot_mask != 0 {
            Some(Side::Bid)
        } else {
            Some(Side::Ask)
        }
    }

    #[inline]
    pub fn orders_with_client_ids(&self) -> impl Iterator<Item = (NonZeroU64, i128, Side)> + '_ {
        self.iter_filled_slots().filter_map(move |slot| {
            let client_order_id = NonZeroU64::new(self.client_order_ids[slot as usize])?;
            let order_id = self.orders[slot as usize];
            let side = self.slot_side(slot).unwrap();
            Some((client_order_id, order_id, side))
        })
    }
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct PerpAccount {
    pub base_position: i64,     // measured in base lots
    pub quote_position: I80F48, // measured in native quote
    pub long_settled_funding: I80F48,
    pub short_settled_funding: I80F48,
    pub open_orders: PerpOpenOrders,
}

impl PerpAccount {
    pub fn change_position(
        &mut self,
        base_change: i64,     // this is in contract size
        quote_change: I80F48, // this is in native units
        long_funding: I80F48,
        short_funding: I80F48,
    ) -> MerpsResult<()> {
        /*
            How to adjust the funding settled
            FS_t = (FS_t-1 - TF) * BP_t-1 / BP_t + TF

            Funding owed:
            FO_t = (TF - FS_t) * BP_t
        */

        // TODO this check unnecessary if callers are smart
        check!(base_change != 0, MerpsErrorCode::Default)?;

        let bp0 = self.base_position;
        self.base_position += base_change;
        self.quote_position += quote_change;

        if bp0 > 0 {
            if self.base_position <= 0 {
                // implies there was a sign change
                let funding_owed = (long_funding - self.long_settled_funding)
                    * I80F48::from_num(self.base_position);
                self.quote_position -= funding_owed;
                self.short_settled_funding = short_funding;
            } else {
                self.long_settled_funding = (self.long_settled_funding - long_funding)
                    * I80F48::from_num(bp0 / self.base_position)
                    + long_funding;
            }
        } else if bp0 < 0 {
            if self.base_position >= 0 {
                let funding_owed = (short_funding - self.short_settled_funding)
                    * I80F48::from_num(self.base_position);
                self.quote_position -= funding_owed;
                self.long_settled_funding = long_funding;
            } else {
                self.short_settled_funding = (self.short_settled_funding - short_funding)
                    * I80F48::from_num(bp0 / self.base_position)
                    + short_funding;
            }
        } else {
            if base_change > 0 {
                self.long_settled_funding = long_funding;
            } else {
                // base_change must be less than 0, if == 0, that's error state
                self.short_settled_funding = short_funding;
            }
        }

        Ok(())
    }

    /// Move unrealized funding payments into the quote_position
    pub fn move_funding(&mut self, long_funding: I80F48, short_funding: I80F48) {
        if self.base_position > 0 {
            self.quote_position -=
                (long_funding - self.long_settled_funding) * I80F48::from_num(self.base_position);
            self.long_settled_funding = long_funding;
        } else if self.base_position < 0 {
            self.quote_position -=
                (short_funding - self.short_settled_funding) * I80F48::from_num(self.base_position);
            self.short_settled_funding = short_funding;
        }
    }

    /// Return the health factor if position changed by `base_change` at current prices
    fn sim_position_health(
        &self,
        perp_market_info: &PerpMarketInfo,
        price: I80F48,
        asset_weight: I80F48,
        liab_weight: I80F48,
        base_change: i64,
    ) -> I80F48 {
        // TODO make checked
        let new_base = self.base_position + base_change;

        let mut health = self.quote_position
            - I80F48::from_num(base_change * perp_market_info.base_lot_size) * price;
        if new_base > 0 {
            health +=
                I80F48::from_num(new_base * perp_market_info.base_lot_size) * price * asset_weight;
        } else {
            health +=
                I80F48::from_num(new_base * perp_market_info.base_lot_size) * price * liab_weight;
        }

        msg!("sim_position_health price={:?} new_base={} health={:?}", price, new_base, health);

        health
    }

    pub fn get_health(
        &self,
        perp_market_info: &PerpMarketInfo,
        price: I80F48,
        asset_weight: I80F48,
        liab_weight: I80F48,
        long_funding: I80F48,
        short_funding: I80F48,
    ) -> I80F48 {
        // TODO make sure bids and asks quantity are updated on FillEvent

        // Account for orders that are expansionary
        let bids_health = self.sim_position_health(
            perp_market_info,
            price,
            asset_weight,
            liab_weight,
            self.open_orders.bids_quantity,
        );

        let asks_health = self.sim_position_health(
            perp_market_info,
            price,
            asset_weight,
            liab_weight,
            -self.open_orders.asks_quantity,
        );

        // Pick the worse of the two simulated health
        let h = if bids_health < asks_health { bids_health } else { asks_health };

        msg!(
            "get_health h={:?} of={} bp={}",
            h,
            long_funding - self.long_settled_funding,
            self.base_position
        );

        // Account for unrealized funding payments
        // TODO make checked
        // TODO - consider force moving funding into the realized at start of every instruction
        if self.base_position > 0 {
            h - (long_funding - self.long_settled_funding) * I80F48::from_num(self.base_position)
        } else {
            h + (short_funding - self.short_settled_funding) * I80F48::from_num(self.base_position)
        }
    }
}

#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct MerpsAccount {
    pub meta_data: MetaData,

    pub merps_group: Pubkey,
    pub owner: Pubkey,

    pub in_basket: [bool; MAX_TOKENS],

    // Spot and Margin related data
    pub deposits: [I80F48; MAX_TOKENS],
    pub borrows: [I80F48; MAX_TOKENS],
    pub spot_open_orders: [Pubkey; MAX_PAIRS],

    // Perps related data
    pub perp_accounts: [PerpAccount; MAX_PAIRS],
}

pub enum HealthType {
    Maint,
    Init,
}

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

        check_eq!(
            merps_account.meta_data.data_type,
            DataType::MerpsAccount as u8,
            MerpsErrorCode::Default
        )?;
        check!(merps_account.meta_data.is_initialized, MerpsErrorCode::Default)?;
        check_eq!(&merps_account.merps_group, merps_group_pk, MerpsErrorCode::Default)?;

        Ok(merps_account)
    }
    pub fn load_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        merps_group_pk: &Pubkey,
    ) -> MerpsResult<Ref<'a, Self>> {
        check_eq!(account.owner, program_id, MerpsErrorCode::InvalidOwner)?;
        check_eq!(account.data_len(), size_of::<MerpsAccount>(), MerpsErrorCode::Default)?;

        let merps_account = Self::load(account)?;

        check_eq!(
            merps_account.meta_data.data_type,
            DataType::MerpsAccount as u8,
            MerpsErrorCode::Default
        )?;
        check!(merps_account.meta_data.is_initialized, MerpsErrorCode::Default)?;
        check_eq!(&merps_account.merps_group, merps_group_pk, MerpsErrorCode::Default)?;

        Ok(merps_account)
    }
    pub fn get_native_deposit(&self, root_bank: &RootBank, token_i: usize) -> MerpsResult<I80F48> {
        self.deposits[token_i].checked_mul(root_bank.deposit_index).ok_or(throw!())
    }
    pub fn get_native_borrow(&self, root_bank: &RootBank, token_i: usize) -> MerpsResult<I80F48> {
        self.borrows[token_i].checked_mul(root_bank.borrow_index).ok_or(throw!())
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

    fn get_spot_health(
        &self,
        merps_cache: &MerpsCache,
        market_index: usize,
        open_orders_ai: &AccountInfo,
        asset_weight: I80F48,
        liab_weight: I80F48,
    ) -> MerpsResult<I80F48> {
        // TODO make checked
        let bank_cache = &merps_cache.root_bank_cache[market_index];
        let price = merps_cache.price_cache[market_index].price;

        let (oo_base, oo_quote) = if self.spot_open_orders[market_index] == Pubkey::default() {
            (ZERO_I80F48, ZERO_I80F48)
        } else {
            // TODO make sure open orders account is checked for validity before passing in here
            // TODO add in support for GUI hoster fee
            let open_orders = load_open_orders(open_orders_ai)?;
            (
                I80F48::from_num(open_orders.native_coin_total),
                I80F48::from_num(
                    open_orders.native_pc_total + open_orders.referrer_rebates_accrued,
                ),
            )
        };

        let health = (((self.deposits[market_index] * bank_cache.deposit_index + oo_base)
            * asset_weight
            - self.borrows[market_index] * bank_cache.borrow_index * liab_weight)
            * price)
            + oo_quote;

        msg!("get_spot_health {} price={:?} health={:?}", market_index, price, health,);

        Ok(health)
    }

    pub fn get_health(
        &self,
        merps_group: &MerpsGroup,
        merps_cache: &MerpsCache,
        spot_open_orders_ais: &[AccountInfo],
        health_type: HealthType,
    ) -> MerpsResult<I80F48> {
        let mut health = (merps_cache.root_bank_cache[QUOTE_INDEX].deposit_index
            * self.deposits[QUOTE_INDEX])
            - (merps_cache.root_bank_cache[QUOTE_INDEX].borrow_index * self.borrows[QUOTE_INDEX]);

        msg!("get_health quote={:?}", health);

        for i in 0..merps_group.num_oracles {
            if !self.in_basket[i] {
                continue;
            }
            let spot_market_info = &merps_group.spot_markets[i];
            let perp_market_info = &merps_group.perp_markets[i];

            let (spot_asset_weight, spot_liab_weight, perp_asset_weight, perp_liab_weight) =
                match health_type {
                    HealthType::Maint => (
                        spot_market_info.maint_asset_weight,
                        spot_market_info.maint_liab_weight,
                        perp_market_info.maint_asset_weight,
                        perp_market_info.maint_liab_weight,
                    ),
                    HealthType::Init => (
                        spot_market_info.init_asset_weight,
                        spot_market_info.init_liab_weight,
                        perp_market_info.init_asset_weight,
                        perp_market_info.init_liab_weight,
                    ),
                };

            if !spot_market_info.is_empty() {
                health += self.get_spot_health(
                    merps_cache,
                    i,
                    &spot_open_orders_ais[i],
                    spot_asset_weight,
                    spot_liab_weight,
                )?;
            }

            if !perp_market_info.is_empty() {
                health += self.perp_accounts[i].get_health(
                    perp_market_info,
                    merps_cache.price_cache[i].price,
                    perp_asset_weight,
                    perp_liab_weight,
                    merps_cache.perp_market_cache[i].long_funding,
                    merps_cache.perp_market_cache[i].short_funding,
                );
            }
            msg!("get_health {} => {:?}", i, health);
        }

        Ok(health)
    }
}

/// This will hold top level info about the perps market
/// Likely all perps transactions on a market will be locked on this one because this will be passed in as writable
#[derive(Copy, Clone, Pod, Loadable, Default)]
#[repr(C)]
pub struct PerpMarket {
    // TODO consider adding the market_index here for easy lookup
    pub meta_data: MetaData,

    pub merps_group: Pubkey,
    pub bids: Pubkey,
    pub asks: Pubkey,
    pub event_queue: Pubkey,

    pub long_funding: I80F48,
    pub short_funding: I80F48,

    // TODO - remove open interest, not being used except maybe on stats?
    pub open_interest: i64, // This is i64 to keep consistent with the units of contracts, but should always be > 0

    pub quote_lot_size: i64, // number of quote native that reresents min tick
    pub index_oracle: Pubkey, // TODO - remove, not being used
    pub last_updated: u64,
    pub seq_num: u64,

    pub contract_size: i64, // represents number of base native quantity; greater than 0

                            // mark_price = used to liquidate and calculate value of positions; function of index and some moving average of basis
                            // index_price = some function of centralized exchange spot prices
                            // book_price = average of impact bid and impact ask; used to calculate basis
                            // basis = book_price / index_price - 1; some moving average of this is used for mark price
}

impl PerpMarket {
    pub fn load_and_init<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        merps_group_ai: &'a AccountInfo,
        bids_ai: &'a AccountInfo,
        asks_ai: &'a AccountInfo,
        event_queue_ai: &'a AccountInfo,

        merps_group: &MerpsGroup,
        rent: &Rent,

        market_index: usize,
        contract_size: i64,
        quote_lot_size: i64,
    ) -> MerpsResult<RefMut<'a, Self>> {
        let mut state = Self::load_mut(account)?;
        check!(account.owner == program_id, MerpsErrorCode::InvalidOwner)?;
        check!(
            rent.is_exempt(account.lamports(), size_of::<Self>()),
            MerpsErrorCode::AccountNotRentExempt
        )?;
        check!(!state.meta_data.is_initialized, MerpsErrorCode::Default)?;

        state.meta_data = MetaData::new(DataType::PerpMarket, 0, true);
        state.merps_group = *merps_group_ai.key;
        state.bids = *bids_ai.key;
        state.asks = *asks_ai.key;
        state.event_queue = *event_queue_ai.key;
        state.quote_lot_size = quote_lot_size;
        state.index_oracle = merps_group.oracles[market_index];
        state.contract_size = contract_size;

        let clock = Clock::get()?;
        state.last_updated = clock.unix_timestamp as u64;

        Ok(state)
    }

    pub fn load_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        merps_group_pk: &Pubkey,
    ) -> MerpsResult<Ref<'a, Self>> {
        check_eq!(account.owner, program_id, MerpsErrorCode::InvalidOwner)?;
        let state = Self::load(account)?;
        check!(state.meta_data.is_initialized, MerpsErrorCode::Default)?;
        check!(state.meta_data.data_type == DataType::PerpMarket as u8, MerpsErrorCode::Default)?;
        check!(merps_group_pk == &state.merps_group, MerpsErrorCode::Default)?;
        Ok(state)
    }

    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        merps_group_pk: &Pubkey,
    ) -> MerpsResult<RefMut<'a, Self>> {
        check_eq!(account.owner, program_id, MerpsErrorCode::InvalidOwner)?;
        let state = Self::load_mut(account)?;
        check!(state.meta_data.is_initialized, MerpsErrorCode::Default)?;
        check!(state.meta_data.data_type == DataType::PerpMarket as u8, MerpsErrorCode::Default)?;
        check!(merps_group_pk == &state.merps_group, MerpsErrorCode::Default)?;
        Ok(state)
    }

    pub fn gen_order_id(&mut self, side: Side, price: i64) -> i128 {
        self.seq_num += 1;

        let upper = (price as i128) << 64;
        match side {
            Side::Bid => upper | (!self.seq_num as i128),
            Side::Ask => upper | (self.seq_num as i128),
        }
    }

    /// Use current order book price
    pub fn update_funding(
        &mut self,
        merps_group: &MerpsGroup,
        book: &Book,
        merps_cache: &MerpsCache,
        market_index: usize,
        now_ts: u64,
    ) -> MerpsResult<()> {
        // Get the index price from cache, ensure it's not outdated
        let price_cache = &merps_cache.price_cache[market_index];
        check!(
            now_ts <= price_cache.last_update + merps_group.valid_interval,
            MerpsErrorCode::InvalidCache
        )?;
        let index_price = price_cache.price;

        // Get current book price & compare it to index price

        // TODO get impact bid and impact ask if compute allows
        // TODO consider corner cases of funding being updated
        let bid = book.get_best_bid_price();
        let ask = book.get_best_ask_price();

        // verify that at least one order is on the book
        check!(bid.is_some() || ask.is_some(), MerpsErrorCode::Default)?;

        const ONE_SIDED_PENALTY_FUNDING: I80F48 = I80F48!(0.05);
        let diff = match (bid, ask) {
            (Some(bid), Some(ask)) => {
                // calculate mid-market rate
                let book_price = self.lot_to_native_price((bid + ask) / 2);
                (book_price / index_price) - ONE_I80F48
            }
            (Some(_bid), None) => ONE_SIDED_PENALTY_FUNDING,
            (None, Some(_ask)) => ONE_SIDED_PENALTY_FUNDING,
            (None, None) => ZERO_I80F48, // checked already before for this case
        };

        // TODO consider what happens if time_factor is very small. Can funding_delta == 0 when diff != 0?
        let time_factor = I80F48::from_num(now_ts - self.last_updated) / DAY;
        let funding_delta: I80F48 = diff
            * time_factor
            * I80F48::from_num(self.contract_size)  // TODO check cost of conversion op
            * index_price;

        msg!(
            "update_funding diff={:?} tf={:?} delta={:?} ds={}-{}",
            diff,
            funding_delta,
            time_factor,
            now_ts,
            self.last_updated
        );

        self.long_funding += funding_delta;
        self.short_funding += funding_delta;
        self.last_updated = now_ts;

        Ok(())
    }

    /// Convert from the price stored on the book to the price used in value calculations
    pub fn lot_to_native_price(&self, price: i64) -> I80F48 {
        I80F48::from_num(price)
            .checked_mul(I80F48::from_num(self.quote_lot_size))
            .unwrap()
            .checked_div(I80F48::from_num(self.contract_size))
            .unwrap()
    }
}

pub fn load_market_state<'a>(
    market_account: &'a AccountInfo,
    program_id: &Pubkey,
) -> MerpsResult<RefMut<'a, serum_dex::state::MarketState>> {
    check_eq!(market_account.owner, program_id, MerpsErrorCode::Default)?;

    let state: RefMut<'a, serum_dex::state::MarketState>;
    state = RefMut::map(market_account.try_borrow_mut_data()?, |data| {
        let data_len = data.len() - 12;
        let (_, rest) = data.split_at_mut(5);
        let (mid, _) = rest.split_at_mut(data_len);
        from_bytes_mut(mid)
    });

    state.check_flags()?;
    Ok(state)
}

fn strip_dex_padding<'a>(acc: &'a AccountInfo) -> MerpsResult<Ref<'a, [u8]>> {
    check!(acc.data_len() >= 12, MerpsErrorCode::Default)?;
    let unpadded_data: Ref<[u8]> = Ref::map(acc.try_borrow_data()?, |data| {
        let data_len = data.len() - 12;
        let (_, rest) = data.split_at(5);
        let (mid, _) = rest.split_at(data_len);
        mid
    });
    Ok(unpadded_data)
}

pub fn load_open_orders<'a>(
    acc: &'a AccountInfo,
) -> Result<Ref<'a, serum_dex::state::OpenOrders>, ProgramError> {
    Ok(Ref::map(strip_dex_padding(acc)?, from_bytes))
}

pub fn check_open_orders(acc: &AccountInfo, owner: &Pubkey) -> MerpsResult<()> {
    if *acc.key == Pubkey::default() {
        return Ok(());
    }
    // if it's not default, it must be initialized
    let open_orders = load_open_orders(acc)?;
    let valid_flags = (serum_dex::state::AccountFlag::Initialized
        | serum_dex::state::AccountFlag::OpenOrders)
        .bits();
    check_eq!(open_orders.account_flags, valid_flags, MerpsErrorCode::Default)?;
    check_eq!(identity(open_orders.owner), owner.to_aligned_bytes(), MerpsErrorCode::Default)?;

    Ok(())
}
