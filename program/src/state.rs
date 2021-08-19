use std::cell::{Ref, RefMut};
use std::cmp::{max, min};
use std::convert::identity;
use std::mem::size_of;

use bytemuck::{from_bytes, from_bytes_mut, try_from_bytes_mut, Pod, Zeroable};
use enumflags2::BitFlags;
use fixed::types::I80F48;
use fixed_macro::types::I80F48;
use mango_common::Loadable;
use mango_macro::{Loadable, Pod};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{Deserialize, Serialize};
use serum_dex::state::ToAlignedBytes;
use solana_program::account_info::AccountInfo;
use solana_program::log::sol_log_compute_units;
use solana_program::msg;
use solana_program::program_error::ProgramError;
use solana_program::program_pack::Pack;
use solana_program::pubkey::Pubkey;
use solana_program::sysvar::{clock::Clock, rent::Rent, Sysvar};
use spl_token::state::Account;

use crate::error::{check_assert, MangoError, MangoErrorCode, MangoResult, SourceFileId};
use crate::ids::mngo_token;
use crate::matching::{Book, LeafNode, Side};
use crate::queue::FillEvent;
use crate::utils::{invert_side, remove_slop_mut, split_open_orders};

pub const MAX_TOKENS: usize = 16; // Just changed
pub const MAX_PAIRS: usize = MAX_TOKENS - 1;
pub const MAX_NODE_BANKS: usize = 8;
pub const QUOTE_INDEX: usize = MAX_TOKENS - 1;
pub const ZERO_I80F48: I80F48 = I80F48!(0);
pub const ONE_I80F48: I80F48 = I80F48!(1);
pub const DAY: I80F48 = I80F48!(86400);
pub const YEAR: I80F48 = I80F48!(31536000);

pub const DUST_THRESHOLD: I80F48 = I80F48!(1); // TODO make this part of MangoGroup state
const MAX_RATE_ADJ: I80F48 = I80F48!(4); // TODO make this part of PerpMarket if we want per market flexibility
const MIN_RATE_ADJ: I80F48 = I80F48!(0.25);
pub const INFO_LEN: usize = 32;
pub const MAX_PERP_OPEN_ORDERS: usize = 64;
pub const FREE_ORDER_SLOT: u8 = u8::MAX; // TODO add check to prevent markets more than 255
pub const MAX_NUM_IN_MARGIN_BASKET: u8 = 10;

declare_check_assert_macros!(SourceFileId::State);

// NOTE: I80F48 multiplication ops are very expensive. Avoid when possible
// TODO: add prop tests for nums
// TODO add GUI hoster fee discount

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
    MangoGroup = 0,
    MangoAccount,
    RootBank,
    NodeBank,
    PerpMarket,
    Bids,
    Asks,
    MangoCache,
    EventQueue,
}

const NUM_HEALTHS: usize = 2;
#[repr(usize)]
#[derive(Copy, Clone, IntoPrimitive, TryFromPrimitive)]
pub enum HealthType {
    Maint,
    Init,
}

#[derive(
    Eq, PartialEq, Copy, Clone, TryFromPrimitive, IntoPrimitive, Serialize, Deserialize, Debug,
)]
#[repr(u8)]
pub enum AssetType {
    Token = 0,
    Perp = 1,
}

#[derive(Copy, Clone, Pod, Default)]
#[repr(C)]
/// Stores meta information about the `Account` on chain
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

// TODO - move all the weights from SpotMarketInfo to TokenInfo
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
    pub liquidation_fee: I80F48,
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
    pub maker_fee: I80F48,
    pub taker_fee: I80F48,
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
pub struct MangoGroup {
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
    pub mango_cache: Pubkey,
    pub valid_interval: u64,

    // insurance vault is funded by the Mango DAO with USDC and can be withdrawn by the DAO
    pub insurance_vault: Pubkey,
    pub srm_vault: Pubkey,
    pub msrm_vault: Pubkey,
    pub fees_vault: Pubkey,

    pub padding: [u8; 32], // padding used for future expansions
}

impl MangoGroup {
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MangoResult<RefMut<'a, Self>> {
        check_eq!(account.data_len(), size_of::<Self>(), MangoErrorCode::Default)?;
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;

        let mango_group = Self::load_mut(account)?;
        check_eq!(
            mango_group.meta_data.data_type,
            DataType::MangoGroup as u8,
            MangoErrorCode::Default
        )?;

        Ok(mango_group)
    }
    pub fn load_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MangoResult<Ref<'a, Self>> {
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;

        let mango_group = Self::load(account)?;
        check!(mango_group.meta_data.is_initialized, MangoErrorCode::Default)?;
        check_eq!(
            mango_group.meta_data.data_type,
            DataType::MangoGroup as u8,
            MangoErrorCode::Default
        )?;

        Ok(mango_group)
    }

    pub fn find_oracle_index(&self, oracle_pk: &Pubkey) -> Option<usize> {
        self.oracles.iter().position(|pk| pk == oracle_pk) // TODO OPT profile
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

    pub optimal_util: I80F48,
    pub optimal_rate: I80F48,
    pub max_rate: I80F48,

    pub num_node_banks: usize,
    pub node_banks: [Pubkey; MAX_NODE_BANKS],

    pub deposit_index: I80F48,
    pub borrow_index: I80F48,
    pub last_updated: u64,

    padding: [u8; 64], // used for future expansions
}

impl RootBank {
    pub fn load_and_init<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        node_bank_ai: &'a AccountInfo,
        rent: &Rent,
        optimal_util: I80F48,
        optimal_rate: I80F48,
        max_rate: I80F48,
    ) -> MangoResult<RefMut<'a, Self>> {
        let mut root_bank = Self::load_mut(account)?;
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;
        check!(
            rent.is_exempt(account.lamports(), size_of::<Self>()),
            MangoErrorCode::AccountNotRentExempt
        )?;
        check!(!root_bank.meta_data.is_initialized, MangoErrorCode::Default)?;

        root_bank.meta_data = MetaData::new(DataType::RootBank, 0, true);
        root_bank.node_banks[0] = *node_bank_ai.key;
        root_bank.num_node_banks = 1;
        root_bank.deposit_index = ONE_I80F48;
        root_bank.borrow_index = ONE_I80F48;

        root_bank.set_rate_params(optimal_util, optimal_rate, max_rate)?;
        Ok(root_bank)
    }
    pub fn set_rate_params(
        &mut self,
        optimal_util: I80F48,
        optimal_rate: I80F48,
        max_rate: I80F48,
    ) -> MangoResult<()> {
        check!(
            optimal_util > ZERO_I80F48 && optimal_util < ONE_I80F48,
            MangoErrorCode::InvalidParam
        )?;
        check!(optimal_rate >= ZERO_I80F48, MangoErrorCode::InvalidParam)?;
        check!(max_rate >= ZERO_I80F48, MangoErrorCode::InvalidParam)?;

        self.optimal_util = optimal_util;
        self.optimal_rate = optimal_rate / YEAR;
        self.max_rate = max_rate / YEAR;

        Ok(())
    }
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MangoResult<RefMut<'a, Self>> {
        check_eq!(account.data_len(), size_of::<Self>(), MangoErrorCode::Default)?;
        check_eq!(account.owner, program_id, MangoErrorCode::Default)?;

        let root_bank = Self::load_mut(account)?;

        check!(root_bank.meta_data.is_initialized, MangoErrorCode::Default)?;
        check_eq!(
            root_bank.meta_data.data_type,
            DataType::RootBank as u8,
            MangoErrorCode::Default
        )?;

        Ok(root_bank)
    }
    pub fn load_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MangoResult<Ref<'a, Self>> {
        check_eq!(account.data_len(), size_of::<Self>(), MangoErrorCode::Default)?;
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;

        let root_bank = Self::load(account)?;

        check!(root_bank.meta_data.is_initialized, MangoErrorCode::Default)?;
        check_eq!(
            root_bank.meta_data.data_type,
            DataType::RootBank as u8,
            MangoErrorCode::Default
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
    ) -> MangoResult<()> {
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
        let interest_rate = if utilization > self.optimal_util {
            let extra_util = utilization - self.optimal_util;
            let slope = (self.max_rate - self.optimal_rate) / (ONE_I80F48 - self.optimal_util);
            self.optimal_rate + slope * extra_util
        } else {
            let slope = self.optimal_rate / self.optimal_util;
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

    pub fn socialize_loss(
        &mut self,
        program_id: &Pubkey,
        token_index: usize,
        mango_cache: &mut MangoCache,
        bankrupt_account: &mut MangoAccount,
        node_bank_ais: &[AccountInfo; MAX_NODE_BANKS],
        liab_node_bank: &mut NodeBank,
        liab_node_bank_key: &Pubkey,
    ) -> MangoResult<()> {
        let mut native_deposits = liab_node_bank.deposits.checked_mul(self.deposit_index).unwrap();
        let mut native_borrows = liab_node_bank.borrows.checked_mul(self.borrow_index).unwrap();

        let mut max_node_bank_index = 0;
        let mut max_node_bank_borrows = ZERO_I80F48;

        for i in 0..self.num_node_banks {
            check!(node_bank_ais[i].key == &self.node_banks[i], MangoErrorCode::InvalidNodeBank)?;

            if liab_node_bank_key == node_bank_ais[i].key {
                continue;
            }

            let node_bank = NodeBank::load_checked(&node_bank_ais[i], program_id)?;
            native_deposits = native_deposits
                .checked_add(node_bank.deposits.checked_mul(self.deposit_index).unwrap())
                .unwrap();

            native_borrows = native_borrows
                .checked_add(node_bank.borrows.checked_mul(self.borrow_index).unwrap())
                .unwrap();

            if node_bank.borrows > max_node_bank_borrows {
                max_node_bank_index = i;
                max_node_bank_borrows = node_bank.borrows;
            }
        }

        let loss = bankrupt_account.borrows[token_index];
        let native_loss: I80F48 = loss * self.borrow_index;

        let percentage_loss = native_loss.checked_div(native_deposits).unwrap();
        self.deposit_index = self
            .deposit_index
            .checked_sub(percentage_loss.checked_mul(self.deposit_index).unwrap())
            .unwrap();

        mango_cache.root_bank_cache[token_index].deposit_index = self.deposit_index;
        mango_cache.root_bank_cache[token_index].borrow_index = self.borrow_index;

        if node_bank_ais[max_node_bank_index].key == liab_node_bank_key {
            bankrupt_account.checked_sub_borrow(token_index, loss)?;
            liab_node_bank.checked_sub_borrow(loss)?;
        } else {
            let mut node_bank =
                NodeBank::load_mut_checked(&node_bank_ais[max_node_bank_index], program_id)?;

            bankrupt_account.checked_sub_borrow(token_index, loss)?;
            node_bank.checked_sub_borrow(loss)?;
        }

        msg!(
            "token_socialized_loss details: {{ \"liab_index\": {}, \"native_loss\":{}, \"percentage_loss\": {} }}",
            token_index,
            native_loss.to_num::<f64>(),
            percentage_loss.to_num::<f64>()
        );

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
    ) -> MangoResult<RefMut<'a, Self>> {
        let mut node_bank = Self::load_mut(account)?;
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;
        check!(
            rent.is_exempt(account.lamports(), size_of::<Self>()),
            MangoErrorCode::AccountNotRentExempt
        )?;
        check!(!node_bank.meta_data.is_initialized, MangoErrorCode::Default)?;

        node_bank.meta_data = MetaData::new(DataType::NodeBank, 0, true);
        node_bank.deposits = ZERO_I80F48;
        node_bank.borrows = ZERO_I80F48;
        node_bank.vault = *vault_ai.key;

        Ok(node_bank)
    }
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MangoResult<RefMut<'a, Self>> {
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;

        // TODO verify if size check necessary. We know load_mut fails if account size is too small for struct,
        //  does it also fail if it's too big?
        check_eq!(account.data_len(), size_of::<Self>(), MangoErrorCode::Default)?;
        let node_bank = Self::load_mut(account)?;

        check!(node_bank.meta_data.is_initialized, MangoErrorCode::Default)?;
        check_eq!(
            node_bank.meta_data.data_type,
            DataType::NodeBank as u8,
            MangoErrorCode::Default
        )?;

        Ok(node_bank)
    }
    #[allow(unused)]
    pub fn load_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MangoResult<Ref<'a, Self>> {
        // TODO
        Ok(Self::load(account)?)
    }

    // TODO - Add checks to these math methods to prevent result from being < 0
    pub fn checked_add_borrow(&mut self, v: I80F48) -> MangoResult<()> {
        Ok(self.borrows = self.borrows.checked_add(v).ok_or(throw!())?)
    }
    pub fn checked_sub_borrow(&mut self, v: I80F48) -> MangoResult<()> {
        Ok(self.borrows = self.borrows.checked_sub(v).ok_or(throw!())?)
    }
    pub fn checked_add_deposit(&mut self, v: I80F48) -> MangoResult<()> {
        Ok(self.deposits = self.deposits.checked_add(v).ok_or(throw!())?)
    }
    pub fn checked_sub_deposit(&mut self, v: I80F48) -> MangoResult<()> {
        Ok(self.deposits = self.deposits.checked_sub(v).ok_or(throw!())?)
    }
    pub fn has_valid_deposits_borrows(&self, root_bank_cache: &RootBankCache) -> bool {
        self.get_total_native_deposit(root_bank_cache)
            >= self.get_total_native_borrow(root_bank_cache)
    }
    pub fn get_total_native_borrow(&self, root_bank_cache: &RootBankCache) -> u64 {
        let native: I80F48 = self.borrows * root_bank_cache.borrow_index;
        native.checked_ceil().unwrap().to_num() // rounds toward +inf
    }
    pub fn get_total_native_deposit(&self, root_bank_cache: &RootBankCache) -> u64 {
        let native: I80F48 = self.deposits * root_bank_cache.deposit_index;
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
pub struct MangoCache {
    pub meta_data: MetaData,

    pub price_cache: [PriceCache; MAX_PAIRS],
    pub root_bank_cache: [RootBankCache; MAX_TOKENS],
    pub perp_market_cache: [PerpMarketCache; MAX_PAIRS],
}

impl MangoCache {
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        mango_group: &MangoGroup,
    ) -> MangoResult<RefMut<'a, Self>> {
        // mango account must be rent exempt to even be initialized
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;
        let mango_cache = Self::load_mut(account)?;

        check_eq!(
            mango_cache.meta_data.data_type,
            DataType::MangoCache as u8,
            MangoErrorCode::Default
        )?;
        check!(mango_cache.meta_data.is_initialized, MangoErrorCode::Default)?;
        check_eq!(&mango_group.mango_cache, account.key, MangoErrorCode::Default)?;

        Ok(mango_cache)
    }

    pub fn load_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        mango_group: &MangoGroup,
    ) -> MangoResult<Ref<'a, Self>> {
        check_eq!(account.data_len(), size_of::<Self>(), MangoErrorCode::Default)?;
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;

        let mango_cache = Self::load(account)?;

        check_eq!(
            mango_cache.meta_data.data_type,
            DataType::MangoCache as u8,
            MangoErrorCode::Default
        )?;
        check!(mango_cache.meta_data.is_initialized, MangoErrorCode::Default)?;
        check_eq!(&mango_group.mango_cache, account.key, MangoErrorCode::Default)?;

        Ok(mango_cache)
    }

    pub fn check_valid(
        &self,
        mango_group: &MangoGroup,
        active_assets: &UserActiveAssets,
        now_ts: u64,
    ) -> MangoResult<()> {
        let valid_interval = mango_group.valid_interval;
        check!(
            now_ts <= self.root_bank_cache[QUOTE_INDEX].last_update + valid_interval,
            MangoErrorCode::InvalidRootBankCache
        )?;

        for i in 0..mango_group.num_oracles {
            if active_assets.spot[i] || active_assets.perps[i] {
                check!(
                    now_ts <= self.price_cache[i].last_update + valid_interval,
                    MangoErrorCode::InvalidPriceCache
                )?;
            }

            if active_assets.spot[i] {
                check!(
                    now_ts <= self.root_bank_cache[i].last_update + valid_interval,
                    MangoErrorCode::InvalidRootBankCache
                )?;
            }

            if active_assets.perps[i] {
                check!(
                    now_ts <= self.perp_market_cache[i].last_update + valid_interval,
                    MangoErrorCode::InvalidRootBankCache
                )?;
            }
        }
        Ok(())
    }
    // TODO - only check caches are valid if balances are non-zero
    pub fn check_caches_valid(
        &self,
        mango_group: &MangoGroup,
        active_assets: &[bool; MAX_TOKENS],
        now_ts: u64,
    ) -> bool {
        let valid_interval = mango_group.valid_interval;
        if now_ts > self.root_bank_cache[QUOTE_INDEX].last_update + valid_interval {
            msg!(
                "root_bank_cache {} invalid: {}",
                QUOTE_INDEX,
                self.root_bank_cache[QUOTE_INDEX].last_update
            );
            return false;
        }

        for i in 0..mango_group.num_oracles {
            // If this asset is not in user basket, then there are no deposits, borrows or perp positions to calculate value of
            if !active_assets[i] {
                continue;
            }

            if now_ts > self.price_cache[i].last_update + valid_interval {
                msg!("price_cache {} invalid: {}", i, self.price_cache[i].last_update);
                return false;
            }

            if !mango_group.spot_markets[i].is_empty() {
                if now_ts > self.root_bank_cache[i].last_update + valid_interval {
                    msg!("root_bank_cache {} invalid: {}", i, self.root_bank_cache[i].last_update);
                    return false;
                }
            }

            if !mango_group.perp_markets[i].is_empty() {
                if now_ts > self.perp_market_cache[i].last_update + valid_interval {
                    msg!(
                        "perp_market_cache {} invalid: {}",
                        i,
                        self.perp_market_cache[i].last_update
                    );
                    return false;
                }
            }
        }

        true
    }

    pub fn get_price(&self, i: usize) -> I80F48 {
        if i == QUOTE_INDEX {
            ONE_I80F48
        } else {
            self.price_cache[i].price // Just panic if index out of bounds
        }
    }
}

pub struct UserActiveAssets {
    pub spot: [bool; MAX_PAIRS],
    pub perps: [bool; MAX_PAIRS],
}

impl UserActiveAssets {
    pub fn new(
        mango_group: &MangoGroup,
        mango_account: &MangoAccount,
        extra: Vec<(AssetType, usize)>,
    ) -> Self {
        let mut spot = [false; MAX_PAIRS];
        let mut perps = [false; MAX_PAIRS];
        for i in 0..mango_group.num_oracles {
            spot[i] = !mango_group.spot_markets[i].is_empty()
                && (mango_account.in_margin_basket[i]
                    || !mango_account.deposits[i].is_zero()
                    || !mango_account.borrows[i].is_zero());

            perps[i] = !mango_group.perp_markets[i].is_empty()
                && mango_account.perp_accounts[i].is_active();
        }
        extra.iter().for_each(|(at, i)| match at {
            AssetType::Token => {
                if *i != QUOTE_INDEX {
                    spot[*i] = true;
                }
            }
            AssetType::Perp => {
                perps[*i] = true;
            }
        });
        Self { spot, perps }
    }

    pub fn merge(a: &Self, b: &Self) -> Self {
        let mut spot = [false; MAX_PAIRS];
        let mut perps = [false; MAX_PAIRS];
        for i in 0..MAX_PAIRS {
            spot[i] = a.spot[i] || b.spot[i];
            perps[i] = a.perps[i] || b.perps[i];
        }
        Self { spot, perps }
    }
}

pub struct HealthCache {
    // pub active_assets: [bool; MAX_TOKENS],
    // pub active_spot: [bool; MAX_PAIRS],
    // pub active_perps: [bool; MAX_PAIRS],
    pub active_assets: UserActiveAssets,

    /// Vec of length MAX_PAIRS containing worst case spot vals; unweighted
    spot: Vec<(I80F48, I80F48)>,
    perp: Vec<(I80F48, I80F48)>,
    quote: I80F48,

    /// This will be zero until update_health is called for the first time
    health: [Option<I80F48>; 2],
}

impl HealthCache {
    pub fn new(active_assets: UserActiveAssets) -> Self {
        Self {
            active_assets,
            spot: vec![(ZERO_I80F48, ZERO_I80F48); MAX_PAIRS],
            perp: vec![(ZERO_I80F48, ZERO_I80F48); MAX_PAIRS],
            quote: ZERO_I80F48,
            health: [None; NUM_HEALTHS],
        }
    }

    pub fn init_vals(
        &mut self,
        mango_group: &MangoGroup,
        mango_cache: &MangoCache,
        mango_account: &MangoAccount,
        open_orders_ais: &[AccountInfo; MAX_PAIRS],
    ) -> MangoResult<()> {
        self.quote = mango_account.get_net(&mango_cache.root_bank_cache[QUOTE_INDEX], QUOTE_INDEX);
        for i in 0..mango_group.num_oracles {
            if self.active_assets.spot[i] {
                self.spot[i] = mango_account.get_spot_val(
                    &mango_cache.root_bank_cache[i],
                    mango_cache.price_cache[i].price,
                    i,
                    &open_orders_ais[i],
                )?;
            }

            if self.active_assets.perps[i] {
                self.perp[i] = mango_account.perp_accounts[i].get_val(
                    &mango_group.perp_markets[i],
                    &mango_cache.perp_market_cache[i],
                    mango_cache.price_cache[i].price,
                )?;
            }
        }
        Ok(())
    }

    pub fn get_health(&mut self, mango_group: &MangoGroup, health_type: HealthType) -> I80F48 {
        let health_index = health_type as usize;
        match self.health[health_index] {
            None => {
                // apply weights, cache result, return health
                let mut health = self.quote;
                for i in 0..mango_group.num_oracles {
                    let spot_market_info = &mango_group.spot_markets[i];
                    let perp_market_info = &mango_group.perp_markets[i];

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

                    if self.active_assets.spot[i] {
                        let (base, quote) = self.spot[i];
                        if base.is_negative() {
                            health += base * spot_liab_weight + quote;
                        } else {
                            health += base * spot_asset_weight + quote
                        }
                    }

                    if self.active_assets.perps[i] {
                        let (base, quote) = self.perp[i];
                        if base.is_negative() {
                            health += base * perp_liab_weight + quote;
                        } else {
                            health += base * perp_asset_weight + quote
                        }
                    }
                }

                self.health[health_index] = Some(health);
                health
            }
            Some(h) => h,
        }
    }

    pub fn update_quote(&mut self, mango_cache: &MangoCache, mango_account: &MangoAccount) {
        let quote = mango_account.get_net(&mango_cache.root_bank_cache[QUOTE_INDEX], QUOTE_INDEX);
        for i in 0..NUM_HEALTHS {
            if let Some(h) = self.health[i] {
                self.health[i] = Some(h + quote - self.quote);
            }
        }
        self.quote = quote;
    }

    /// Note market_index < QUOTE_INDEX
    pub fn update_spot_val(
        &mut self,
        mango_group: &MangoGroup,
        mango_cache: &MangoCache,
        mango_account: &MangoAccount,
        open_orders_ai: &AccountInfo,
        market_index: usize,
    ) -> MangoResult<()> {
        let (base, quote) = mango_account.get_spot_val(
            &mango_cache.root_bank_cache[market_index],
            mango_cache.price_cache[market_index].price,
            market_index,
            open_orders_ai,
        )?;

        let (prev_base, prev_quote) = self.spot[market_index];

        for i in 0..NUM_HEALTHS {
            if let Some(h) = self.health[i] {
                let health_type: HealthType = HealthType::try_from_primitive(i).unwrap();
                let smi = &mango_group.spot_markets[market_index];

                let (asset_weight, liab_weight) = match health_type {
                    HealthType::Maint => (smi.maint_asset_weight, smi.maint_liab_weight),
                    HealthType::Init => (smi.init_asset_weight, smi.init_liab_weight),
                };

                // Get health from val
                let prev_spot_health = if prev_base.is_negative() {
                    prev_base * liab_weight + prev_quote
                } else {
                    prev_base * asset_weight + prev_quote
                };

                let curr_spot_health = if base.is_negative() {
                    base * liab_weight + quote
                } else {
                    base * asset_weight + quote
                };

                self.health[i] = Some(h + curr_spot_health - prev_spot_health);
            }
        }

        self.spot[market_index] = (base, quote);

        Ok(())
    }

    /// Sends to update_quote if QUOTE_INDEX, else sends to update_spot_val
    pub fn update_token_val(
        &mut self,
        mango_group: &MangoGroup,
        mango_cache: &MangoCache,
        mango_account: &MangoAccount,
        open_orders_ais: &[AccountInfo; MAX_PAIRS],
        token_index: usize,
    ) -> MangoResult<()> {
        if token_index == QUOTE_INDEX {
            Ok(self.update_quote(mango_cache, mango_account))
        } else {
            self.update_spot_val(
                mango_group,
                mango_cache,
                mango_account,
                &open_orders_ais[token_index],
                token_index,
            )
        }
    }

    /// Update perp val and then update the healths
    pub fn update_perp_val(
        &mut self,
        mango_group: &MangoGroup,
        mango_cache: &MangoCache,
        mango_account: &MangoAccount,
        market_index: usize,
    ) -> MangoResult<()> {
        let (base, quote) = mango_account.perp_accounts[market_index].get_val(
            &mango_group.perp_markets[market_index],
            &mango_cache.perp_market_cache[market_index],
            mango_cache.price_cache[market_index].price,
        )?;

        let (prev_base, prev_quote) = self.perp[market_index];

        for i in 0..NUM_HEALTHS {
            if let Some(h) = self.health[i] {
                let health_type: HealthType = HealthType::try_from_primitive(i).unwrap();
                let pmi = &mango_group.perp_markets[market_index];

                let (asset_weight, liab_weight) = match health_type {
                    HealthType::Maint => (pmi.maint_asset_weight, pmi.maint_liab_weight),
                    HealthType::Init => (pmi.init_asset_weight, pmi.init_liab_weight),
                };

                // Get health from val
                let prev_perp_health = if prev_base.is_negative() {
                    prev_base * liab_weight + prev_quote
                } else {
                    prev_base * asset_weight + prev_quote
                };

                let curr_perp_health = if base.is_negative() {
                    base * liab_weight + quote
                } else {
                    base * asset_weight + quote
                };

                self.health[i] = Some(h + curr_perp_health - prev_perp_health);
            }
        }

        self.perp[market_index] = (base, quote);

        Ok(())
    }
}

#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct MangoAccount {
    pub meta_data: MetaData,

    pub mango_group: Pubkey,
    pub owner: Pubkey,

    pub in_margin_basket: [bool; MAX_PAIRS],
    pub num_in_margin_basket: u8,

    // Spot and Margin related data
    pub deposits: [I80F48; MAX_TOKENS],
    pub borrows: [I80F48; MAX_TOKENS],
    pub spot_open_orders: [Pubkey; MAX_PAIRS],

    // Perps related data
    pub perp_accounts: [PerpAccount; MAX_PAIRS],

    pub order_market: [u8; MAX_PERP_OPEN_ORDERS],
    pub order_side: [Side; MAX_PERP_OPEN_ORDERS],
    pub orders: [i128; MAX_PERP_OPEN_ORDERS],
    pub client_order_ids: [u64; MAX_PERP_OPEN_ORDERS],

    pub msrm_amount: u64,

    /// This account cannot open new positions or borrow until `init_health >= 0`
    pub being_liquidated: bool,

    /// This account cannot do anything except go through `resolve_bankruptcy`
    pub is_bankrupt: bool,
    pub info: [u8; INFO_LEN],
    /// padding for expansions
    pub padding: [u8; 70],
}

impl MangoAccount {
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        mango_group_pk: &Pubkey,
    ) -> MangoResult<RefMut<'a, Self>> {
        // load_mut checks for size already
        // mango account must be rent exempt to even be initialized
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;
        let mango_account = Self::load_mut(account)?;

        check_eq!(
            mango_account.meta_data.data_type,
            DataType::MangoAccount as u8,
            MangoErrorCode::Default
        )?;
        check!(mango_account.meta_data.is_initialized, MangoErrorCode::Default)?;
        check_eq!(&mango_account.mango_group, mango_group_pk, MangoErrorCode::Default)?;

        Ok(mango_account)
    }
    pub fn load_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        mango_group_pk: &Pubkey,
    ) -> MangoResult<Ref<'a, Self>> {
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;
        check_eq!(account.data_len(), size_of::<MangoAccount>(), MangoErrorCode::Default)?;

        let mango_account = Self::load(account)?;

        check_eq!(
            mango_account.meta_data.data_type,
            DataType::MangoAccount as u8,
            MangoErrorCode::Default
        )?;
        check!(mango_account.meta_data.is_initialized, MangoErrorCode::Default)?;
        check_eq!(&mango_account.mango_group, mango_group_pk, MangoErrorCode::Default)?;

        Ok(mango_account)
    }
    pub fn get_native_deposit(
        &self,
        root_bank_cache: &RootBankCache,
        token_i: usize,
    ) -> MangoResult<I80F48> {
        self.deposits[token_i].checked_mul(root_bank_cache.deposit_index).ok_or(math_err!())
    }
    pub fn get_native_borrow(
        &self,
        root_bank_cache: &RootBankCache,
        token_i: usize,
    ) -> MangoResult<I80F48> {
        self.borrows[token_i].checked_mul(root_bank_cache.borrow_index).ok_or(math_err!())
    }

    // TODO - Add unchecked versions to be used when we're confident
    // TODO OPT - remove negative and zero checks if we're confident
    pub fn checked_add_borrow(&mut self, token_i: usize, v: I80F48) -> MangoResult<()> {
        self.borrows[token_i] = self.borrows[token_i].checked_add(v).ok_or(math_err!())?;
        // TODO - actually try to hit this error
        check!(
            self.borrows[token_i].is_zero() || self.deposits[token_i].is_zero(),
            MangoErrorCode::MathError
        )
    }
    pub fn checked_sub_borrow(&mut self, token_i: usize, v: I80F48) -> MangoResult<()> {
        self.borrows[token_i] = self.borrows[token_i].checked_sub(v).ok_or(math_err!())?;
        check!(!self.borrows[token_i].is_negative(), MangoErrorCode::MathError)?;
        check!(
            self.borrows[token_i].is_zero() || self.deposits[token_i].is_zero(),
            MangoErrorCode::MathError
        )
    }
    pub fn checked_add_deposit(&mut self, token_i: usize, v: I80F48) -> MangoResult<()> {
        self.deposits[token_i] = self.deposits[token_i].checked_add(v).ok_or(math_err!())?;
        check!(
            self.borrows[token_i].is_zero() || self.deposits[token_i].is_zero(),
            MangoErrorCode::MathError
        )
    }
    pub fn checked_sub_deposit(&mut self, token_i: usize, v: I80F48) -> MangoResult<()> {
        self.deposits[token_i] = self.deposits[token_i].checked_sub(v).ok_or(math_err!())?;
        check!(!self.deposits[token_i].is_negative(), MangoErrorCode::MathError)?;
        check!(
            self.borrows[token_i].is_zero() || self.deposits[token_i].is_zero(),
            MangoErrorCode::MathError
        )
    }

    fn get_net(&self, bank_cache: &RootBankCache, token_index: usize) -> I80F48 {
        if self.deposits[token_index].is_positive() {
            self.deposits[token_index].checked_mul(bank_cache.deposit_index).unwrap()
        } else if self.borrows[token_index].is_positive() {
            -self.borrows[token_index].checked_mul(bank_cache.borrow_index).unwrap()
        } else {
            ZERO_I80F48
        }
    }

    /// Return the token value and quote token value for this market taking into account open order
    /// but not doing asset weighting
    #[inline(always)]
    fn get_spot_val(
        &self,
        bank_cache: &RootBankCache,
        price: I80F48,
        market_index: usize,
        open_orders_ai: &AccountInfo,
    ) -> MangoResult<(I80F48, I80F48)> {
        let base_net = self.get_net(bank_cache, market_index);
        if !self.in_margin_basket[market_index] || *open_orders_ai.key == Pubkey::default() {
            Ok((base_net * price, ZERO_I80F48))
        } else {
            let open_orders = load_open_orders(open_orders_ai)?;
            let (quote_free, quote_locked, base_free, base_locked) =
                split_open_orders(&open_orders);

            // Simulate the health if all bids are executed at current price
            let bids_base_net: I80F48 = base_net + quote_locked / price + base_free + base_locked;
            let asks_base_net = base_net + base_free;

            if bids_base_net.abs() > asks_base_net.abs() {
                Ok((bids_base_net * price, quote_free))
            } else {
                Ok((asks_base_net * price, base_locked * price + quote_free + quote_locked))
            }
        }
    }

    /// Add a market to margin basket
    /// This function should be called any time you place a spot order
    pub fn add_to_basket(&mut self, market_index: usize) -> MangoResult<()> {
        if self.num_in_margin_basket == MAX_NUM_IN_MARGIN_BASKET {
            check!(self.in_margin_basket[market_index], MangoErrorCode::MarginBasketFull)
        } else {
            if !self.in_margin_basket[market_index] {
                self.in_margin_basket[market_index] = true;
                self.num_in_margin_basket += 1;
            }
            Ok(())
        }
    }

    /// Determine if margin basket should be updated.
    /// This function should be called any time you settle funds on serum dex
    pub fn update_basket(
        &mut self,
        market_index: usize,
        open_orders: &serum_dex::state::OpenOrders,
    ) -> MangoResult<()> {
        let is_empty = open_orders.native_pc_total == 0
            && open_orders.native_coin_total == 0
            && open_orders.referrer_rebates_accrued == 0;

        if self.in_margin_basket[market_index] && is_empty {
            self.in_margin_basket[market_index] = false;
            self.num_in_margin_basket -= 1;
        } else if !self.in_margin_basket[market_index] && !is_empty {
            check!(
                self.num_in_margin_basket < MAX_NUM_IN_MARGIN_BASKET,
                MangoErrorCode::MarginBasketFull
            )?;
            self.in_margin_basket[market_index] = true;
            self.num_in_margin_basket += 1;
        }
        Ok(())
    }

    /// Return true if account should enter bankruptcy.
    /// Note entering bankruptcy is calculated differently from exiting bankruptcy because of
    /// possible rounding issues and dust
    pub fn check_enter_bankruptcy(
        &self,
        mango_group: &MangoGroup,
        open_orders_ais: &[AccountInfo; MAX_PAIRS],
    ) -> bool {
        // TODO - consider using DUST_THRESHOLD
        // TODO - what if asset_weight == 0 for some asset? should it still be counted
        if self.deposits[QUOTE_INDEX] > DUST_THRESHOLD {
            return false;
        }

        for i in 0..mango_group.num_oracles {
            if self.deposits[i] > DUST_THRESHOLD {
                return false;
            }
            if open_orders_ais[i].key != &Pubkey::default() {
                let open_orders = load_open_orders(&open_orders_ais[i]).unwrap();
                if open_orders.native_pc_total > 0 || open_orders.native_coin_total > 0 {
                    return false;
                }
                let pa = &self.perp_accounts[i];
                // We know the bids and asks are empty to even be inside the liquidate function
                // So no need to check that
                // TODO - what if there's unsettled funding for a short position which turns this positive?
                if pa.quote_position.is_positive() || pa.base_position.is_positive() {
                    return false;
                }
            }
        }
        true
    }

    /// Return true if account should exit bankruptcy.
    /// An account can leave bankruptcy if all borrows are zero and all perp positions are non-negative
    /// Note entering bankruptcy is calculated differently from exiting bankruptcy because of
    /// possible rounding issues and dust
    pub fn check_exit_bankruptcy(&self, mango_group: &MangoGroup) -> bool {
        // TODO - consider if account above bankruptcy because assets have been boosted due to rounding
        //      Maybe replace these checks with DUST_THRESHOLD instead
        if self.borrows[QUOTE_INDEX] > DUST_THRESHOLD {
            return false;
        }

        for i in 0..mango_group.num_oracles {
            if self.perp_accounts[i].quote_position.is_negative()
                || self.perp_accounts[i].base_position.is_negative()
            {
                return false;
            }
        }
        true
    }

    pub fn check_open_orders(
        &self,
        mango_group: &MangoGroup,
        open_orders_ais: &[AccountInfo; MAX_PAIRS],
    ) -> MangoResult<()> {
        for i in 0..mango_group.num_oracles {
            if self.in_margin_basket[i] {
                check_eq!(
                    open_orders_ais[i].key,
                    &self.spot_open_orders[i],
                    MangoErrorCode::InvalidOpenOrdersAccount
                )?;
                check_open_orders(&open_orders_ais[i], &mango_group.signer_key)?;
            }
        }
        Ok(())
    }

    /// *** Below are methods related to the perps open orders ***
    pub fn next_order_slot(&self) -> Option<usize> {
        self.order_market.iter().position(|&i| i == FREE_ORDER_SLOT)
    }

    /// Add a perp order for the market_index
    pub fn add_order(
        &mut self,
        market_index: usize,
        side: Side,
        order: &LeafNode,
    ) -> MangoResult<()> {
        match side {
            Side::Bid => {
                // TODO make checked
                self.perp_accounts[market_index].bids_quantity += order.quantity;
            }
            Side::Ask => {
                self.perp_accounts[market_index].asks_quantity += order.quantity;
            }
        };
        let slot = order.owner_slot as usize;
        self.order_market[slot] = market_index as u8;
        self.order_side[slot] = side;
        self.orders[slot] = order.key;
        self.client_order_ids[slot] = order.client_order_id;
        Ok(())
    }

    ///
    pub fn remove_order(&mut self, slot: usize, quantity: i64) -> MangoResult<()> {
        check!(self.order_market[slot] != FREE_ORDER_SLOT, MangoErrorCode::Default)?;
        let market_index = self.order_market[slot] as usize;

        // accounting
        match self.order_side[slot] {
            Side::Bid => {
                self.perp_accounts[market_index].bids_quantity -= quantity;
            }
            Side::Ask => {
                self.perp_accounts[market_index].asks_quantity -= quantity;
            }
        }

        // release space
        self.order_market[slot] = FREE_ORDER_SLOT;

        // TODO OPT - remove these; unnecessary
        self.order_side[slot] = Side::Bid;
        self.orders[slot] = 0i128;
        self.client_order_ids[slot] = 0u64;
        Ok(())
    }

    pub fn execute_taker(
        &mut self,
        market_index: usize,
        perp_market: &mut PerpMarket,
        info: &PerpMarketInfo,
        cache: &PerpMarketCache,
        fill: &FillEvent,
    ) -> MangoResult<()> {
        let pa = &mut self.perp_accounts[market_index];
        pa.settle_funding(cache);
        let (base_change, quote_change) = fill.base_quote_change(fill.taker_side);
        pa.remove_taker_trade(base_change, quote_change);
        pa.change_base_position(perp_market, base_change);
        let quote = I80F48::from_num(perp_market.quote_lot_size * quote_change);
        let fees = quote.abs() * info.taker_fee;
        perp_market.fees_accrued += fees;
        pa.quote_position += quote - fees;
        Ok(())
    }

    pub fn execute_maker(
        &mut self,
        market_index: usize,
        perp_market: &mut PerpMarket,
        info: &PerpMarketInfo,
        cache: &PerpMarketCache,
        fill: &FillEvent,
    ) -> MangoResult<()> {
        let pa = &mut self.perp_accounts[market_index];
        pa.settle_funding(cache);

        let side = invert_side(fill.taker_side);
        let (base_change, quote_change) = fill.base_quote_change(side);
        pa.change_base_position(perp_market, base_change);
        let quote = I80F48::from_num(perp_market.quote_lot_size * quote_change);
        let fees = quote.abs() * info.maker_fee;
        perp_market.fees_accrued += fees;
        pa.quote_position += quote - fees;

        pa.apply_incentives(
            perp_market,
            side,
            fill.price,
            fill.best_initial,
            fill.price,
            fill.maker_timestamp,
            Clock::get()?.unix_timestamp as u64,
            fill.quantity,
        )?;

        if fill.maker_out {
            self.remove_order(fill.maker_slot as usize, base_change.abs())
        } else {
            match side {
                Side::Bid => {
                    pa.bids_quantity -= base_change.abs();
                }
                Side::Ask => {
                    pa.asks_quantity -= base_change.abs();
                }
            }
            Ok(())
        }
    }

    pub fn find_order_with_client_id(
        &self,
        market_index: usize,
        client_id: u64,
    ) -> Option<(i128, Side)> {
        let market_index = market_index as u8;
        for i in 0..MAX_PERP_OPEN_ORDERS {
            if self.order_market[i] == market_index && self.client_order_ids[i] == client_id {
                return Some((self.orders[i], self.order_side[i]));
            }
        }
        None
    }
    pub fn find_order_side(&self, market_index: usize, order_id: i128) -> Option<Side> {
        let market_index = market_index as u8;
        for i in 0..MAX_PERP_OPEN_ORDERS {
            if self.order_market[i] == market_index && self.orders[i] == order_id {
                return Some(self.order_side[i]);
            }
        }
        None
    }
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct PerpAccount {
    pub base_position: i64,     // measured in base lots
    pub quote_position: I80F48, // measured in native quote

    pub long_settled_funding: I80F48,
    pub short_settled_funding: I80F48,

    // orders related info
    pub bids_quantity: i64, // total contracts in sell orders
    pub asks_quantity: i64, // total quote currency in buy orders

    /// Amount that's on EventQueue waiting to be processed
    pub taker_base: i64,
    pub taker_quote: i64,

    pub mngo_accrued: u64,
}

impl PerpAccount {
    /// Add taker trade after it has been matched but before it has been process on EventQueue
    pub fn add_taker_trade(&mut self, base_change: i64, quote_change: i64) {
        // TODO make checked? estimate chances of overflow here
        self.taker_base += base_change;
        self.taker_quote += quote_change;
    }
    /// Remove taker trade after it has been processed on EventQueue
    pub fn remove_taker_trade(&mut self, base_change: i64, quote_change: i64) {
        self.taker_base -= base_change;
        self.taker_quote -= quote_change;
    }

    pub fn apply_incentives(
        &mut self,
        perp_market: &mut PerpMarket,

        side: Side,
        price: i64,
        best_initial: i64,
        best_final: i64,
        time_initial: u64,
        time_final: u64,
        quantity: i64,
    ) -> MangoResult<()> {
        let lmi = &mut perp_market.liquidity_mining_info;
        if lmi.rate == 0 || lmi.mngo_per_period == 0 {
            return Ok(());
        }

        let best = match side {
            Side::Bid => max(best_initial, best_final),
            Side::Ask => min(best_initial, best_final),
        };

        // TODO limit incentives to orders that were on book at least 5 seconds
        let dist_bps = I80F48::from_num((best - price).abs() * 10_000) / I80F48::from_num(best);
        let dist_factor: I80F48 = max(lmi.max_depth_bps - dist_bps, ZERO_I80F48);

        // TODO - check overflow possibilities here by throwing in reasonable large numbers
        let mut points = dist_factor
            .checked_mul(dist_factor)
            .unwrap()
            .checked_mul(I80F48::from_num(time_final - time_initial))
            .unwrap()
            .checked_mul(I80F48::from_num(quantity))
            .unwrap();

        // TODO OPT remove this sanity check if confident
        check!(!points.is_negative(), MangoErrorCode::MathError)?;

        let points_in_period = I80F48::from_num(lmi.mngo_left).checked_div(lmi.rate).unwrap();

        if points >= points_in_period {
            sol_log_compute_units();

            self.mngo_accrued += lmi.mngo_left;
            points -= points_in_period;

            let rate_adj = I80F48::from_num(time_final - lmi.period_start)
                .checked_div(I80F48::from_num(lmi.target_period_length))
                .unwrap()
                .clamp(MIN_RATE_ADJ, MAX_RATE_ADJ);

            lmi.rate = lmi.rate.checked_mul(rate_adj).unwrap();
            lmi.period_start = time_final;
            lmi.mngo_left = lmi.mngo_per_period;

            sol_log_compute_units(); // To figure out how much rate adjust costs
        }

        let mngo_earned =
            points.checked_mul(lmi.rate).unwrap().to_num::<u64>().min(lmi.mngo_per_period); // limit mngo payout to max mngo in a period

        self.mngo_accrued += mngo_earned;
        lmi.mngo_left -= mngo_earned;

        Ok(())
    }

    /// This assumes settle_funding was already called
    pub fn change_base_position(&mut self, perp_market: &mut PerpMarket, base_change: i64) {
        let start = self.base_position;
        self.base_position += base_change;
        perp_market.open_interest += self.base_position.abs() - start.abs();
    }

    /// Move unrealized funding payments into the quote_position
    pub fn settle_funding(&mut self, cache: &PerpMarketCache) {
        if self.base_position > 0 {
            self.quote_position -= (cache.long_funding - self.long_settled_funding)
                * I80F48::from_num(self.base_position);
        } else if self.base_position < 0 {
            self.quote_position -= (cache.short_funding - self.short_settled_funding)
                * I80F48::from_num(self.base_position);
        }
        self.long_settled_funding = cache.long_funding;
        self.short_settled_funding = cache.short_funding;
    }

    /// Get quote position adjusted for funding
    pub fn get_quote_position(&self, pmc: &PerpMarketCache) -> I80F48 {
        if self.base_position > 0 {
            // TODO OPT use checked_fmul to not do the mul if one of these is zero
            self.quote_position
                - (pmc.long_funding - self.long_settled_funding)
                    * I80F48::from_num(self.base_position)
        } else if self.base_position < 0 {
            self.quote_position
                - (pmc.short_funding - self.short_settled_funding)
                    * I80F48::from_num(self.base_position)
        } else {
            self.quote_position
        }
    }

    /// Return (base_val, quote_val) unweighted
    pub fn get_val(
        &self,
        pmi: &PerpMarketInfo,
        pmc: &PerpMarketCache,
        price: I80F48,
    ) -> MangoResult<(I80F48, I80F48)> {
        let bids_base_net = self.base_position + self.taker_base + self.bids_quantity;
        let asks_base_net = self.base_position + self.taker_base - self.asks_quantity;
        if bids_base_net.abs() > asks_base_net.abs() {
            let base = I80F48::from_num(bids_base_net * pmi.base_lot_size) * price;
            let quote = self.get_quote_position(pmc)
                + I80F48::from_num(self.taker_quote * pmi.quote_lot_size)
                - I80F48::from_num(self.bids_quantity * pmi.base_lot_size) * price;
            Ok((base, quote))
        } else {
            let base = I80F48::from_num(asks_base_net * pmi.base_lot_size) * price;
            let quote = self.get_quote_position(pmc)
                + I80F48::from_num(self.taker_quote * pmi.quote_lot_size)
                + I80F48::from_num(self.asks_quantity * pmi.base_lot_size) * price;
            Ok((base, quote))
        }
    }

    pub fn is_active(&self) -> bool {
        self.base_position != 0
            || !self.quote_position.is_zero()
            || self.bids_quantity != 0
            || self.asks_quantity != 0
            || self.taker_base != 0
            || self.taker_quote != 0

        // Note funding only applies if base position not 0
    }

    /// Decrement self and increment other
    pub fn transfer_quote_position(&mut self, other: &mut PerpAccount, quantity: I80F48) {
        self.quote_position -= quantity;
        other.quote_position += quantity;
    }

    /// All orders must be canceled and there must be no unprocessed FillEvents for this PerpAccount
    pub fn is_liquidatable(&self) -> bool {
        self.bids_quantity == 0
            && self.asks_quantity == 0
            && self.taker_quote == 0
            && self.taker_base == 0
    }
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
/// Information regarding market maker incentives for a perp market
pub struct LiquidityMiningInfo {
    /// Used to convert liquidity points to MNGO
    pub rate: I80F48,

    pub max_depth_bps: I80F48,

    /// start timestamp of current liquidity incentive period; gets updated when mngo_left goes to 0
    pub period_start: u64,

    /// Target time length of a period in seconds
    pub target_period_length: u64,

    /// Paper MNGO left for this period
    pub mngo_left: u64,

    /// Total amount of MNGO allocated for current period
    pub mngo_per_period: u64,
}

/// This will hold top level info about the perps market
/// Likely all perps transactions on a market will be locked on this one because this will be passed in as writable
#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct PerpMarket {
    pub meta_data: MetaData,

    pub mango_group: Pubkey,
    pub bids: Pubkey,
    pub asks: Pubkey,
    pub event_queue: Pubkey,
    pub quote_lot_size: i64, // number of quote native that reresents min tick
    pub base_lot_size: i64,  // represents number of base native quantity; greater than 0

    // TODO - consider just moving this into the cache
    pub long_funding: I80F48,
    pub short_funding: I80F48,

    pub open_interest: i64, // This is i64 to keep consistent with the units of contracts, but should always be > 0

    pub last_updated: u64,
    pub seq_num: u64,
    pub fees_accrued: I80F48, // native quote currency

    pub liquidity_mining_info: LiquidityMiningInfo,

    // mngo_vault holds mango tokens to be disbursed as liquidity incentives for this perp market
    pub mngo_vault: Pubkey,
}

impl PerpMarket {
    pub fn load_and_init<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        mango_group_ai: &'a AccountInfo,
        bids_ai: &'a AccountInfo,
        asks_ai: &'a AccountInfo,
        event_queue_ai: &'a AccountInfo,
        mngo_vault_ai: &'a AccountInfo,
        mango_group: &MangoGroup,
        rent: &Rent,
        base_lot_size: i64,
        quote_lot_size: i64,
        rate: I80F48,
        max_depth_bps: I80F48,
        target_period_length: u64,
        mngo_per_period: u64,
    ) -> MangoResult<RefMut<'a, Self>> {
        let mut state = Self::load_mut(account)?;
        check!(account.owner == program_id, MangoErrorCode::InvalidOwner)?;
        check!(
            rent.is_exempt(account.lamports(), size_of::<Self>()),
            MangoErrorCode::AccountNotRentExempt
        )?;
        check!(!state.meta_data.is_initialized, MangoErrorCode::Default)?;

        state.meta_data = MetaData::new(DataType::PerpMarket, 0, true);
        state.mango_group = *mango_group_ai.key;
        state.bids = *bids_ai.key;
        state.asks = *asks_ai.key;
        state.event_queue = *event_queue_ai.key;
        state.quote_lot_size = quote_lot_size;
        state.base_lot_size = base_lot_size;

        let vault = Account::unpack(&mngo_vault_ai.try_borrow_data()?)?;
        check!(vault.owner == mango_group.signer_key, MangoErrorCode::InvalidOwner)?;
        check!(vault.mint == mngo_token::ID, MangoErrorCode::InvalidVault)?;
        check!(mngo_vault_ai.owner == &spl_token::ID, MangoErrorCode::InvalidOwner)?;
        state.mngo_vault = *mngo_vault_ai.key;

        let clock = Clock::get()?;
        let period_start = clock.unix_timestamp as u64;
        state.last_updated = period_start;

        state.liquidity_mining_info = LiquidityMiningInfo {
            rate,
            max_depth_bps,
            period_start,
            target_period_length,
            mngo_left: mngo_per_period,
            mngo_per_period,
        };

        Ok(state)
    }

    pub fn load_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        mango_group_pk: &Pubkey,
    ) -> MangoResult<Ref<'a, Self>> {
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;
        let state = Self::load(account)?;
        check!(state.meta_data.is_initialized, MangoErrorCode::Default)?;
        check!(state.meta_data.data_type == DataType::PerpMarket as u8, MangoErrorCode::Default)?;
        check!(mango_group_pk == &state.mango_group, MangoErrorCode::Default)?;
        Ok(state)
    }

    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        mango_group_pk: &Pubkey,
    ) -> MangoResult<RefMut<'a, Self>> {
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;
        let state = Self::load_mut(account)?;
        check!(state.meta_data.is_initialized, MangoErrorCode::Default)?;
        check!(state.meta_data.data_type == DataType::PerpMarket as u8, MangoErrorCode::Default)?;
        check!(mango_group_pk == &state.mango_group, MangoErrorCode::Default)?;
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

    /// Use current order book price and index price to update the instantaneous funding
    pub fn update_funding(
        &mut self,
        mango_group: &MangoGroup,
        book: &Book,
        mango_cache: &MangoCache,
        market_index: usize,
        now_ts: u64,
    ) -> MangoResult<()> {
        // Get the index price from cache, ensure it's not outdated
        let price_cache = &mango_cache.price_cache[market_index];
        check!(
            now_ts <= price_cache.last_update + mango_group.valid_interval,
            MangoErrorCode::InvalidCache
        )?;
        let index_price = price_cache.price;

        // Get current book price & compare it to index price

        // TODO get impact bid and impact ask if compute allows
        // TODO consider corner cases of funding being updated
        let bid = book.get_best_bid_price();
        let ask = book.get_best_ask_price();

        const MAX_FUNDING: I80F48 = I80F48!(0.05);
        const MIN_FUNDING: I80F48 = I80F48!(-0.05);

        let diff = match (bid, ask) {
            (Some(bid), Some(ask)) => {
                // calculate mid-market rate
                let book_price = self.lot_to_native_price((bid + ask) / 2);
                (book_price / index_price - ONE_I80F48).clamp(MIN_FUNDING, MAX_FUNDING)
            }
            (Some(_bid), None) => MAX_FUNDING,
            (None, Some(_ask)) => MIN_FUNDING,
            (None, None) => ZERO_I80F48,
        };

        // TODO TEST consider what happens if time_factor is very small. Can funding_delta == 0 when diff != 0?
        let time_factor = I80F48::from_num(now_ts - self.last_updated) / DAY;
        let funding_delta: I80F48 = index_price
            .checked_mul(diff)
            .unwrap()
            .checked_mul(I80F48::from_num(self.base_lot_size))
            .unwrap()
            .checked_mul(time_factor)
            .unwrap();

        self.long_funding += funding_delta;
        self.short_funding += funding_delta;
        self.last_updated = now_ts;

        // Check if liquidity incentives ought to be paid out and if so pay them out
        Ok(())
    }

    /// Convert from the price stored on the book to the price used in value calculations
    pub fn lot_to_native_price(&self, price: i64) -> I80F48 {
        I80F48::from_num(price)
            .checked_mul(I80F48::from_num(self.quote_lot_size))
            .unwrap()
            .checked_div(I80F48::from_num(self.base_lot_size))
            .unwrap()
    }

    /// Socialize the loss in this account across all longs and shorts
    pub fn socialize_loss(
        &mut self,
        account: &mut PerpAccount,
        cache: &mut PerpMarketCache,
    ) -> MangoResult<()> {
        // TODO convert into only socializing on one side
        // native USDC per contract open interest
        let socialized_loss = account.quote_position / (I80F48::from_num(self.open_interest));
        account.quote_position = ZERO_I80F48;
        self.long_funding -= socialized_loss;
        self.short_funding += socialized_loss;

        cache.short_funding = self.short_funding;
        cache.long_funding = self.long_funding;

        Ok(())
    }
}

pub fn load_market_state<'a>(
    market_account: &'a AccountInfo,
    program_id: &Pubkey,
) -> MangoResult<RefMut<'a, serum_dex::state::MarketState>> {
    check_eq!(market_account.owner, program_id, MangoErrorCode::Default)?;

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

fn strip_dex_padding<'a>(acc: &'a AccountInfo) -> MangoResult<Ref<'a, [u8]>> {
    check!(acc.data_len() >= 12, MangoErrorCode::Default)?;
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

pub fn check_open_orders(acc: &AccountInfo, owner: &Pubkey) -> MangoResult<()> {
    if *acc.key == Pubkey::default() {
        return Ok(());
    }
    // if it's not default, it must be initialized
    let open_orders = load_open_orders(acc)?;
    let valid_flags = (serum_dex::state::AccountFlag::Initialized
        | serum_dex::state::AccountFlag::OpenOrders)
        .bits();
    check_eq!(open_orders.account_flags, valid_flags, MangoErrorCode::Default)?;
    check_eq!(identity(open_orders.owner), owner.to_aligned_bytes(), MangoErrorCode::Default)?;

    Ok(())
}

fn strip_dex_padding_mut<'a>(acc: &'a AccountInfo) -> MangoResult<RefMut<'a, [u8]>> {
    check!(acc.data_len() >= 12, MangoErrorCode::Default)?;
    let unpadded_data: RefMut<[u8]> = RefMut::map(acc.try_borrow_mut_data()?, |data| {
        let data_len = data.len() - 12;
        let (_, rest) = data.split_at_mut(5);
        let (mid, _) = rest.split_at_mut(data_len);
        mid
    });
    Ok(unpadded_data)
}

fn strip_data_header_mut<'a, H: Pod, D: Pod>(
    orig_data: RefMut<'a, [u8]>,
) -> MangoResult<(RefMut<'a, H>, RefMut<'a, [D]>)> {
    let (header, inner): (RefMut<'a, [H]>, RefMut<'a, [D]>) =
        RefMut::map_split(orig_data, |data| {
            let (header_bytes, inner_bytes) = data.split_at_mut(size_of::<H>());
            let header: &mut H;
            let inner: &mut [D];
            header = try_from_bytes_mut(header_bytes).unwrap();
            inner = remove_slop_mut(inner_bytes);
            (std::slice::from_mut(header), inner)
        });
    let header = RefMut::map(header, |s| s.first_mut().unwrap_or_else(|| unreachable!()));
    Ok((header, inner))
}

pub fn load_bids_mut<'a>(
    sm: &RefMut<serum_dex::state::MarketState>,
    bids: &'a AccountInfo,
) -> MangoResult<RefMut<'a, serum_dex::critbit::Slab>> {
    check_eq!(&bids.key.to_aligned_bytes(), &identity(sm.bids), MangoErrorCode::Default)?;

    let orig_data = strip_dex_padding_mut(bids)?;
    let (header, buf) = strip_data_header_mut::<OrderBookStateHeader, u8>(orig_data)?;
    let flags = BitFlags::from_bits(header.account_flags).unwrap();
    check_eq!(
        &flags,
        &(serum_dex::state::AccountFlag::Initialized | serum_dex::state::AccountFlag::Bids),
        MangoErrorCode::Default
    )?;
    Ok(RefMut::map(buf, serum_dex::critbit::Slab::new))
}

pub fn load_asks_mut<'a>(
    sm: &RefMut<serum_dex::state::MarketState>,
    asks: &'a AccountInfo,
) -> MangoResult<RefMut<'a, serum_dex::critbit::Slab>> {
    check_eq!(&asks.key.to_aligned_bytes(), &identity(sm.asks), MangoErrorCode::Default)?;
    let orig_data = strip_dex_padding_mut(asks)?;
    let (header, buf) = strip_data_header_mut::<OrderBookStateHeader, u8>(orig_data)?;
    let flags = BitFlags::from_bits(header.account_flags).unwrap();
    check_eq!(
        &flags,
        &(serum_dex::state::AccountFlag::Initialized | serum_dex::state::AccountFlag::Asks),
        MangoErrorCode::Default
    )?;
    Ok(RefMut::map(buf, serum_dex::critbit::Slab::new))
}

/// Copied over from serum dex
#[derive(Copy, Clone)]
#[repr(packed)]
pub struct OrderBookStateHeader {
    pub account_flags: u64, // Initialized, (Bids or Asks)
}
unsafe impl Zeroable for OrderBookStateHeader {}
unsafe impl Pod for OrderBookStateHeader {}
