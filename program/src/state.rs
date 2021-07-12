use std::cell::{Ref, RefMut};
use std::convert::identity;
use std::mem::size_of;
use std::num::NonZeroU64;

use bytemuck::{from_bytes, from_bytes_mut, try_from_bytes_mut, Pod, Zeroable};
use fixed::types::I80F48;
use fixed_macro::types::I80F48;
use mango_common::Loadable;
use mango_macro::{Loadable, Pod};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{Deserialize, Serialize};
use serum_dex::state::ToAlignedBytes;
use solana_program::account_info::AccountInfo;
use solana_program::msg;
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;
use solana_program::sysvar::{clock::Clock, rent::Rent, Sysvar};

use crate::error::{check_assert, MangoError, MangoErrorCode, MangoResult, SourceFileId};
use crate::ids::mngo_token;
use crate::matching::{Book, LeafNode, Side};
use crate::queue::FillEvent;
use crate::utils::{invert_side, remove_slop_mut};
use enumflags2::BitFlags;
use solana_program::program_pack::Pack;
use spl_token::state::Account;
use std::cmp::{max, min};

pub const MAX_TOKENS: usize = 32;
pub const MAX_PAIRS: usize = MAX_TOKENS - 1;
pub const MAX_NODE_BANKS: usize = 8;
pub const QUOTE_INDEX: usize = MAX_TOKENS - 1;
pub const ZERO_I80F48: I80F48 = I80F48!(0);
pub const ONE_I80F48: I80F48 = I80F48!(1);
pub const DAY: I80F48 = I80F48!(86400);
pub const YEAR: I80F48 = I80F48!(31536000);

pub const DUST_THRESHOLD: I80F48 = I80F48!(1); // TODO make this part of MangoGroup state

declare_check_assert_macros!(SourceFileId::State);

// TODO: all unit numbers are just place holders. make decisions on each unit number
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

    // DAO vault is funded by the Mango DAO with USDC and can be withdrawn by the DAO
    pub dao_vault: Pubkey,
    pub srm_vault: Pubkey,
    pub msrm_vault: Pubkey,
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
    ) -> MangoResult<()> {
        let mut native_deposits = ZERO_I80F48;
        let mut native_borrows = ZERO_I80F48;

        let mut max_node_bank_index = 0;
        let mut max_node_bank_borrows = ZERO_I80F48;
        for i in 0..self.num_node_banks {
            check!(node_bank_ais[i].key == &self.node_banks[i], MangoErrorCode::InvalidNodeBank)?;
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

        let mut node_bank =
            NodeBank::load_mut_checked(&node_bank_ais[max_node_bank_index], program_id)?;

        bankrupt_account.checked_sub_borrow(token_index, loss)?;
        node_bank.checked_sub_borrow(loss)?;

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

pub const MAX_NUM_IN_MARGIN_BASKET: u8 = 10;
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

    pub msrm_amount: u64,
    /// This account cannot open new positions or borrow until `init_health >= 0`
    pub being_liquidated: bool,

    /// This account cannot do anything except go through `resolve_bankruptcy`
    pub is_bankrupt: bool,
    pub padding: [u8; 6],
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

    pub fn get_token_assets_liabs(
        &self,
        mango_group: &MangoGroup,
        mango_cache: &MangoCache,
        open_orders_ais: &[AccountInfo],
    ) -> MangoResult<([I80F48; MAX_TOKENS], [I80F48; MAX_TOKENS])> {
        let mut assets = [ZERO_I80F48; MAX_TOKENS];
        let mut liabs = [ZERO_I80F48; MAX_TOKENS];

        assets[QUOTE_INDEX] =
            mango_cache.root_bank_cache[QUOTE_INDEX].deposit_index * self.deposits[QUOTE_INDEX];
        liabs[QUOTE_INDEX] =
            mango_cache.root_bank_cache[QUOTE_INDEX].borrow_index * self.borrows[QUOTE_INDEX];

        for i in 0..mango_group.num_oracles {
            if !self.in_margin_basket[i] || self.spot_open_orders[i] == Pubkey::default() {
                assets[i] = mango_cache.root_bank_cache[i].deposit_index * self.deposits[i];
            } else {
                // TODO make sure open orders account is checked for validity before passing in here
                let open_orders = load_open_orders(&open_orders_ais[i])?;
                assets[i] = mango_cache.root_bank_cache[i].deposit_index * self.deposits[i]
                    + I80F48::from_num(open_orders.native_coin_total);
                assets[QUOTE_INDEX] += I80F48::from_num(
                    open_orders.native_pc_total + open_orders.referrer_rebates_accrued,
                );
            }

            liabs[i] = mango_cache.root_bank_cache[i].borrow_index * self.borrows[i];
        }

        Ok((assets, liabs))
    }

    // TODO conform to new way of determining spot health
    pub fn get_spot_val(
        &self,
        mango_cache: &MangoCache,
        market_index: usize,
        open_orders_ai: &AccountInfo,
        asset_weight: I80F48,
    ) -> MangoResult<I80F48> {
        // TODO make checked
        let bank_cache = &mango_cache.root_bank_cache[market_index];
        let price = mango_cache.price_cache[market_index].price;

        Ok(
            if !self.in_margin_basket[market_index]
                || self.spot_open_orders[market_index] == Pubkey::default()
            {
                self.deposits[market_index] * bank_cache.deposit_index * asset_weight * price
            } else {
                // TODO - confirm only checked open orders are sent in here
                let open_orders = load_open_orders(open_orders_ai)?;

                ((self.deposits[market_index] * bank_cache.deposit_index
                    + I80F48::from_num(open_orders.native_coin_total))
                    * asset_weight
                    * price)
                    + I80F48::from_num(
                        open_orders.native_pc_total + open_orders.referrer_rebates_accrued,
                    )
            },
        )
    }

    pub fn get_assets_val(
        &self,
        mango_group: &MangoGroup,
        mango_cache: &MangoCache,
        open_orders_ais: &[AccountInfo],
        active_assets: &[bool; MAX_TOKENS],
        health_type: HealthType,
    ) -> MangoResult<I80F48> {
        let mut assets_val =
            mango_cache.root_bank_cache[QUOTE_INDEX].deposit_index * self.deposits[QUOTE_INDEX];

        for i in 0..mango_group.num_oracles {
            if !active_assets[i] {
                continue;
            }
            let spot_market_info = &mango_group.spot_markets[i];
            let perp_market_info = &mango_group.perp_markets[i];

            let (spot_weight, perp_weight) = match health_type {
                HealthType::Maint => {
                    (spot_market_info.maint_asset_weight, perp_market_info.maint_asset_weight)
                }
                HealthType::Init => {
                    (spot_market_info.init_asset_weight, perp_market_info.init_asset_weight)
                }
            };

            if !spot_market_info.is_empty() {
                assets_val +=
                    self.get_spot_val(mango_cache, i, &open_orders_ais[i], spot_weight)?;
            }

            if !perp_market_info.is_empty() {
                assets_val += self.perp_accounts[i].get_assets_val(
                    mango_cache.price_cache[i].price,
                    perp_weight,
                    mango_cache.perp_market_cache[i].long_funding,
                    mango_cache.perp_market_cache[i].short_funding,
                );
            }
        }

        Ok(assets_val)
    }
    fn get_spot_health(
        &self,
        mango_cache: &MangoCache,
        market_index: usize,
        open_orders_ai: &AccountInfo,
        asset_weight: I80F48,
        liab_weight: I80F48,
    ) -> MangoResult<I80F48> {
        // TODO make checked
        let bank_cache = &mango_cache.root_bank_cache[market_index];
        let price = mango_cache.price_cache[market_index].price;

        // Note, only one of deposits and borrows are positive
        let base_net: I80F48 = self.deposits[market_index] * bank_cache.deposit_index
            - self.borrows[market_index] * bank_cache.borrow_index;

        let health = if !self.in_margin_basket[market_index]
            || self.spot_open_orders[market_index] == Pubkey::default()
        {
            if base_net.is_positive() {
                base_net * asset_weight * price
            } else {
                base_net * liab_weight * price
            }
        } else {
            let open_orders = load_open_orders(open_orders_ai)?;

            let quote_free =
                I80F48::from_num(open_orders.native_pc_free + open_orders.referrer_rebates_accrued);
            let quote_locked =
                I80F48::from_num(open_orders.native_pc_total - open_orders.native_pc_free);
            let base_free = I80F48::from_num(open_orders.native_coin_free);
            let base_locked =
                I80F48::from_num(open_orders.native_coin_total - open_orders.native_coin_free);

            // TODO account for serum dex fees in these calculations

            // Simulate the health if all bids are executed at current price
            let bids_base_net: I80F48 = base_net + quote_locked / price + base_free + base_locked;
            let bids_weight = if bids_base_net.is_positive() { asset_weight } else { liab_weight };
            let bids_health: I80F48 = bids_base_net * bids_weight * price + quote_free;

            // Simulate health if all asks are executed at current price
            let asks_base_net = base_net - base_locked + base_free;
            let asks_weight = if asks_base_net.is_positive() { asset_weight } else { liab_weight };
            let asks_health: I80F48 = asks_base_net * asks_weight * price
                + price * base_locked
                + quote_free
                + quote_locked;

            // Pick the worse of the two possibilities
            bids_health.min(asks_health)
        };

        msg!("spot health {:?}", health);

        Ok(health)
    }

    /// Must validate the caches before
    #[inline(never)]
    pub fn get_health(
        &self,
        mango_group: &MangoGroup,
        mango_cache: &MangoCache,
        spot_open_orders_ais: &[AccountInfo],
        active_assets: &[bool; MAX_TOKENS],
        health_type: HealthType,
    ) -> MangoResult<I80F48> {
        let mut health = (mango_cache.root_bank_cache[QUOTE_INDEX].deposit_index
            * self.deposits[QUOTE_INDEX])
            - (mango_cache.root_bank_cache[QUOTE_INDEX].borrow_index * self.borrows[QUOTE_INDEX]);

        for i in 0..mango_group.num_oracles {
            if !active_assets[i] {
                continue;
            }

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
            if !spot_market_info.is_empty() {
                health += self.get_spot_health(
                    mango_cache,
                    i,
                    &spot_open_orders_ais[i],
                    spot_asset_weight,
                    spot_liab_weight,
                )?;
            }

            if !perp_market_info.is_empty() {
                health += self.perp_accounts[i].get_health(
                    perp_market_info,
                    mango_cache.price_cache[i].price,
                    perp_asset_weight,
                    perp_liab_weight,
                    mango_cache.perp_market_cache[i].long_funding,
                    mango_cache.perp_market_cache[i].short_funding,
                );
            }
            msg!("get_health {} => {:?}", i, health);
        }

        Ok(health)
    }

    /// Return an array of bools that are true if any part of token, spot or perps for that index are nonzero
    pub fn get_active_assets(&self, mango_group: &MangoGroup) -> [bool; MAX_TOKENS] {
        let mut active_assets = [false; MAX_TOKENS];
        active_assets[QUOTE_INDEX] = true;
        for i in 0..mango_group.num_oracles {
            active_assets[i] = self.in_margin_basket[i]
                || !self.deposits[i].is_zero()
                || !self.borrows[i].is_zero()
                || self.perp_accounts[i].is_active();
        }
        active_assets
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

    #[allow(dead_code)]
    /// Determine the bankruptcy status of the account
    pub fn get_bankruptcy(&self) -> MangoResult<bool> {
        unimplemented!()
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

    pub fn remove_order(&mut self, side: Side, slot: u8, quantity: i64) -> MangoResult<()> {
        let slot_mask = 1u32 << slot;
        // TODO OPT - remove this check if we're confident
        check_eq!(Some(side), self.slot_side(slot), MangoErrorCode::Default)?;

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
    pub fn add_order(&mut self, side: Side, order: &LeafNode) -> MangoResult<()> {
        check!(self.is_free_bits != 0, MangoErrorCode::TooManyOpenOrders)?;
        let slot = self.next_order_slot();
        let slot_mask = 1u32 << slot;
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

        self.is_free_bits &= !slot_mask;
        self.orders[slot as usize] = order.key;
        self.client_order_ids[slot as usize] = order.client_order_id;
        Ok(())
    }

    pub fn cancel_order(
        &mut self,
        order: &LeafNode,
        order_id: i128,
        side: Side,
    ) -> MangoResult<()> {
        // input verification
        let slot = order.owner_slot;
        let slot_mask = 1u32 << slot;
        check_eq!(0u32, slot_mask & self.is_free_bits, MangoErrorCode::Default)?;
        check_eq!(Some(side), self.slot_side(slot), MangoErrorCode::Default)?;
        check_eq!(order_id, self.orders[slot as usize], MangoErrorCode::Default)?;

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
    pub liquidity_points: I80F48,
}

impl PerpAccount {
    pub fn execute_taker(
        &mut self,
        perp_market: &mut PerpMarket,
        info: &PerpMarketInfo,
        fill: &FillEvent,
    ) -> MangoResult<()> {
        let (base_change, quote_change) = fill.base_quote_change(fill.side);
        self.change_base_position(perp_market, base_change);
        let quote = I80F48::from_num(perp_market.quote_lot_size * quote_change);
        let fees = quote.abs() * info.taker_fee;
        perp_market.fees_accrued += fees;
        self.quote_position += quote - fees;
        Ok(())
    }

    pub fn execute_maker(
        &mut self,
        perp_market: &mut PerpMarket,
        info: &PerpMarketInfo,
        fill: &FillEvent,
    ) -> MangoResult<()> {
        let side = invert_side(fill.side);
        let (base_change, quote_change) = fill.base_quote_change(side);
        self.change_base_position(perp_market, base_change);
        let quote = I80F48::from_num(perp_market.quote_lot_size * quote_change);
        let fees = quote.abs() * info.taker_fee;
        perp_market.fees_accrued += fees;
        self.quote_position += quote - fees;

        self.apply_incentives(
            perp_market,
            side,
            fill.price,
            fill.best_initial,
            fill.price,
            fill.timestamp,
            Clock::get()?.unix_timestamp as u64,
            fill.quantity,
        )?;

        if fill.maker_out {
            self.open_orders.remove_order(side, fill.maker_slot, base_change.abs())
        } else {
            match side {
                Side::Bid => {
                    self.open_orders.bids_quantity -= base_change.abs();
                }
                Side::Ask => {
                    self.open_orders.asks_quantity -= base_change.abs();
                }
            }
            Ok(())
        }
    }

    pub fn apply_incentives(
        &mut self,
        perp_market: &PerpMarket,

        side: Side,
        price: i64,
        best_initial: i64,
        best_final: i64,
        time_initial: u64,
        time_final: u64,
        quantity: i64,
    ) -> MangoResult<()> {
        let best = match side {
            Side::Bid => max(best_initial, best_final),
            Side::Ask => min(best_initial, best_final),
        };

        let dist_bps = I80F48::from_num((best - price).abs() * 10_000) / I80F48::from_num(best);
        let dist_factor = max(perp_market.max_depth_bps - dist_bps, ZERO_I80F48);

        // TODO - check overflow possibilities here by throwing in reasonable large numbers
        let points = dist_factor
            * dist_factor
            * I80F48::from_num(time_final - time_initial)
            * I80F48::from_num(quantity)
            * perp_market.scaler;

        // TODO OPT remove this sanity check if confident
        check!(!points.is_negative(), MangoErrorCode::Default)?;

        // Not necessary for now, but may come in useful later
        // perp_market.total_liquidity_points += points;

        self.liquidity_points += points;

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
        // TODO account for fees
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

        // Account for unrealized funding payments
        // TODO make checked
        // TODO - consider force moving funding into the realized at start of every instruction
        let x = if self.base_position > 0 {
            h - (long_funding - self.long_settled_funding) * I80F48::from_num(self.base_position)
        } else {
            h + (short_funding - self.short_settled_funding) * I80F48::from_num(self.base_position)
        };
        msg!("perp health {:?}", x);
        x
    }

    /// Returns value of assets, excludes open orders
    pub fn get_assets_val(
        &self,
        price: I80F48,
        asset_weight: I80F48,
        long_funding: I80F48,
        short_funding: I80F48,
    ) -> I80F48 {
        let base_position = I80F48::from_num(self.base_position);
        if self.base_position > ZERO_I80F48 {
            let quote_position =
                self.quote_position - (long_funding - self.long_settled_funding) * base_position;

            if quote_position > ZERO_I80F48 {
                base_position * asset_weight * price + quote_position
            } else {
                base_position * asset_weight * price
            }
        } else {
            let quote_position =
                self.quote_position + (short_funding - self.short_settled_funding) * base_position;

            if quote_position > ZERO_I80F48 {
                quote_position
            } else {
                ZERO_I80F48
            }
        }
    }

    pub fn is_active(&self) -> bool {
        self.base_position != 0
            || !self.quote_position.is_zero()
            || self.open_orders.bids_quantity != 0
            || self.open_orders.asks_quantity != 0

        // Note funding only applies if base position not 0
    }

    /// Decrement self and increment other
    pub fn transfer_quote_position(&mut self, other: &mut PerpAccount, quantity: I80F48) {
        self.quote_position -= quantity;
        other.quote_position += quantity;
    }
}

/// This will hold top level info about the perps market
/// Likely all perps transactions on a market will be locked on this one because this will be passed in as writable
#[derive(Copy, Clone, Pod, Loadable, Default)]
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

    // Liquidity incentive params
    pub max_depth_bps: I80F48,
    pub scaler: I80F48,
    pub total_liquidity_points: I80F48,
    pub points_per_mngo: I80F48, // how many points equal 1 native MNGO

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
        state.last_updated = clock.unix_timestamp as u64;

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
                min(max((book_price / index_price) - ONE_I80F48, MIN_FUNDING), MAX_FUNDING)
            }
            (Some(_bid), None) => MAX_FUNDING,
            (None, Some(_ask)) => MIN_FUNDING,
            (None, None) => ZERO_I80F48,
        };

        // TODO consider what happens if time_factor is very small. Can funding_delta == 0 when diff != 0?
        let time_factor = I80F48::from_num(now_ts - self.last_updated) / DAY;
        let funding_delta: I80F48 = diff
            * time_factor
            * I80F48::from_num(self.base_lot_size)  // TODO check cost of conversion op
            * index_price;

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
        // native USDC per contract open interest
        let socialized_loss = account.quote_position / (I80F48::from_num(self.open_interest));
        account.quote_position = ZERO_I80F48;
        self.long_funding -= socialized_loss;
        self.short_funding += socialized_loss;

        cache.short_funding = self.short_funding;
        cache.long_funding = self.long_funding;

        Ok(())
    }

    /// Execute a trade between the maker and taker accounts
    /// base_change is from the taker's perspective; maker's base_change will be -base_change
    pub fn execute_trade(
        &mut self,
        cache: &PerpMarketCache,
        info: &PerpMarketInfo,
        fill: &FillEvent,
        maker: &mut PerpAccount,
        taker: &mut PerpAccount,
    ) -> MangoResult<()> {
        maker.settle_funding(cache);
        taker.settle_funding(cache);

        taker.execute_taker(self, info, fill)?;
        maker.execute_maker(self, info, fill)?;

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
