use std::cell::{Ref, RefMut};
use std::mem::size_of;

use bytemuck::from_bytes_mut;
use fixed::types::I80F48;
use fixed_macro::types::I80F48;
use solana_program::account_info::AccountInfo;
use solana_program::pubkey::Pubkey;

use crate::error::{check_assert, MerpsError, MerpsErrorCode, MerpsResult, SourceFileId};
use crate::matching::Side;

use mango_common::Loadable;
use mango_macro::{Loadable, Pod};

pub const MAX_TOKENS: usize = 32;
pub const MAX_PAIRS: usize = MAX_TOKENS - 1;
pub const MAX_NODE_BANKS: usize = 8;
pub const QUOTE_INDEX: usize = 0;
pub const ZERO_I80F48: I80F48 = I80F48!(0);
pub const ONE_I80F48: I80F48 = I80F48!(1);

declare_check_assert_macros!(SourceFileId::State);

// TODO: all unit numbers are just place holders. make decisions on each unit number
// TODO: add prop tests for nums
// TODO add GUI hoster fee discount

#[repr(u8)]
pub enum DataType {
    MerpsGroup = 0,
    MerpsAccount,
    RootBank,
    NodeBank,
    PerpMarket,
    Bids,
    Asks,
    MerpsCache,
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct MetaData {
    pub data_type: u8,
    pub version: u8,
    pub is_initialized: bool,
    pub padding: [u8; 5], // This makes explicit the 8 byte alignment padding
}

#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct MerpsGroup {
    pub meta_data: MetaData,

    pub num_tokens: usize,
    pub num_markets: usize, // Note: does not increase if there is a spot and perp market for same base token

    pub tokens: [Pubkey; MAX_TOKENS],
    pub oracles: [Pubkey; MAX_PAIRS],
    // Note: oracle used for perps mark price is same as the one for spot. This is not ideal so it may change

    // Right now Serum dex spot markets. TODO make this general to an interface
    pub spot_markets: [Pubkey; MAX_PAIRS],

    // Pubkeys of different perpetuals markets
    pub perp_markets: [Pubkey; MAX_PAIRS],

    pub root_banks: [Pubkey; MAX_TOKENS],

    pub asset_weights: [I80F48; MAX_TOKENS],

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
        self.root_banks.iter().position(|pk| pk == root_bank_pk) // TODO profile and optimize
    }
    pub fn find_spot_market_index(&self, spot_market_pk: &Pubkey) -> Option<usize> {
        self.spot_markets.iter().position(|pk| pk == spot_market_pk)
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
    #[allow(unused)]
    pub fn update_index(&mut self, node_banks: &[NodeBank]) -> MerpsResult<()> {
        unimplemented!() // TODO
    }

    #[allow(unused)]
    pub fn get_interest_rate(&self) -> MerpsResult<()> {
        unimplemented!() // TODO
    }
}

impl RootBank {
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
    pub price: I80F48,
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
    pub funding_earned: I80F48,
    pub last_update: u64,
}

#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct MerpsCache {
    pub meta_data: MetaData,

    pub price_cache: [PriceCache; MAX_PAIRS],
    pub root_bank_cache: [RootBankCache; MAX_PAIRS],
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
            if merps_group.perp_markets[i] != Pubkey::default() {
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
    pub total_base: i64,  // total contracts in sell orders
    pub total_quote: i64, // total quote currency in buy orders
    pub is_free_bits: u32,
    pub is_bid_bits: u32,
    pub orders: [u128; 32],
    pub client_order_ids: [u64; 32],
}

#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct MerpsAccount {
    pub meta_data: MetaData,

    pub merps_group: Pubkey,
    pub owner: Pubkey,

    pub in_basket: [bool; MAX_TOKENS], // this can be done with u64 and bit shifting to save space

    // Spot and Margin related data
    pub deposits: [I80F48; MAX_TOKENS],
    pub borrows: [I80F48; MAX_TOKENS],
    pub spot_open_orders: [Pubkey; MAX_PAIRS],

    // Perps related data
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
                //  TODO pass in open orders
                //
                // assets_val = self.open_orders_cache[i]
                //     .quote_total
                //     .checked_add(assets_val)
                //     .ok_or(throw_err!(MerpsErrorCode::MathError))?;

                // base_assets = self.open_orders_cache[i]
                //     .base_total
                //     .checked_add(base_assets)
                //     .ok_or(throw_err!(MerpsErrorCode::MathError))?;
            }

            if merps_group.perp_markets[i] != Pubkey::default() {
                // TODO fill this in once perp logic is a little bit more clear
                if self.base_positions[i] > 0 {
                    // increment assets_val with base position and liabs val with quote position
                    // What if both base position and quote position are positive
                    // Do we force settling of pnl from positions?
                    // settling of losses means marking contracts to the current price and taking any profits (losses) into deposits (borrows)
                } else if self.base_positions[i] < 0 {
                }
            }

            let asset_weight = merps_group.asset_weights[i];
            let liab_weight = ONE_I80F48 / asset_weight;
            assets_val = base_assets
                .checked_mul(merps_cache.price_cache[i].price)
                .ok_or(throw_err!(MerpsErrorCode::MathError))?
                .checked_mul(asset_weight)
                .ok_or(throw_err!(MerpsErrorCode::MathError))?
                .checked_add(assets_val)
                .ok_or(throw_err!(MerpsErrorCode::MathError))?;

            liabs_val = base_liabs
                .checked_mul(merps_cache.price_cache[i].price)
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
#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct PerpMarket {
    pub meta_data: MetaData,

    pub bids: Pubkey,
    pub asks: Pubkey,
    pub event_queue: Pubkey,
    pub matching_queue: Pubkey,
    pub total_funding: I80F48,
    pub open_interest: i64, // This is i64 to keep consistent with the units of contracts, but should always be > 0

    pub quote_lot_size: i64, //
    pub index_oracle: Pubkey,
    pub last_updated: u64,
    pub seq_num: u64,

    pub contract_size: i64, // mark_price = used to liquidate and calculate value of positions; function of index and some moving average of basis
                            // index_price = some function of centralized exchange spot prices
                            // book_price = average of impact bid and impact ask; used to calculate basis
                            // basis = book_price / index_price - 1; some moving average of this is used for mark price
}

impl PerpMarket {
    pub fn gen_order_id(&mut self, side: Side, price: u64) -> u128 {
        self.seq_num += 1;

        let upper = (price as u128) << 64;
        match side {
            Side::Bid => upper | (!self.seq_num as u128),
            Side::Ask => upper | (self.seq_num as u128),
        }
    }

    /// Use current order book price
    pub fn update_funding(&mut self) -> MerpsResult<()> {
        unimplemented!()
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
