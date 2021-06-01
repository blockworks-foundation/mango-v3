use std::cell::{Ref, RefMut};
use std::mem::size_of;

use bytemuck::from_bytes_mut;
use fixed::types::I80F48;
use fixed_macro::types::I80F48;
use solana_program::account_info::AccountInfo;
use solana_program::pubkey::Pubkey;

use crate::error::{check_assert, MerpsError, MerpsErrorCode, MerpsResult, SourceFileId};
use crate::matching::{Book, LeafNode, Side};

use mango_common::Loadable;
use mango_macro::{Loadable, Pod};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use solana_program::sysvar::rent::Rent;

pub const MAX_TOKENS: usize = 32;
pub const MAX_PAIRS: usize = MAX_TOKENS - 1;
pub const MAX_NODE_BANKS: usize = 8;
pub const QUOTE_INDEX: usize = MAX_TOKENS - 1;
pub const ZERO_I80F48: I80F48 = I80F48!(0);
pub const ONE_I80F48: I80F48 = I80F48!(1);
pub const DAY: I80F48 = I80F48!(86400);

declare_check_assert_macros!(SourceFileId::State);

// TODO: all unit numbers are just place holders. make decisions on each unit number
// TODO: add prop tests for nums
// TODO add GUI hoster fee discount

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

#[derive(Copy, Clone, Pod)]
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

    pub num_markets: usize, // Note: does not increase if there is a spot and perp market for same base token
    pub num_oracles: usize, // incremented every time add_oracle is called

    pub tokens: [TokenInfo; MAX_TOKENS],
    pub spot_markets: [SpotMarketInfo; MAX_PAIRS],
    pub perp_markets: [PerpMarketInfo; MAX_PAIRS],

    pub oracles: [Pubkey; MAX_PAIRS],

    // TODO add liab versions of each of these as a compute optimization
    pub signer_nonce: u64,
    pub signer_key: Pubkey,
    pub admin: Pubkey, // Used to add new markets and adjust risk params
    pub dex_program_id: Pubkey,
    pub merps_cache: Pubkey,
    // TODO determine liquidation incentives for each token
    // TODO determine maint weight and init weight

    // TODO store risk params (collateral weighting, liability weighting, perp weighting, liq weighting (?))
    // TODO consider storing oracle prices here
    //      it makes this more single threaded if cranks are writing to merps group constantly with oracle prices
    pub valid_interval: u8,
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

    #[allow(unused)]
    pub fn update_index(&mut self, node_banks: &[NodeBank]) -> MerpsResult<()> {
        unimplemented!() // TODO
    }
    #[allow(unused)]
    pub fn get_interest_rate(&self) -> MerpsResult<()> {
        unimplemented!() // TODO
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
    pub total_funding: I80F48,
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
        let valid_interval = merps_group.valid_interval as u64;
        if now_ts > self.root_bank_cache[QUOTE_INDEX].last_update + valid_interval {
            return false;
        }

        for i in 0..merps_group.num_markets {
            // If this asset is not in user basket, then there are no deposits, borrows or perp positions to calculate value of
            if !merps_account.in_basket[i] {
                continue;
            }
            if now_ts > self.price_cache[i].last_update + valid_interval {
                return false;
            }
            if now_ts > self.root_bank_cache[i].last_update + valid_interval {
                return false;
            }
            // TODO uncomment this when cache_perp_market() is implemented
            // if merps_group.perp_markets[i] != Pubkey::default() {
            //     if now_ts > self.perp_market_cache[i].last_update + valid_interval {
            //         return false;
            //     }
            // }
        }

        true
    }
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct PerpOpenOrders {
    pub total_base: i64,  // total contracts in sell orders
    pub total_quote: i64, // total quote currency in buy orders
    pub is_free_bits: u32,
    pub is_bid_bits: u32,
    pub orders: [i128; 32],
    pub client_order_ids: [u64; 32],
}

impl PerpOpenOrders {
    pub fn add_order(&mut self, side: Side, order: &LeafNode) -> MerpsResult<()> {
        check!(self.is_free_bits != 0, MerpsErrorCode::TooManyOpenOrders)?;
        let slot = self.is_free_bits.trailing_zeros();
        let slot_mask = 1u32 << slot;
        self.is_free_bits &= !slot_mask;
        match side {
            Side::Bid => {
                // TODO make checked
                self.is_bid_bits |= slot_mask;
                self.total_base += order.quantity;
                self.total_quote -= order.quantity * order.price();
            }
            Side::Ask => {
                self.is_bid_bits &= !slot_mask;
                self.total_base -= order.quantity;
                self.total_quote += order.quantity * order.price();
            }
        };

        self.orders[slot as usize] = order.key;
        Ok(())
    }
}

#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct MerpsAccount {
    pub meta_data: MetaData,

    pub merps_group: Pubkey,
    pub owner: Pubkey,

    pub in_basket: [bool; MAX_PAIRS], // this can be done with u64 and bit shifting to save space

    // Spot and Margin related data
    pub deposits: [I80F48; MAX_TOKENS],
    pub borrows: [I80F48; MAX_TOKENS],
    pub spot_open_orders: [Pubkey; MAX_PAIRS],

    // Perps related data

    // TODO consider storing positions as two separate arrays, i.e. base_longs, base_shorts
    pub base_positions: [i64; MAX_PAIRS], // measured in base lots
    pub quote_positions: [i64; MAX_PAIRS], // measured in quote lots
    pub funding_settled: [I80F48; MAX_PAIRS],
    pub perp_open_orders: [PerpOpenOrders; MAX_PAIRS],
    // settlement
    // two merps accounts are passed in
    // only equal amounts can be settled
    // if an account doesn't have enough of the quote currency, it is borrowed
    // if there is no availability to borrow or account doesn't have the coll ratio, keeper may swap some of his USDC for collateral at discount
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
    pub fn get_native_deposit(&self, root_bank: &RootBank, token_i: usize) -> u64 {
        (self.deposits[token_i] * root_bank.deposit_index).to_num()
    }
    pub fn get_native_borrow(&self, root_bank: &RootBank, token_i: usize) -> u64 {
        (self.borrows[token_i] * root_bank.borrow_index).to_num()
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

    // TODO need a new name for this as it's not exactly collateral ratio
    pub fn get_coll_ratio(
        &self,
        merps_group: &MerpsGroup,
        merps_cache: &MerpsCache,
        _spot_open_orders_ais: &[AccountInfo],
    ) -> MerpsResult<I80F48> {
        // Value of all assets and liabs in quote currency
        let quote_i = QUOTE_INDEX;
        let mut assets_val = merps_cache.root_bank_cache[quote_i]
            .deposit_index
            .checked_mul(self.deposits[quote_i])
            .ok_or(throw_err!(MerpsErrorCode::MathError))?;

        let mut liabs_val = merps_cache.root_bank_cache[quote_i]
            .borrow_index
            .checked_mul(self.borrows[quote_i])
            .ok_or(throw_err!(MerpsErrorCode::MathError))?;

        for i in 0..merps_group.num_markets {
            // If this asset is not in user basket, then there are no deposits, borrows or perp positions to calculate value of
            if !self.in_basket[i] {
                continue;
            }

            // TODO make mut when uncommenting the todo below
            let base_assets = merps_cache.root_bank_cache[i]
                .deposit_index
                .checked_mul(self.deposits[i])
                .ok_or(throw_err!(MerpsErrorCode::MathError))?;

            let base_liabs = merps_cache.root_bank_cache[i]
                .borrow_index
                .checked_mul(self.borrows[i])
                .ok_or(throw_err!(MerpsErrorCode::MathError))?;

            if self.spot_open_orders[i] != Pubkey::default() {
                //  TODO load open orders
                //
                // assets_val = open_orders_ais[i]
                //     .quote_total
                //     .checked_add(assets_val)
                //     .ok_or(throw_err!(MerpsErrorCode::MathError))?;

                // base_assets = open_orders_ais[i]
                //     .base_total
                //     .checked_add(base_assets)
                //     .ok_or(throw_err!(MerpsErrorCode::MathError))?;
            }

            let perp_market_info = &merps_group.perp_markets[i];
            if !perp_market_info.is_empty() {
                // TODO fill this in once perp logic is a little bit more clear
                let native_pos = I80F48::from_num(
                    self.base_positions[i]
                        + self.perp_open_orders[i]
                            .total_base
                            .checked_mul(perp_market_info.base_lot_size)
                            .ok_or(throw_err!(MerpsErrorCode::MathError))?,
                );

                if self.base_positions[i] > 0 {
                    assets_val = native_pos
                        .checked_mul(perp_market_info.init_asset_weight)
                        .ok_or(math_err!())?
                        .checked_mul(merps_cache.price_cache[i].price)
                        .ok_or(math_err!())?
                        .checked_add(assets_val)
                        .ok_or(math_err!())?;
                } else if self.base_positions[i] < 0 {
                    liabs_val = -native_pos
                        .checked_mul(perp_market_info.init_liab_weight)
                        .ok_or(math_err!())?
                        .checked_mul(merps_cache.price_cache[i].price)
                        .ok_or(math_err!())?
                        .checked_add(assets_val)
                        .ok_or(math_err!())?;
                }

                if self.quote_positions[i] < 0 {
                    // TODO
                }

                // Account for unrealized funding payments
                // TODO make checked
                let funding: I80F48 = (merps_cache.perp_market_cache[i].total_funding
                    - self.funding_settled[i])
                    * I80F48::from_num(self.base_positions[i]);

                // units
                // total_funding: I80F48 - native quote currency per contract
                // funding_settled: I80F48 - native quote currency per contract
                // base_positions: i64 - number of contracts
                // quote_positions: i64 - number of quote_lot_size
                // funding: I80F48 - native quote currency
                // assets_val: I80F48 - native quote currency

                if funding > ZERO_I80F48 {
                    // funding positive, means liab
                    liabs_val += funding;
                } else if funding < ZERO_I80F48 {
                    assets_val -= funding;
                }

                // Account for open orders

                let oo_base = I80F48::from_num(self.perp_open_orders[i].total_base); // num contracts
                let _oo_quote = I80F48::from_num(self.perp_open_orders[i].total_quote);

                if self.base_positions[i] > 0 {
                    if oo_base > 0 { // open long
                    } else if oo_base < 0 {
                        // close long
                    }
                } else if self.base_positions[i] < 0 {
                    if oo_base > 0 { // close short
                    } else if oo_base < 0 { // open short
                    }
                }

                // lot price

                /*
                    1. The amount on the open orders that are closing existing positions don't count against collateral
                    2. open orders that are opening do count against collateral
                    3. Possibly the open orders itself can store this information
                */
            }

            assets_val = base_assets
                .checked_mul(merps_cache.price_cache[i].price)
                .ok_or(throw_err!(MerpsErrorCode::MathError))?
                .checked_mul(merps_group.spot_markets[i].init_asset_weight)
                .ok_or(throw_err!(MerpsErrorCode::MathError))?
                .checked_add(assets_val)
                .ok_or(throw_err!(MerpsErrorCode::MathError))?;

            liabs_val = base_liabs
                .checked_mul(merps_cache.price_cache[i].price)
                .ok_or(throw_err!(MerpsErrorCode::MathError))?
                .checked_mul(merps_group.spot_markets[i].init_liab_weight)
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
#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct PerpMarket {
    pub meta_data: MetaData,

    pub merps_group: Pubkey,
    pub bids: Pubkey,
    pub asks: Pubkey,
    pub event_queue: Pubkey,
    pub total_funding: I80F48,
    pub open_interest: i64, // This is i64 to keep consistent with the units of contracts, but should always be > 0

    pub quote_lot_size: i64, // number of quote native that reresents min tick
    pub index_oracle: Pubkey,
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
        // Get current book price
        // compare it to index price using the merps cache

        // TODO handle case of one sided book
        // TODO get impact bid and impact ask if compute allows
        let bid = book.get_best_bid_price().unwrap();
        let ask = book.get_best_ask_price().unwrap();

        let book_price = self.lot_to_native_price((bid + ask) / 2);

        // TODO make checked
        let price_cache = &merps_cache.price_cache[market_index];
        check!(
            now_ts <= price_cache.last_update + (merps_group.valid_interval as u64),
            MerpsErrorCode::InvalidCache
        )?;

        let index_price = price_cache.price;
        let diff: I80F48 = (book_price / index_price) - ONE_I80F48;
        let time_factor = I80F48::from_num(now_ts - self.last_updated) / DAY;
        let funding_delta: I80F48 = diff
            * time_factor
            * I80F48::from_num(self.open_interest)
            * I80F48::from_num(self.contract_size)
            * index_price;

        self.total_funding += funding_delta;
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
