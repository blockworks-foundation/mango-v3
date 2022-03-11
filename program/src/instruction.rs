use crate::matching::{OrderType, Side};
use crate::state::{AssetType, INFO_LEN};
use crate::state::{TriggerCondition, MAX_PAIRS};
use arrayref::{array_ref, array_refs};
use fixed::types::I80F48;
use num_enum::TryFromPrimitive;
use serde::{Deserialize, Serialize};
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;
use std::convert::{TryFrom, TryInto};
use std::num::NonZeroU64;

#[repr(C)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MangoInstruction {
    /// Initialize a group of lending pools that can be cross margined
    ///
    /// Accounts expected by this instruction (12):
    ///
    /// 0. `[writable]` mango_group_ai
    /// 1. `[]` signer_ai
    /// 2. `[]` admin_ai
    /// 3. `[]` quote_mint_ai
    /// 4. `[]` quote_vault_ai
    /// 5. `[writable]` quote_node_bank_ai
    /// 6. `[writable]` quote_root_bank_ai
    /// 7. `[]` dao_vault_ai - aka insurance fund
    /// 8. `[]` msrm_vault_ai - msrm deposits for fee discounts; can be Pubkey::default()
    /// 9. `[]` fees_vault_ai - vault owned by Mango DAO token governance to receive fees
    /// 10. `[writable]` mango_cache_ai - Account to cache prices, root banks, and perp markets
    /// 11. `[]` dex_prog_ai
    InitMangoGroup {
        signer_nonce: u64,
        valid_interval: u64,
        quote_optimal_util: I80F48,
        quote_optimal_rate: I80F48,
        quote_max_rate: I80F48,
    },

    /// DEPRECATED Initialize a mango account for a user
    /// Accounts created with this function cannot be closed without upgrading with UpgradeMangoAccountV0V1
    ///
    /// Accounts expected by this instruction (3):
    ///
    /// 0. `[]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[writable]` mango_account_ai - the mango account data
    /// 2. `[signer]` owner_ai - Solana account of owner of the mango account
    InitMangoAccount,

    /// Deposit funds into mango account
    ///
    /// Accounts expected by this instruction (9):
    ///
    /// 0. `[]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[writable]` mango_account_ai - the mango account for this user
    /// 2. `[signer]` owner_ai - Solana account of owner of the mango account
    /// 3. `[]` mango_cache_ai - MangoCache
    /// 4. `[]` root_bank_ai - RootBank owned by MangoGroup
    /// 5. `[writable]` node_bank_ai - NodeBank owned by RootBank
    /// 6. `[writable]` vault_ai - TokenAccount owned by MangoGroup
    /// 7. `[]` token_prog_ai - acc pointed to by SPL token program id
    /// 8. `[writable]` owner_token_account_ai - TokenAccount owned by user which will be sending the funds
    Deposit {
        quantity: u64,
    },

    /// Withdraw funds that were deposited earlier.
    ///
    /// Accounts expected by this instruction (10):
    ///
    /// 0. `[read]` mango_group_ai,   -
    /// 1. `[write]` mango_account_ai, -
    /// 2. `[read]` owner_ai,         -
    /// 3. `[read]` mango_cache_ai,   -
    /// 4. `[read]` root_bank_ai,     -
    /// 5. `[write]` node_bank_ai,     -
    /// 6. `[write]` vault_ai,         -
    /// 7. `[write]` token_account_ai, -
    /// 8. `[read]` signer_ai,        -
    /// 9. `[read]` token_prog_ai,    -
    /// 10..+ `[]` open_orders_accs - open orders for each of the spot market
    Withdraw {
        quantity: u64,
        allow_borrow: bool,
    },

    /// Add a token to a mango group
    ///
    /// Accounts expected by this instruction (8):
    ///
    /// 0. `[writable]` mango_group_ai
    /// 1  `[]` oracle_ai
    /// 2. `[]` spot_market_ai
    /// 3. `[]` dex_program_ai
    /// 4. `[]` mint_ai
    /// 5. `[writable]` node_bank_ai
    /// 6. `[]` vault_ai
    /// 7. `[writable]` root_bank_ai
    /// 8. `[signer]` admin_ai
    AddSpotMarket {
        maint_leverage: I80F48,
        init_leverage: I80F48,
        liquidation_fee: I80F48,
        optimal_util: I80F48,
        optimal_rate: I80F48,
        max_rate: I80F48,
    },

    /// DEPRECATED
    AddToBasket {
        market_index: usize,
    },

    /// DEPRECATED - use Withdraw with allow_borrow = true
    Borrow {
        quantity: u64,
    },

    /// Cache prices
    ///
    /// Accounts expected: 3 + Oracles
    /// 0. `[]` mango_group_ai -
    /// 1. `[writable]` mango_cache_ai -
    /// 2+... `[]` oracle_ais - flux aggregator feed accounts
    CachePrices,

    /// DEPRECATED - caching of root banks now happens in update index
    /// Cache root banks
    ///
    /// Accounts expected: 2 + Root Banks
    /// 0. `[]` mango_group_ai
    /// 1. `[writable]` mango_cache_ai
    CacheRootBanks,

    /// Place an order on the Serum Dex using Mango account
    ///
    /// Accounts expected by this instruction (23 + MAX_PAIRS):
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[writable]` mango_account_ai - the MangoAccount of owner
    /// 2. `[signer]` owner_ai - owner of MangoAccount
    /// 3. `[]` mango_cache_ai - MangoCache for this MangoGroup
    /// 4. `[]` dex_prog_ai - serum dex program id
    /// 5. `[writable]` spot_market_ai - serum dex MarketState account
    /// 6. `[writable]` bids_ai - bids account for serum dex market
    /// 7. `[writable]` asks_ai - asks account for serum dex market
    /// 8. `[writable]` dex_request_queue_ai - request queue for serum dex market
    /// 9. `[writable]` dex_event_queue_ai - event queue for serum dex market
    /// 10. `[writable]` dex_base_ai - base currency serum dex market vault
    /// 11. `[writable]` dex_quote_ai - quote currency serum dex market vault
    /// 12. `[]` base_root_bank_ai - root bank of base currency
    /// 13. `[writable]` base_node_bank_ai - node bank of base currency
    /// 14. `[writable]` base_vault_ai - vault of the basenode bank
    /// 15. `[]` quote_root_bank_ai - root bank of quote currency
    /// 16. `[writable]` quote_node_bank_ai - node bank of quote currency
    /// 17. `[writable]` quote_vault_ai - vault of the quote node bank
    /// 18. `[]` token_prog_ai - SPL token program id
    /// 19. `[]` signer_ai - signer key for this MangoGroup
    /// 20. `[]` rent_ai - rent sysvar var
    /// 21. `[]` dex_signer_key - signer for serum dex
    /// 22. `[]` msrm_or_srm_vault_ai - the msrm or srm vault in this MangoGroup. Can be zero key
    /// 23+ `[writable]` open_orders_ais - An array of MAX_PAIRS. Only OpenOrders of current market
    ///         index needs to be writable. Only OpenOrders in_margin_basket needs to be correct;
    ///         remaining open orders can just be Pubkey::default() (the zero key)
    PlaceSpotOrder {
        order: serum_dex::instruction::NewOrderInstructionV3,
    },

    /// Add oracle
    ///
    /// Accounts expected: 3
    /// 0. `[writable]` mango_group_ai - MangoGroup
    /// 1. `[writable]` oracle_ai - oracle
    /// 2. `[signer]` admin_ai - admin
    AddOracle, // = 10

    /// Add a perp market to a mango group
    ///
    /// Accounts expected by this instruction (7):
    ///
    /// 0. `[writable]` mango_group_ai
    /// 1. `[]` oracle_ai
    /// 2. `[writable]` perp_market_ai
    /// 3. `[writable]` event_queue_ai
    /// 4. `[writable]` bids_ai
    /// 5. `[writable]` asks_ai
    /// 6. `[]` mngo_vault_ai - the vault from which liquidity incentives will be paid out for this market
    /// 7. `[signer]` admin_ai
    AddPerpMarket {
        maint_leverage: I80F48,
        init_leverage: I80F48,
        liquidation_fee: I80F48,
        maker_fee: I80F48,
        taker_fee: I80F48,
        base_lot_size: i64,
        quote_lot_size: i64,
        /// Starting rate for liquidity mining
        rate: I80F48,
        /// depth liquidity mining works for
        max_depth_bps: I80F48,
        /// target length in seconds of one period
        target_period_length: u64,
        /// amount MNGO rewarded per period
        mngo_per_period: u64,
        /// Optional: Exponent in the liquidity mining formula; default 2
        exp: u8,
    },

    /// Place an order on a perp market
    ///
    /// In case this order is matched, the corresponding order structs on both
    /// PerpAccounts (taker & maker) will be adjusted, and the position size
    /// will be adjusted w/o accounting for fees.
    /// In addition a FillEvent will be placed on the event queue.
    /// Through a subsequent invocation of ConsumeEvents the FillEvent can be
    /// executed and the perp account balances (base/quote) and fees will be
    /// paid from the quote position. Only at this point the position balance
    /// is 100% refelecting the trade.
    ///
    /// Accounts expected by this instruction (8 + `MAX_PAIRS` + (optional 1)):
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[writable]` mango_account_ai - the MangoAccount of owner
    /// 2. `[signer]` owner_ai - owner of MangoAccount
    /// 3. `[]` mango_cache_ai - MangoCache for this MangoGroup
    /// 4. `[writable]` perp_market_ai
    /// 5. `[writable]` bids_ai - bids account for this PerpMarket
    /// 6. `[writable]` asks_ai - asks account for this PerpMarket
    /// 7. `[writable]` event_queue_ai - EventQueue for this PerpMarket
    /// 8..23 `[]` open_orders_ais - array of open orders accounts on this MangoAccount
    /// 23. `[writable]` referrer_mango_account_ai - optional, mango account of referrer
    PlacePerpOrder {
        price: i64,
        quantity: i64,
        client_order_id: u64,
        side: Side,
        /// Can be 0 -> LIMIT, 1 -> IOC, 2 -> PostOnly, 3 -> Market, 4 -> PostOnlySlide
        order_type: OrderType,
        /// Optional to be backward compatible; default false
        reduce_only: bool,
    },

    CancelPerpOrderByClientId {
        client_order_id: u64,
        invalid_id_ok: bool,
    },

    CancelPerpOrder {
        order_id: i128,
        invalid_id_ok: bool,
    },

    ConsumeEvents {
        limit: usize,
    },

    /// Cache perp markets
    ///
    /// Accounts expected: 2 + Perp Markets
    /// 0. `[]` mango_group_ai
    /// 1. `[writable]` mango_cache_ai
    CachePerpMarkets,

    /// Update funding related variables
    UpdateFunding,

    /// Can only be used on a stub oracle in devnet
    SetOracle {
        price: I80F48,
    },

    /// Settle all funds from serum dex open orders
    ///
    /// Accounts expected by this instruction (18):
    ///
    /// 0. `[]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[]` mango_cache_ai - MangoCache for this MangoGroup
    /// 2. `[signer]` owner_ai - MangoAccount owner
    /// 3. `[writable]` mango_account_ai - MangoAccount
    /// 4. `[]` dex_prog_ai - program id of serum dex
    /// 5.  `[writable]` spot_market_ai - dex MarketState account
    /// 6.  `[writable]` open_orders_ai - open orders for this market for this MangoAccount
    /// 7. `[]` signer_ai - MangoGroup signer key
    /// 8. `[writable]` dex_base_ai - base vault for dex MarketState
    /// 9. `[writable]` dex_quote_ai - quote vault for dex MarketState
    /// 10. `[]` base_root_bank_ai - MangoGroup base vault acc
    /// 11. `[writable]` base_node_bank_ai - MangoGroup quote vault acc
    /// 12. `[]` quote_root_bank_ai - MangoGroup quote vault acc
    /// 13. `[writable]` quote_node_bank_ai - MangoGroup quote vault acc
    /// 14. `[writable]` base_vault_ai - MangoGroup base vault acc
    /// 15. `[writable]` quote_vault_ai - MangoGroup quote vault acc
    /// 16. `[]` dex_signer_ai - dex Market signer account
    /// 17. `[]` spl token program
    SettleFunds,

    /// Cancel an order using dex instruction
    ///
    /// Accounts expected by this instruction ():
    ///
    CancelSpotOrder {
        // 20
        order: serum_dex::instruction::CancelOrderInstructionV2,
    },

    /// Update a root bank's indexes by providing all it's node banks
    ///
    /// Accounts expected: 2 + Node Banks
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[]` root_bank_ai - RootBank
    /// 2+... `[]` node_bank_ais - NodeBanks
    UpdateRootBank,

    /// Take two MangoAccounts and settle profits and losses between them for a perp market
    ///
    /// Accounts expected (6):
    SettlePnl {
        market_index: usize,
    },

    /// DEPRECATED - no longer makes sense
    /// Use this token's position and deposit to reduce borrows
    ///
    /// Accounts expected by this instruction (5):
    SettleBorrow {
        token_index: usize,
        quantity: u64,
    },

    /// Force cancellation of open orders for a user being liquidated
    ///
    /// Accounts expected: 19 + Liqee open orders accounts (MAX_PAIRS)
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[]` mango_cache_ai - MangoCache
    /// 2. `[writable]` liqee_mango_account_ai - MangoAccount
    /// 3. `[]` base_root_bank_ai - RootBank
    /// 4. `[writable]` base_node_bank_ai - NodeBank
    /// 5. `[writable]` base_vault_ai - MangoGroup base vault acc
    /// 6. `[]` quote_root_bank_ai - RootBank
    /// 7. `[writable]` quote_node_bank_ai - NodeBank
    /// 8. `[writable]` quote_vault_ai - MangoGroup quote vault acc
    /// 9. `[writable]` spot_market_ai - SpotMarket
    /// 10. `[writable]` bids_ai - SpotMarket bids acc
    /// 11. `[writable]` asks_ai - SpotMarket asks acc
    /// 12. `[signer]` signer_ai - Signer
    /// 13. `[writable]` dex_event_queue_ai - Market event queue acc
    /// 14. `[writable]` dex_base_ai -
    /// 15. `[writable]` dex_quote_ai -
    /// 16. `[]` dex_signer_ai -
    /// 17. `[]` dex_prog_ai - Dex Program acc
    /// 18. `[]` token_prog_ai - Token Program acc
    /// 19+... `[]` liqee_open_orders_ais - Liqee open orders accs
    ForceCancelSpotOrders {
        limit: u8,
    },

    /// Force cancellation of open orders for a user being liquidated
    ///
    /// Accounts expected: 6 + Liqee open orders accounts (MAX_PAIRS)
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[]` mango_cache_ai - MangoCache
    /// 2. `[]` perp_market_ai - PerpMarket
    /// 3. `[writable]` bids_ai - Bids acc
    /// 4. `[writable]` asks_ai - Asks acc
    /// 5. `[writable]` liqee_mango_account_ai - Liqee MangoAccount
    /// 6+... `[]` liqor_open_orders_ais - Liqee open orders accs
    ForceCancelPerpOrders {
        limit: u8,
    },

    /// Liquidator takes some of borrows at token at `liab_index` and receives some deposits from
    /// the token at `asset_index`
    ///
    /// Accounts expected: 9 + Liqee open orders accounts (MAX_PAIRS) + Liqor open orders accounts (MAX_PAIRS)
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[]` mango_cache_ai - MangoCache
    /// 2. `[writable]` liqee_mango_account_ai - MangoAccount
    /// 3. `[writable]` liqor_mango_account_ai - MangoAccount
    /// 4. `[signer]` liqor_ai - Liqor Account
    /// 5. `[]` asset_root_bank_ai - RootBank
    /// 6. `[writable]` asset_node_bank_ai - NodeBank
    /// 7. `[]` liab_root_bank_ai - RootBank
    /// 8. `[writable]` liab_node_bank_ai - NodeBank
    /// 9+... `[]` liqee_open_orders_ais - Liqee open orders accs
    /// 9+MAX_PAIRS... `[]` liqor_open_orders_ais - Liqor open orders accs
    LiquidateTokenAndToken {
        max_liab_transfer: I80F48,
    },

    /// Swap tokens for perp quote position if only and only if the base position in that market is 0
    ///
    /// Accounts expected: 7 + Liqee open orders accounts (MAX_PAIRS) + Liqor open orders accounts (MAX_PAIRS)
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[]` mango_cache_ai - MangoCache
    /// 2. `[writable]` liqee_mango_account_ai - MangoAccount
    /// 3. `[writable]` liqor_mango_account_ai - MangoAccount
    /// 4. `[signer]` liqor_ai - Liqor Account
    /// 5. `[]` root_bank_ai - RootBank
    /// 6. `[writable]` node_bank_ai - NodeBank
    /// 7+... `[]` liqee_open_orders_ais - Liqee open orders accs
    /// 7+MAX_PAIRS... `[]` liqor_open_orders_ais - Liqor open orders accs
    LiquidateTokenAndPerp {
        asset_type: AssetType,
        asset_index: usize,
        liab_type: AssetType,
        liab_index: usize,
        max_liab_transfer: I80F48,
    },

    /// Reduce some of the base position in exchange for quote position in this market
    ///
    /// Accounts expected: 7 + Liqee open orders accounts (MAX_PAIRS) + Liqor open orders accounts (MAX_PAIRS)
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[]` mango_cache_ai - MangoCache
    /// 2. `[writable]` perp_market_ai - PerpMarket
    /// 3. `[writable]` event_queue_ai - EventQueue
    /// 4. `[writable]` liqee_mango_account_ai - MangoAccount
    /// 5. `[writable]` liqor_mango_account_ai - MangoAccount
    /// 6. `[signer]` liqor_ai - Liqor Account
    /// 7+... `[]` liqee_open_orders_ais - Liqee open orders accs
    /// 7+MAX_PAIRS... `[]` liqor_open_orders_ais - Liqor open orders accs
    LiquidatePerpMarket {
        base_transfer_request: i64,
    },

    /// Take an account that has losses in the selected perp market to account for fees_accrued
    ///
    /// Accounts expected: 10
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[]` mango_cache_ai - MangoCache
    /// 2. `[writable]` perp_market_ai - PerpMarket
    /// 3. `[writable]` mango_account_ai - MangoAccount
    /// 4. `[]` root_bank_ai - RootBank
    /// 5. `[writable]` node_bank_ai - NodeBank
    /// 6. `[writable]` bank_vault_ai - ?
    /// 7. `[writable]` fees_vault_ai - fee vault owned by mango DAO token governance
    /// 8. `[]` signer_ai - Group Signer Account
    /// 9. `[]` token_prog_ai - Token Program Account
    SettleFees,

    /// Claim insurance fund and then socialize loss
    ///
    /// Accounts expected: 12 + Liqor open orders accounts (MAX_PAIRS)
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[writable]` mango_cache_ai - MangoCache
    /// 2. `[writable]` liqee_mango_account_ai - Liqee MangoAccount
    /// 3. `[writable]` liqor_mango_account_ai - Liqor MangoAccount
    /// 4. `[signer]` liqor_ai - Liqor Account
    /// 5. `[]` root_bank_ai - RootBank
    /// 6. `[writable]` node_bank_ai - NodeBank
    /// 7. `[writable]` vault_ai - ?
    /// 8. `[writable]` dao_vault_ai - DAO Vault
    /// 9. `[]` signer_ai - Group Signer Account
    /// 10. `[]` perp_market_ai - PerpMarket
    /// 11. `[]` token_prog_ai - Token Program Account
    /// 12+... `[]` liqor_open_orders_ais - Liqor open orders accs
    ResolvePerpBankruptcy {
        // 30
        liab_index: usize,
        max_liab_transfer: I80F48,
    },

    /// Claim insurance fund and then socialize loss
    ///
    /// Accounts expected: 13 + Liqor open orders accounts (MAX_PAIRS) + Liab node banks (MAX_NODE_BANKS)
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[writable]` mango_cache_ai - MangoCache
    /// 2. `[writable]` liqee_mango_account_ai - Liqee MangoAccount
    /// 3. `[writable]` liqor_mango_account_ai - Liqor MangoAccount
    /// 4. `[signer]` liqor_ai - Liqor Account
    /// 5. `[]` quote_root_bank_ai - RootBank
    /// 6. `[writable]` quote_node_bank_ai - NodeBank
    /// 7. `[writable]` quote_vault_ai - ?
    /// 8. `[writable]` dao_vault_ai - DAO Vault
    /// 9. `[]` signer_ai - Group Signer Account
    /// 10. `[]` liab_root_bank_ai - RootBank
    /// 11. `[writable]` liab_node_bank_ai - NodeBank
    /// 12. `[]` token_prog_ai - Token Program Account
    /// 13+... `[]` liqor_open_orders_ais - Liqor open orders accs
    /// 14+MAX_PAIRS... `[]` liab_node_bank_ais - Lib token node banks
    ResolveTokenBankruptcy {
        max_liab_transfer: I80F48,
    },

    /// Initialize open orders
    ///
    /// Accounts expected by this instruction (8):
    ///
    /// 0. `[]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[writable]` mango_account_ai - MangoAccount
    /// 2. `[signer]` owner_ai - MangoAccount owner
    /// 3. `[]` dex_prog_ai - program id of serum dex
    /// 4. `[writable]` open_orders_ai - open orders for this market for this MangoAccount
    /// 5. `[]` spot_market_ai - dex MarketState account
    /// 6. `[]` signer_ai - Group Signer Account
    /// 7. `[]` rent_ai - Rent sysvar account
    InitSpotOpenOrders,

    /// Redeem the mngo_accrued in a PerpAccount for MNGO in MangoAccount deposits
    ///
    /// Accounts expected by this instruction (11):
    /// 0. `[]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[]` mango_cache_ai - MangoCache
    /// 2. `[writable]` mango_account_ai - MangoAccount
    /// 3. `[signer]` owner_ai - MangoAccount owner
    /// 4. `[]` perp_market_ai - PerpMarket
    /// 5. `[writable]` mngo_perp_vault_ai
    /// 6. `[]` mngo_root_bank_ai
    /// 7. `[writable]` mngo_node_bank_ai
    /// 8. `[writable]` mngo_bank_vault_ai
    /// 9. `[]` signer_ai - Group Signer Account
    /// 10. `[]` token_prog_ai - SPL Token program id
    RedeemMngo,

    /// Add account info; useful for naming accounts
    ///
    /// Accounts expected by this instruction (3):
    /// 0. `[]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[writable]` mango_account_ai - MangoAccount
    /// 2. `[signer]` owner_ai - MangoAccount owner
    AddMangoAccountInfo {
        info: [u8; INFO_LEN],
    },

    /// Deposit MSRM to reduce fees. This MSRM is not at risk and is not used for any health calculations
    ///
    /// Accounts expected by this instruction (6):
    ///
    /// 0. `[]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[writable]` mango_account_ai - MangoAccount
    /// 2. `[signer]` owner_ai - MangoAccount owner
    /// 3. `[writable]` msrm_account_ai - MSRM token account
    /// 4. `[writable]` msrm_vault_ai - MSRM vault owned by mango program
    /// 5. `[]` token_prog_ai - SPL Token program id
    DepositMsrm {
        quantity: u64,
    },

    /// Withdraw the MSRM deposited
    ///
    /// Accounts expected by this instruction (7):
    ///
    /// 0. `[]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[writable]` mango_account_ai - MangoAccount
    /// 2. `[signer]` owner_ai - MangoAccount owner
    /// 3. `[writable]` msrm_account_ai - MSRM token account
    /// 4. `[writable]` msrm_vault_ai - MSRM vault owned by mango program
    /// 5. `[]` signer_ai - signer key of the MangoGroup
    /// 6. `[]` token_prog_ai - SPL Token program id
    WithdrawMsrm {
        quantity: u64,
    },

    /// Change the params for perp market.
    ///
    /// Accounts expected by this instruction (3):
    /// 0. `[writable]` mango_group_ai - MangoGroup
    /// 1. `[writable]` perp_market_ai - PerpMarket
    /// 2. `[signer]` admin_ai - MangoGroup admin
    ChangePerpMarketParams {
        #[serde(serialize_with = "serialize_option_fixed_width")]
        maint_leverage: Option<I80F48>,

        #[serde(serialize_with = "serialize_option_fixed_width")]
        init_leverage: Option<I80F48>,

        #[serde(serialize_with = "serialize_option_fixed_width")]
        liquidation_fee: Option<I80F48>,

        #[serde(serialize_with = "serialize_option_fixed_width")]
        maker_fee: Option<I80F48>,

        #[serde(serialize_with = "serialize_option_fixed_width")]
        taker_fee: Option<I80F48>,

        /// Starting rate for liquidity mining
        #[serde(serialize_with = "serialize_option_fixed_width")]
        rate: Option<I80F48>,

        /// depth liquidity mining works for
        #[serde(serialize_with = "serialize_option_fixed_width")]
        max_depth_bps: Option<I80F48>,

        /// target length in seconds of one period
        #[serde(serialize_with = "serialize_option_fixed_width")]
        target_period_length: Option<u64>,

        /// amount MNGO rewarded per period
        #[serde(serialize_with = "serialize_option_fixed_width")]
        mngo_per_period: Option<u64>,

        /// Optional: Exponent in the liquidity mining formula
        #[serde(serialize_with = "serialize_option_fixed_width")]
        exp: Option<u8>,
    },

    /// Transfer admin permissions over group to another account
    ///
    /// Accounts expected by this instruction (3):
    /// 0. `[writable]` mango_group_ai - MangoGroup
    /// 1. `[]` new_admin_ai - New MangoGroup admin
    /// 2. `[signer]` admin_ai - MangoGroup admin
    SetGroupAdmin,

    /// Cancel all perp open orders (batch cancel)
    ///
    /// Accounts expected: 6
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[writable]` mango_account_ai - MangoAccount
    /// 2. `[signer]` owner_ai - Owner of Mango Account
    /// 3. `[writable]` perp_market_ai - PerpMarket
    /// 4. `[writable]` bids_ai - Bids acc
    /// 5. `[writable]` asks_ai - Asks acc
    CancelAllPerpOrders {
        limit: u8,
    },

    /// DEPRECATED - No longer valid instruction as of release 3.0.5
    /// Liqor takes on all the quote positions where base_position == 0
    /// Equivalent amount of quote currency is credited/debited in deposits/borrows.
    /// This is very similar to the settle_pnl function, but is forced for Sick accounts
    ///
    /// Accounts expected: 7 + MAX_PAIRS
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[]` mango_cache_ai - MangoCache
    /// 2. `[writable]` liqee_mango_account_ai - MangoAccount
    /// 3. `[writable]` liqor_mango_account_ai - MangoAccount
    /// 4. `[signer]` liqor_ai - Liqor Account
    /// 5. `[]` root_bank_ai - RootBank
    /// 6. `[writable]` node_bank_ai - NodeBank
    /// 7+... `[]` liqee_open_orders_ais - Liqee open orders accs
    ForceSettleQuotePositions, // instruction 40

    /// Place an order on the Serum Dex using Mango account. Improved over PlaceSpotOrder
    /// by reducing the tx size
    PlaceSpotOrder2 {
        order: serum_dex::instruction::NewOrderInstructionV3,
    },

    /// Initialize the advanced open orders account for a MangoAccount and set
    InitAdvancedOrders,

    /// Add a trigger order which executes if the trigger condition is met.
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[]` mango_account_ai - the MangoAccount of owner
    /// 2. `[writable, signer]` owner_ai - owner of MangoAccount
    /// 3  `[writable]` advanced_orders_ai - the AdvanceOrdersAccount of owner
    /// 4. `[]` mango_cache_ai - MangoCache for this MangoGroup
    /// 5. `[]` perp_market_ai
    /// 6. `[]` system_prog_ai
    /// 7.. `[]` open_orders_ais - OpenOrders account for each serum dex market in margin basket
    AddPerpTriggerOrder {
        order_type: OrderType,
        side: Side,
        trigger_condition: TriggerCondition,
        reduce_only: bool, // only valid on perp order
        client_order_id: u64,
        price: i64,
        quantity: i64,
        trigger_price: I80F48,
    },
    /// Remove the order at the order_index
    RemoveAdvancedOrder {
        order_index: u8,
    },

    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[writable]` mango_account_ai - the MangoAccount of owner
    /// 2  `[writable]` advanced_orders_ai - the AdvanceOrdersAccount of owner
    /// 3. `[writable,signer]` agent_ai - operator of the execution service (receives lamports)
    /// 4. `[]` mango_cache_ai - MangoCache for this MangoGroup
    /// 5. `[writable]` perp_market_ai
    /// 6. `[writable]` bids_ai - bids account for this PerpMarket
    /// 7. `[writable]` asks_ai - asks account for this PerpMarket
    /// 8. `[writable]` event_queue_ai - EventQueue for this PerpMarket
    /// 9. `[] system_prog_ai
    ExecutePerpTriggerOrder {
        order_index: u8,
    },

    /// Create the necessary PDAs for the perp market and initialize them and add to MangoGroup
    ///
    /// Accounts expected by this instruction (13):
    ///
    /// 0. `[writable]` mango_group_ai
    /// 1. `[]` oracle_ai
    /// 2. `[writable]` perp_market_ai
    /// 3. `[writable]` event_queue_ai
    /// 4. `[writable]` bids_ai
    /// 5. `[writable]` asks_ai
    /// 6. `[]` mngo_mint_ai - mngo token mint
    /// 7. `[writable]` mngo_vault_ai - the vault from which liquidity incentives will be paid out for this market
    /// 8. `[signer, writable]` admin_ai - writable if admin_ai is also funder
    /// 9. `[writable]` signer_ai - optionally writable if funder is signer_ai
    /// 10. `[]` system_prog_ai - system program
    /// 11. `[]` token_prog_ai - SPL token program
    /// 12. `[]` rent_ai - rent sysvar because SPL token program requires it
    CreatePerpMarket {
        maint_leverage: I80F48,
        init_leverage: I80F48,
        liquidation_fee: I80F48,
        maker_fee: I80F48,
        taker_fee: I80F48,
        base_lot_size: i64,
        quote_lot_size: i64,
        /// Starting rate for liquidity mining
        rate: I80F48,
        /// v0: depth in bps for liquidity mining; v1: depth in contract size
        max_depth_bps: I80F48,
        /// target length in seconds of one period
        target_period_length: u64,
        /// amount MNGO rewarded per period
        mngo_per_period: u64,
        exp: u8,
        version: u8,
        /// Helps with integer overflow
        lm_size_shift: u8,
        /// define base decimals in case spot market has not yet been listed
        base_decimals: u8,
    },

    /// Change the params for perp market.
    ///
    /// Accounts expected by this instruction (3):
    /// 0. `[writable]` mango_group_ai - MangoGroup
    /// 1. `[writable]` perp_market_ai - PerpMarket
    /// 2. `[signer]` admin_ai - MangoGroup admin
    ChangePerpMarketParams2 {
        #[serde(serialize_with = "serialize_option_fixed_width")]
        maint_leverage: Option<I80F48>,

        #[serde(serialize_with = "serialize_option_fixed_width")]
        init_leverage: Option<I80F48>,

        #[serde(serialize_with = "serialize_option_fixed_width")]
        liquidation_fee: Option<I80F48>,

        #[serde(serialize_with = "serialize_option_fixed_width")]
        maker_fee: Option<I80F48>,

        #[serde(serialize_with = "serialize_option_fixed_width")]
        taker_fee: Option<I80F48>,

        /// Starting rate for liquidity mining
        #[serde(serialize_with = "serialize_option_fixed_width")]
        rate: Option<I80F48>,

        /// depth liquidity mining works for
        #[serde(serialize_with = "serialize_option_fixed_width")]
        max_depth_bps: Option<I80F48>,

        /// target length in seconds of one period
        #[serde(serialize_with = "serialize_option_fixed_width")]
        target_period_length: Option<u64>,

        /// amount MNGO rewarded per period
        #[serde(serialize_with = "serialize_option_fixed_width")]
        mngo_per_period: Option<u64>,

        #[serde(serialize_with = "serialize_option_fixed_width")]
        exp: Option<u8>,
        #[serde(serialize_with = "serialize_option_fixed_width")]
        version: Option<u8>,
        #[serde(serialize_with = "serialize_option_fixed_width")]
        lm_size_shift: Option<u8>,
    },

    /// Change the params for perp market.
    ///
    /// Accounts expected by this instruction (2 + MAX_PAIRS):
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[writable]` mango_account_ai - MangoAccount
    /// 2+ `[]` open_orders_ais - An array of MAX_PAIRS. Only OpenOrders of current market
    ///         index needs to be writable. Only OpenOrders in_margin_basket needs to be correct;
    ///         remaining open orders can just be Pubkey::default() (the zero key)
    UpdateMarginBasket,

    /// Change the maximum number of closeable MangoAccounts.v1 allowed
    ///
    /// Accounts expected by this instruction (2):
    ///
    /// 0. `[writable]` mango_group_ai - MangoGroup
    /// 1. `[signer]` admin_ai - Admin
    ChangeMaxMangoAccounts {
        max_mango_accounts: u32,
    },
    /// Delete a mango account and return lamports
    ///
    /// Accounts expected by this instruction (3):
    ///
    /// 0. `[writable]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[writable]` mango_account_ai - the mango account data
    /// 2. `[signer]` owner_ai - Solana account of owner of the mango account
    CloseMangoAccount, // instruction 50

    /// Delete a spot open orders account and return lamports
    ///
    /// Accounts expected by this instruction (7):
    ///
    /// 0. `[]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[writable]` mango_account_ai - the mango account data
    /// 2. `[signer, writable]` owner_ai - Solana account of owner of the mango account
    /// 3. `[]` dex_prog_ai - The serum dex program id
    /// 4. `[writable]` open_orders_ai - The open orders account to close
    /// 5. `[]` spot_market_ai - The spot market for the account
    /// 6. `[]` signer_ai - Mango group signer key
    CloseSpotOpenOrders,

    /// Delete an advanced orders account and return lamports
    ///
    /// Accounts expected by this instruction (4):
    ///
    /// 0. `[]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[writable]` mango_account_ai - the mango account data
    /// 2. `[signer, writable]` owner_ai - Solana account of owner of the mango account
    /// 3. `[writable]` advanced_orders_ai - the advanced orders account
    CloseAdvancedOrders,

    /// Create a PDA Mango Account for collecting dust owned by a group
    ///
    /// Accounts expected by this instruction (4)
    /// 0. `[]` mango_group_ai - MangoGroup to create the dust account for
    /// 1. `[writable]` mango_account_ai - the mango account data
    /// 2. `[signer, writable]` signer_ai - Signer and fee payer account
    /// 3. `[writable]` system_prog_ai - System program
    CreateDustAccount,

    /// Transfer dust (< 1 native SPL token) assets and liabilities for a single token to the group's dust account
    ///
    /// Accounts expected by this instruction (7)
    ///
    /// 0. `[]` mango_group_ai - MangoGroup of the mango account
    /// 1. `[writable]` mango_account_ai - the mango account data
    /// 2. `[signer, writable]` owner_ai - Solana account of owner of the mango account
    /// 3. `[writable]` dust_account_ai - Dust Account for the group
    /// 4. `[]` root_bank_ai - The root bank for the token
    /// 5. `[writable]` node_bank_ai - A node bank for the token
    /// 6. `[]` mango_cache_ai - The cache for the group
    ResolveDust,

    /// Create a PDA mango account for a user
    ///
    /// Accounts expected by this instruction (5):
    ///
    /// 0. `[writable]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[writable]` mango_account_ai - the mango account data
    /// 2. `[signer]` owner_ai - Solana account of owner of the mango account
    /// 3. `[]` system_prog_ai - System program
    /// 4. `[signer, writable]` payer_ai - pays for the PDA creation
    CreateMangoAccount {
        account_num: u64,
    },

    /// Upgrade a V0 Mango Account to V1 allowing it to be closed
    ///
    /// Accounts expected by this instruction (3):
    ///
    /// 0. `[writable]` mango_group_ai - MangoGroup
    /// 1. `[writable]` mango_account_ai - MangoAccount
    /// 2. `[signer]` owner_ai - Solana account of owner of the mango account
    UpgradeMangoAccountV0V1,

    /// Cancel all perp open orders for one side of the book
    ///
    /// Accounts expected: 6
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[writable]` mango_account_ai - MangoAccount
    /// 2. `[signer]` owner_ai - Owner of Mango Account
    /// 3. `[writable]` perp_market_ai - PerpMarket
    /// 4. `[writable]` bids_ai - Bids acc
    /// 5. `[writable]` asks_ai - Asks acc
    CancelPerpOrdersSide {
        side: Side,
        limit: u8,
    },

    /// https://github.com/blockworks-foundation/mango-v3/pull/97/
    /// Set delegate authority to mango account which can do everything regular account can do
    /// except Withdraw and CloseMangoAccount. Set to Pubkey::default() to revoke delegate
    ///
    /// Accounts expected: 4
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[writable]` mango_account_ai - MangoAccount
    /// 2. `[signer]` owner_ai - Owner of Mango Account
    /// 3. `[]` delegate_ai - delegate
    SetDelegate,

    /// Change the params for a spot market.
    ///
    /// Accounts expected by this instruction (4):
    /// 0. `[writable]` mango_group_ai - MangoGroup
    /// 1. `[writable]` spot_market_ai - Market
    /// 2. `[writable]` root_bank_ai - RootBank
    /// 3. `[signer]` admin_ai - MangoGroup admin
    ChangeSpotMarketParams {
        #[serde(serialize_with = "serialize_option_fixed_width")]
        maint_leverage: Option<I80F48>,

        #[serde(serialize_with = "serialize_option_fixed_width")]
        init_leverage: Option<I80F48>,

        #[serde(serialize_with = "serialize_option_fixed_width")]
        liquidation_fee: Option<I80F48>,

        #[serde(serialize_with = "serialize_option_fixed_width")]
        optimal_util: Option<I80F48>,

        #[serde(serialize_with = "serialize_option_fixed_width")]
        optimal_rate: Option<I80F48>,

        #[serde(serialize_with = "serialize_option_fixed_width")]
        max_rate: Option<I80F48>,

        #[serde(serialize_with = "serialize_option_fixed_width")]
        version: Option<u8>,
    },

    /// Create an OpenOrders PDA and initialize it with InitOpenOrders call to serum dex
    ///
    /// Accounts expected by this instruction (9):
    ///
    /// 0. `[]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[writable]` mango_account_ai - MangoAccount
    /// 2. `[signer]` owner_ai - MangoAccount owner
    /// 3. `[]` dex_prog_ai - program id of serum dex
    /// 4. `[writable]` open_orders_ai - open orders PDA
    /// 5. `[]` spot_market_ai - dex MarketState account
    /// 6. `[]` signer_ai - Group Signer Account
    /// 7. `[]` system_prog_ai - System program
    /// 8. `[signer, writable]` payer_ai - pays for the PDA creation
    CreateSpotOpenOrders, // instruction 60

    /// Set the `ref_surcharge_centibps`, `ref_share_centibps` and `ref_mngo_required` on `MangoGroup`
    ///
    /// Accounts expected by this instruction (2):
    /// 0. `[writable]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[signer]` admin_ai - mango_group.admin
    ChangeReferralFeeParams {
        ref_surcharge_centibps: u32,
        ref_share_centibps: u32,
        ref_mngo_required: u64,
    },
    /// Store the referrer's MangoAccount pubkey on the Referrer account
    /// It will create the Referrer account as a PDA of user's MangoAccount if it doesn't exist
    /// This is primarily useful for the UI; the referrer address stored here is not necessarily
    /// who earns the ref fees.
    ///
    /// Accounts expected by this instruction (7):
    ///
    /// 0. `[]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[]` mango_account_ai - MangoAccount of the referred
    /// 2. `[signer]` owner_ai - MangoAccount owner or delegate
    /// 3. `[writable]` referrer_memory_ai - ReferrerMemory struct; will be initialized if required
    /// 4. `[]` referrer_mango_account_ai - referrer's MangoAccount
    /// 5. `[signer, writable]` payer_ai - payer for PDA; can be same as owner
    /// 6. `[]` system_prog_ai - System program
    SetReferrerMemory,

    /// Associate the referrer's MangoAccount with a human readable `referrer_id` which can be used
    /// in a ref link. This is primarily useful for the UI.
    /// Create the `ReferrerIdRecord` PDA; if it already exists throw error
    ///
    /// Accounts expected by this instruction (5):
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[]` referrer_mango_account_ai - MangoAccount
    /// 2. `[writable]` referrer_id_record_ai - The PDA to store the record on
    /// 3. `[signer, writable]` payer_ai - payer for PDA; can be same as owner
    /// 4. `[]` system_prog_ai - System program
    RegisterReferrerId {
        referrer_id: [u8; INFO_LEN],
    },

    /// Place an order on a perp market
    ///
    /// In case this order is matched, the corresponding order structs on both
    /// PerpAccounts (taker & maker) will be adjusted, and the position size
    /// will be adjusted w/o accounting for fees.
    /// In addition a FillEvent will be placed on the event queue.
    /// Through a subsequent invocation of ConsumeEvents the FillEvent can be
    /// executed and the perp account balances (base/quote) and fees will be
    /// paid from the quote position. Only at this point the position balance
    /// is 100% reflecting the trade.
    ///
    /// Accounts expected by this instruction (9 + `NUM_IN_MARGIN_BASKET`):
    /// 0. `[]` mango_group_ai - MangoGroup
    /// 1. `[writable]` mango_account_ai - the MangoAccount of owner
    /// 2. `[signer]` owner_ai - owner of MangoAccount
    /// 3. `[]` mango_cache_ai - MangoCache for this MangoGroup
    /// 4. `[writable]` perp_market_ai
    /// 5. `[writable]` bids_ai - bids account for this PerpMarket
    /// 6. `[writable]` asks_ai - asks account for this PerpMarket
    /// 7. `[writable]` event_queue_ai - EventQueue for this PerpMarket
    /// 8. `[writable]` referrer_mango_account_ai - referrer's mango account;
    ///                 pass in mango_account_ai as duplicate if you don't have a referrer
    /// 9..9 + NUM_IN_MARGIN_BASKET `[]` open_orders_ais - pass in open orders in margin basket
    PlacePerpOrder2 {
        /// Price in quote lots per base lots.
        ///
        /// Effect is based on order type, it's usually
        /// - fill orders on the book up to this price or
        /// - place an order on the book at this price.
        ///
        /// Ignored for Market orders and potentially adjusted for PostOnlySlide orders.
        price: i64,

        /// Max base lots to buy/sell.
        max_base_quantity: i64,

        /// Max quote lots to pay/receive (not taking fees into account).
        max_quote_quantity: i64,

        /// Arbitrary user-controlled order id.
        client_order_id: u64,

        /// Timestamp of when order expires
        ///
        /// Send 0 if you want the order to never expire.
        /// Timestamps in the past mean the instruction is skipped.
        /// Timestamps in the future are reduced to now + 255s.
        expiry_timestamp: u64,

        side: Side,

        /// Can be 0 -> LIMIT, 1 -> IOC, 2 -> PostOnly, 3 -> Market, 4 -> PostOnlySlide
        order_type: OrderType,

        reduce_only: bool,

        /// Maximum number of orders from the book to fill.
        ///
        /// Use this to limit compute used during order matching.
        /// When the limit is reached, processing stops and the instruction succeeds.
        limit: u8,
    },
}

impl MangoInstruction {
    pub fn unpack(input: &[u8]) -> Option<Self> {
        let (&discrim, data) = array_refs![input, 4; ..;];
        let discrim = u32::from_le_bytes(discrim);
        Some(match discrim {
            0 => {
                let data = array_ref![data, 0, 64];
                let (
                    signer_nonce,
                    valid_interval,
                    quote_optimal_util,
                    quote_optimal_rate,
                    quote_max_rate,
                ) = array_refs![data, 8, 8, 16, 16, 16];

                MangoInstruction::InitMangoGroup {
                    signer_nonce: u64::from_le_bytes(*signer_nonce),
                    valid_interval: u64::from_le_bytes(*valid_interval),
                    quote_optimal_util: I80F48::from_le_bytes(*quote_optimal_util),
                    quote_optimal_rate: I80F48::from_le_bytes(*quote_optimal_rate),
                    quote_max_rate: I80F48::from_le_bytes(*quote_max_rate),
                }
            }
            1 => MangoInstruction::InitMangoAccount,
            2 => {
                let quantity = array_ref![data, 0, 8];
                MangoInstruction::Deposit { quantity: u64::from_le_bytes(*quantity) }
            }
            3 => {
                let data = array_ref![data, 0, 9];
                let (quantity, allow_borrow) = array_refs![data, 8, 1];

                let allow_borrow = match allow_borrow {
                    [0] => false,
                    [1] => true,
                    _ => return None,
                };
                MangoInstruction::Withdraw { quantity: u64::from_le_bytes(*quantity), allow_borrow }
            }
            4 => {
                let data = array_ref![data, 0, 96];
                let (
                    maint_leverage,
                    init_leverage,
                    liquidation_fee,
                    optimal_util,
                    optimal_rate,
                    max_rate,
                ) = array_refs![data, 16, 16, 16, 16, 16, 16];
                MangoInstruction::AddSpotMarket {
                    maint_leverage: I80F48::from_le_bytes(*maint_leverage),
                    init_leverage: I80F48::from_le_bytes(*init_leverage),
                    liquidation_fee: I80F48::from_le_bytes(*liquidation_fee),
                    optimal_util: I80F48::from_le_bytes(*optimal_util),
                    optimal_rate: I80F48::from_le_bytes(*optimal_rate),
                    max_rate: I80F48::from_le_bytes(*max_rate),
                }
            }
            5 => {
                let market_index = array_ref![data, 0, 8];
                MangoInstruction::AddToBasket { market_index: usize::from_le_bytes(*market_index) }
            }
            6 => {
                let quantity = array_ref![data, 0, 8];
                MangoInstruction::Borrow { quantity: u64::from_le_bytes(*quantity) }
            }
            7 => MangoInstruction::CachePrices,
            8 => MangoInstruction::CacheRootBanks,
            9 => {
                let data_arr = array_ref![data, 0, 46];
                let order = unpack_dex_new_order_v3(data_arr)?;
                MangoInstruction::PlaceSpotOrder { order }
            }
            10 => MangoInstruction::AddOracle,
            11 => {
                let exp = if data.len() > 144 { data[144] } else { 2 };
                let data_arr = array_ref![data, 0, 144];
                let (
                    maint_leverage,
                    init_leverage,
                    liquidation_fee,
                    maker_fee,
                    taker_fee,
                    base_lot_size,
                    quote_lot_size,
                    rate,
                    max_depth_bps,
                    target_period_length,
                    mngo_per_period,
                ) = array_refs![data_arr, 16, 16, 16, 16, 16, 8, 8, 16, 16, 8, 8];
                MangoInstruction::AddPerpMarket {
                    maint_leverage: I80F48::from_le_bytes(*maint_leverage),
                    init_leverage: I80F48::from_le_bytes(*init_leverage),
                    liquidation_fee: I80F48::from_le_bytes(*liquidation_fee),
                    maker_fee: I80F48::from_le_bytes(*maker_fee),
                    taker_fee: I80F48::from_le_bytes(*taker_fee),
                    base_lot_size: i64::from_le_bytes(*base_lot_size),
                    quote_lot_size: i64::from_le_bytes(*quote_lot_size),
                    rate: I80F48::from_le_bytes(*rate),
                    max_depth_bps: I80F48::from_le_bytes(*max_depth_bps),
                    target_period_length: u64::from_le_bytes(*target_period_length),
                    mngo_per_period: u64::from_le_bytes(*mngo_per_period),
                    exp,
                }
            }
            12 => {
                let reduce_only = if data.len() > 26 { data[26] != 0 } else { false };
                let data_arr = array_ref![data, 0, 26];
                let (price, quantity, client_order_id, side, order_type) =
                    array_refs![data_arr, 8, 8, 8, 1, 1];
                MangoInstruction::PlacePerpOrder {
                    price: i64::from_le_bytes(*price),
                    quantity: i64::from_le_bytes(*quantity),
                    client_order_id: u64::from_le_bytes(*client_order_id),
                    side: Side::try_from_primitive(side[0]).ok()?,
                    order_type: OrderType::try_from_primitive(order_type[0]).ok()?,
                    reduce_only,
                }
            }
            13 => {
                let data_arr = array_ref![data, 0, 9];
                let (client_order_id, invalid_id_ok) = array_refs![data_arr, 8, 1];

                MangoInstruction::CancelPerpOrderByClientId {
                    client_order_id: u64::from_le_bytes(*client_order_id),
                    invalid_id_ok: invalid_id_ok[0] != 0,
                }
            }
            14 => {
                let data_arr = array_ref![data, 0, 17];
                let (order_id, invalid_id_ok) = array_refs![data_arr, 16, 1];
                MangoInstruction::CancelPerpOrder {
                    order_id: i128::from_le_bytes(*order_id),
                    invalid_id_ok: invalid_id_ok[0] != 0,
                }
            }
            15 => {
                let data_arr = array_ref![data, 0, 8];
                MangoInstruction::ConsumeEvents { limit: usize::from_le_bytes(*data_arr) }
            }
            16 => MangoInstruction::CachePerpMarkets,
            17 => MangoInstruction::UpdateFunding,
            18 => {
                let data_arr = array_ref![data, 0, 16];
                MangoInstruction::SetOracle { price: I80F48::from_le_bytes(*data_arr) }
            }
            19 => MangoInstruction::SettleFunds,
            20 => {
                let data_array = array_ref![data, 0, 20];
                let fields = array_refs![data_array, 4, 16];
                let side = match u32::from_le_bytes(*fields.0) {
                    0 => serum_dex::matching::Side::Bid,
                    1 => serum_dex::matching::Side::Ask,
                    _ => return None,
                };
                let order_id = u128::from_le_bytes(*fields.1);
                let order = serum_dex::instruction::CancelOrderInstructionV2 { side, order_id };
                MangoInstruction::CancelSpotOrder { order }
            }
            21 => MangoInstruction::UpdateRootBank,

            22 => {
                let data_arr = array_ref![data, 0, 8];

                MangoInstruction::SettlePnl { market_index: usize::from_le_bytes(*data_arr) }
            }
            23 => {
                let data = array_ref![data, 0, 16];
                let (token_index, quantity) = array_refs![data, 8, 8];

                MangoInstruction::SettleBorrow {
                    token_index: usize::from_le_bytes(*token_index),
                    quantity: u64::from_le_bytes(*quantity),
                }
            }
            24 => {
                let data_arr = array_ref![data, 0, 1];

                MangoInstruction::ForceCancelSpotOrders { limit: u8::from_le_bytes(*data_arr) }
            }
            25 => {
                let data_arr = array_ref![data, 0, 1];

                MangoInstruction::ForceCancelPerpOrders { limit: u8::from_le_bytes(*data_arr) }
            }
            26 => {
                let data_arr = array_ref![data, 0, 16];

                MangoInstruction::LiquidateTokenAndToken {
                    max_liab_transfer: I80F48::from_le_bytes(*data_arr),
                }
            }
            27 => {
                let data = array_ref![data, 0, 34];
                let (asset_type, asset_index, liab_type, liab_index, max_liab_transfer) =
                    array_refs![data, 1, 8, 1, 8, 16];

                MangoInstruction::LiquidateTokenAndPerp {
                    asset_type: AssetType::try_from(u8::from_le_bytes(*asset_type)).unwrap(),
                    asset_index: usize::from_le_bytes(*asset_index),
                    liab_type: AssetType::try_from(u8::from_le_bytes(*liab_type)).unwrap(),
                    liab_index: usize::from_le_bytes(*liab_index),
                    max_liab_transfer: I80F48::from_le_bytes(*max_liab_transfer),
                }
            }
            28 => {
                let data_arr = array_ref![data, 0, 8];

                MangoInstruction::LiquidatePerpMarket {
                    base_transfer_request: i64::from_le_bytes(*data_arr),
                }
            }
            29 => MangoInstruction::SettleFees,
            30 => {
                let data = array_ref![data, 0, 24];
                let (liab_index, max_liab_transfer) = array_refs![data, 8, 16];

                MangoInstruction::ResolvePerpBankruptcy {
                    liab_index: usize::from_le_bytes(*liab_index),
                    max_liab_transfer: I80F48::from_le_bytes(*max_liab_transfer),
                }
            }
            31 => {
                let data_arr = array_ref![data, 0, 16];

                MangoInstruction::ResolveTokenBankruptcy {
                    max_liab_transfer: I80F48::from_le_bytes(*data_arr),
                }
            }
            32 => MangoInstruction::InitSpotOpenOrders,
            33 => MangoInstruction::RedeemMngo,
            34 => {
                let info = array_ref![data, 0, INFO_LEN];
                MangoInstruction::AddMangoAccountInfo { info: *info }
            }
            35 => {
                let quantity = array_ref![data, 0, 8];
                MangoInstruction::DepositMsrm { quantity: u64::from_le_bytes(*quantity) }
            }
            36 => {
                let quantity = array_ref![data, 0, 8];
                MangoInstruction::WithdrawMsrm { quantity: u64::from_le_bytes(*quantity) }
            }

            37 => {
                let exp =
                    if data.len() > 137 { unpack_u8_opt(&[data[137], data[138]]) } else { None };
                let data_arr = array_ref![data, 0, 137];
                let (
                    maint_leverage,
                    init_leverage,
                    liquidation_fee,
                    maker_fee,
                    taker_fee,
                    rate,
                    max_depth_bps,
                    target_period_length,
                    mngo_per_period,
                ) = array_refs![data_arr, 17, 17, 17, 17, 17, 17, 17, 9, 9];

                MangoInstruction::ChangePerpMarketParams {
                    maint_leverage: unpack_i80f48_opt(maint_leverage),
                    init_leverage: unpack_i80f48_opt(init_leverage),
                    liquidation_fee: unpack_i80f48_opt(liquidation_fee),
                    maker_fee: unpack_i80f48_opt(maker_fee),
                    taker_fee: unpack_i80f48_opt(taker_fee),
                    rate: unpack_i80f48_opt(rate),
                    max_depth_bps: unpack_i80f48_opt(max_depth_bps),
                    target_period_length: unpack_u64_opt(target_period_length),
                    mngo_per_period: unpack_u64_opt(mngo_per_period),
                    exp,
                }
            }

            38 => MangoInstruction::SetGroupAdmin,

            39 => {
                let data_arr = array_ref![data, 0, 1];
                MangoInstruction::CancelAllPerpOrders { limit: u8::from_le_bytes(*data_arr) }
            }

            40 => MangoInstruction::ForceSettleQuotePositions,
            41 => {
                let data_arr = array_ref![data, 0, 46];
                let order = unpack_dex_new_order_v3(data_arr)?;
                MangoInstruction::PlaceSpotOrder2 { order }
            }

            42 => MangoInstruction::InitAdvancedOrders,

            43 => {
                let data_arr = array_ref![data, 0, 44];
                let (
                    order_type,
                    side,
                    trigger_condition,
                    reduce_only,
                    client_order_id,
                    price,
                    quantity,
                    trigger_price,
                ) = array_refs![data_arr, 1, 1, 1, 1, 8, 8, 8, 16];
                MangoInstruction::AddPerpTriggerOrder {
                    order_type: OrderType::try_from_primitive(order_type[0]).ok()?,
                    side: Side::try_from_primitive(side[0]).ok()?,
                    trigger_condition: TriggerCondition::try_from(u8::from_le_bytes(
                        *trigger_condition,
                    ))
                    .unwrap(),
                    reduce_only: reduce_only[0] != 0,
                    client_order_id: u64::from_le_bytes(*client_order_id),
                    price: i64::from_le_bytes(*price),
                    quantity: i64::from_le_bytes(*quantity),
                    trigger_price: I80F48::from_le_bytes(*trigger_price),
                }
            }

            44 => {
                let order_index = array_ref![data, 0, 1][0];
                MangoInstruction::RemoveAdvancedOrder { order_index }
            }
            45 => {
                let order_index = array_ref![data, 0, 1][0];
                MangoInstruction::ExecutePerpTriggerOrder { order_index }
            }
            46 => {
                let data_arr = array_ref![data, 0, 148];
                let (
                    maint_leverage,
                    init_leverage,
                    liquidation_fee,
                    maker_fee,
                    taker_fee,
                    base_lot_size,
                    quote_lot_size,
                    rate,
                    max_depth_bps,
                    target_period_length,
                    mngo_per_period,
                    exp,
                    version,
                    lm_size_shift,
                    base_decimals,
                ) = array_refs![data_arr, 16, 16, 16, 16, 16, 8, 8, 16, 16, 8, 8, 1, 1, 1, 1];
                MangoInstruction::CreatePerpMarket {
                    maint_leverage: I80F48::from_le_bytes(*maint_leverage),
                    init_leverage: I80F48::from_le_bytes(*init_leverage),
                    liquidation_fee: I80F48::from_le_bytes(*liquidation_fee),
                    maker_fee: I80F48::from_le_bytes(*maker_fee),
                    taker_fee: I80F48::from_le_bytes(*taker_fee),
                    base_lot_size: i64::from_le_bytes(*base_lot_size),
                    quote_lot_size: i64::from_le_bytes(*quote_lot_size),
                    rate: I80F48::from_le_bytes(*rate),
                    max_depth_bps: I80F48::from_le_bytes(*max_depth_bps),
                    target_period_length: u64::from_le_bytes(*target_period_length),
                    mngo_per_period: u64::from_le_bytes(*mngo_per_period),
                    exp: exp[0],
                    version: version[0],
                    lm_size_shift: lm_size_shift[0],
                    base_decimals: base_decimals[0],
                }
            }
            47 => {
                let data_arr = array_ref![data, 0, 143];
                let (
                    maint_leverage,
                    init_leverage,
                    liquidation_fee,
                    maker_fee,
                    taker_fee,
                    rate,
                    max_depth_bps,
                    target_period_length,
                    mngo_per_period,
                    exp,
                    version,
                    lm_size_shift,
                ) = array_refs![data_arr, 17, 17, 17, 17, 17, 17, 17, 9, 9, 2, 2, 2];

                MangoInstruction::ChangePerpMarketParams2 {
                    maint_leverage: unpack_i80f48_opt(maint_leverage),
                    init_leverage: unpack_i80f48_opt(init_leverage),
                    liquidation_fee: unpack_i80f48_opt(liquidation_fee),
                    maker_fee: unpack_i80f48_opt(maker_fee),
                    taker_fee: unpack_i80f48_opt(taker_fee),
                    rate: unpack_i80f48_opt(rate),
                    max_depth_bps: unpack_i80f48_opt(max_depth_bps),
                    target_period_length: unpack_u64_opt(target_period_length),
                    mngo_per_period: unpack_u64_opt(mngo_per_period),
                    exp: unpack_u8_opt(exp),
                    version: unpack_u8_opt(version),
                    lm_size_shift: unpack_u8_opt(lm_size_shift),
                }
            }
            48 => MangoInstruction::UpdateMarginBasket,
            49 => {
                let data_arr = array_ref![data, 0, 4];
                MangoInstruction::ChangeMaxMangoAccounts {
                    max_mango_accounts: u32::from_le_bytes(*data_arr),
                }
            }
            50 => MangoInstruction::CloseMangoAccount,
            51 => MangoInstruction::CloseSpotOpenOrders,
            52 => MangoInstruction::CloseAdvancedOrders,
            53 => MangoInstruction::CreateDustAccount,
            54 => MangoInstruction::ResolveDust,
            55 => {
                let account_num = array_ref![data, 0, 8];
                MangoInstruction::CreateMangoAccount {
                    account_num: u64::from_le_bytes(*account_num),
                }
            }
            56 => MangoInstruction::UpgradeMangoAccountV0V1,
            57 => {
                let data_arr = array_ref![data, 0, 2];
                let (side, limit) = array_refs![data_arr, 1, 1];

                MangoInstruction::CancelPerpOrdersSide {
                    side: Side::try_from_primitive(side[0]).ok()?,
                    limit: u8::from_le_bytes(*limit),
                }
            }
            58 => MangoInstruction::SetDelegate,
            59 => {
                let data_arr = array_ref![data, 0, 104];
                let (
                    maint_leverage,
                    init_leverage,
                    liquidation_fee,
                    optimal_util,
                    optimal_rate,
                    max_rate,
                    version,
                ) = array_refs![data_arr, 17, 17, 17, 17, 17, 17, 2];

                MangoInstruction::ChangeSpotMarketParams {
                    maint_leverage: unpack_i80f48_opt(maint_leverage),
                    init_leverage: unpack_i80f48_opt(init_leverage),
                    liquidation_fee: unpack_i80f48_opt(liquidation_fee),
                    optimal_util: unpack_i80f48_opt(optimal_util),
                    optimal_rate: unpack_i80f48_opt(optimal_rate),
                    max_rate: unpack_i80f48_opt(max_rate),
                    version: unpack_u8_opt(version),
                }
            }
            60 => MangoInstruction::CreateSpotOpenOrders,
            61 => {
                let data = array_ref![data, 0, 16];
                let (ref_surcharge_centibps, ref_share_centibps, ref_mngo_required) =
                    array_refs![data, 4, 4, 8];
                MangoInstruction::ChangeReferralFeeParams {
                    ref_surcharge_centibps: u32::from_le_bytes(*ref_surcharge_centibps),
                    ref_share_centibps: u32::from_le_bytes(*ref_share_centibps),
                    ref_mngo_required: u64::from_le_bytes(*ref_mngo_required),
                }
            }
            62 => MangoInstruction::SetReferrerMemory,
            63 => {
                let referrer_id = array_ref![data, 0, INFO_LEN];
                MangoInstruction::RegisterReferrerId { referrer_id: *referrer_id }
            }
            64 => {
                let data_arr = array_ref![data, 0, 44];
                let (
                    price,
                    max_base_quantity,
                    max_quote_quantity,
                    client_order_id,
                    expiry_timestamp,
                    side,
                    order_type,
                    reduce_only,
                    limit,
                ) = array_refs![data_arr, 8, 8, 8, 8, 8, 1, 1, 1, 1];
                MangoInstruction::PlacePerpOrder2 {
                    price: i64::from_le_bytes(*price),
                    max_base_quantity: i64::from_le_bytes(*max_base_quantity),
                    max_quote_quantity: i64::from_le_bytes(*max_quote_quantity),
                    client_order_id: u64::from_le_bytes(*client_order_id),
                    expiry_timestamp: u64::from_le_bytes(*expiry_timestamp),
                    side: Side::try_from_primitive(side[0]).ok()?,
                    order_type: OrderType::try_from_primitive(order_type[0]).ok()?,
                    reduce_only: reduce_only[0] != 0,
                    limit: u8::from_le_bytes(*limit),
                }
            }
            _ => {
                return None;
            }
        })
    }
    pub fn pack(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap()
    }
}

fn unpack_u8_opt(data: &[u8; 2]) -> Option<u8> {
    if data[0] == 0 {
        None
    } else {
        Some(data[1])
    }
}

fn unpack_i80f48_opt(data: &[u8; 17]) -> Option<I80F48> {
    let (opt, val) = array_refs![data, 1, 16];
    if opt[0] == 0 {
        None
    } else {
        Some(I80F48::from_le_bytes(*val))
    }
}
fn unpack_u64_opt(data: &[u8; 9]) -> Option<u64> {
    let (opt, val) = array_refs![data, 1, 8];
    if opt[0] == 0 {
        None
    } else {
        Some(u64::from_le_bytes(*val))
    }
}

fn unpack_dex_new_order_v3(
    data: &[u8; 46],
) -> Option<serum_dex::instruction::NewOrderInstructionV3> {
    let (
        &side_arr,
        &price_arr,
        &max_coin_qty_arr,
        &max_native_pc_qty_arr,
        &self_trade_behavior_arr,
        &otype_arr,
        &client_order_id_bytes,
        &limit_arr,
    ) = array_refs![data, 4, 8, 8, 8, 4, 4, 8, 2];

    let side = serum_dex::matching::Side::try_from_primitive(
        u32::from_le_bytes(side_arr).try_into().ok()?,
    )
    .ok()?;
    let limit_price = NonZeroU64::new(u64::from_le_bytes(price_arr))?;
    let max_coin_qty = NonZeroU64::new(u64::from_le_bytes(max_coin_qty_arr))?;
    let max_native_pc_qty_including_fees =
        NonZeroU64::new(u64::from_le_bytes(max_native_pc_qty_arr))?;
    let self_trade_behavior = serum_dex::instruction::SelfTradeBehavior::try_from_primitive(
        u32::from_le_bytes(self_trade_behavior_arr).try_into().ok()?,
    )
    .ok()?;
    let order_type = serum_dex::matching::OrderType::try_from_primitive(
        u32::from_le_bytes(otype_arr).try_into().ok()?,
    )
    .ok()?;
    let client_order_id = u64::from_le_bytes(client_order_id_bytes);
    let limit = u16::from_le_bytes(limit_arr);

    Some(serum_dex::instruction::NewOrderInstructionV3 {
        side,
        limit_price,
        max_coin_qty,
        max_native_pc_qty_including_fees,
        self_trade_behavior,
        order_type,
        client_order_id,
        limit,
    })
}

pub fn init_mango_group(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    signer_pk: &Pubkey,
    admin_pk: &Pubkey,
    quote_mint_pk: &Pubkey,
    quote_vault_pk: &Pubkey,
    quote_node_bank_pk: &Pubkey,
    quote_root_bank_pk: &Pubkey,
    insurance_vault_pk: &Pubkey,
    msrm_vault_pk: &Pubkey, // send in Pubkey:default() if not using this feature
    fees_vault_pk: &Pubkey,
    mango_cache_ai: &Pubkey,
    dex_program_pk: &Pubkey,

    signer_nonce: u64,
    valid_interval: u64,
    quote_optimal_util: I80F48,
    quote_optimal_rate: I80F48,
    quote_max_rate: I80F48,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*mango_group_pk, false),
        AccountMeta::new_readonly(*signer_pk, false),
        AccountMeta::new_readonly(*admin_pk, true),
        AccountMeta::new_readonly(*quote_mint_pk, false),
        AccountMeta::new_readonly(*quote_vault_pk, false),
        AccountMeta::new(*quote_node_bank_pk, false),
        AccountMeta::new(*quote_root_bank_pk, false),
        AccountMeta::new_readonly(*insurance_vault_pk, false),
        AccountMeta::new_readonly(*msrm_vault_pk, false),
        AccountMeta::new_readonly(*fees_vault_pk, false),
        AccountMeta::new(*mango_cache_ai, false),
        AccountMeta::new_readonly(*dex_program_pk, false),
    ];

    let instr = MangoInstruction::InitMangoGroup {
        signer_nonce,
        valid_interval,
        quote_optimal_util,
        quote_optimal_rate,
        quote_max_rate,
    };

    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn init_mango_account(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    owner_pk: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
    ];

    let instr = MangoInstruction::InitMangoAccount;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn close_mango_account(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    owner_pk: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
    ];

    let instr = MangoInstruction::CloseMangoAccount;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn create_mango_account(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    owner_pk: &Pubkey,
    system_prog_pk: &Pubkey,
    payer_pk: &Pubkey,
    account_num: u64,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new_readonly(*system_prog_pk, false),
        AccountMeta::new(*payer_pk, true),
    ];

    let instr = MangoInstruction::CreateMangoAccount { account_num };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn set_delegate(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    owner_pk: &Pubkey,
    delegate_pk: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new_readonly(*delegate_pk, false),
    ];

    let instr = MangoInstruction::SetDelegate {};
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn upgrade_mango_account_v0_v1(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    owner_pk: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
    ];

    let instr = MangoInstruction::UpgradeMangoAccountV0V1;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn deposit(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    owner_pk: &Pubkey,
    mango_cache_pk: &Pubkey,
    root_bank_pk: &Pubkey,
    node_bank_pk: &Pubkey,
    vault_pk: &Pubkey,
    owner_token_account_pk: &Pubkey,

    quantity: u64,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new_readonly(*mango_cache_pk, false),
        AccountMeta::new_readonly(*root_bank_pk, false),
        AccountMeta::new(*node_bank_pk, false),
        AccountMeta::new(*vault_pk, false),
        AccountMeta::new_readonly(spl_token::ID, false),
        AccountMeta::new(*owner_token_account_pk, false),
    ];

    let instr = MangoInstruction::Deposit { quantity };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn add_spot_market(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    oracle_pk: &Pubkey,
    spot_market_pk: &Pubkey,
    dex_program_pk: &Pubkey,
    token_mint_pk: &Pubkey,
    node_bank_pk: &Pubkey,
    vault_pk: &Pubkey,
    root_bank_pk: &Pubkey,
    admin_pk: &Pubkey,

    maint_leverage: I80F48,
    init_leverage: I80F48,
    liquidation_fee: I80F48,
    optimal_util: I80F48,
    optimal_rate: I80F48,
    max_rate: I80F48,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*mango_group_pk, false),
        AccountMeta::new_readonly(*oracle_pk, false),
        AccountMeta::new_readonly(*spot_market_pk, false),
        AccountMeta::new_readonly(*dex_program_pk, false),
        AccountMeta::new_readonly(*token_mint_pk, false),
        AccountMeta::new(*node_bank_pk, false),
        AccountMeta::new_readonly(*vault_pk, false),
        AccountMeta::new(*root_bank_pk, false),
        AccountMeta::new_readonly(*admin_pk, true),
    ];

    let instr = MangoInstruction::AddSpotMarket {
        maint_leverage,
        init_leverage,
        liquidation_fee,
        optimal_util,
        optimal_rate,
        max_rate,
    };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn add_perp_market(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    oracle_pk: &Pubkey,
    perp_market_pk: &Pubkey,
    event_queue_pk: &Pubkey,
    bids_pk: &Pubkey,
    asks_pk: &Pubkey,
    mngo_vault_pk: &Pubkey,
    admin_pk: &Pubkey,

    maint_leverage: I80F48,
    init_leverage: I80F48,
    liquidation_fee: I80F48,
    maker_fee: I80F48,
    taker_fee: I80F48,
    base_lot_size: i64,
    quote_lot_size: i64,
    rate: I80F48,
    max_depth_bps: I80F48,
    target_period_length: u64,
    mngo_per_period: u64,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*mango_group_pk, false),
        AccountMeta::new(*oracle_pk, false),
        AccountMeta::new(*perp_market_pk, false),
        AccountMeta::new(*event_queue_pk, false),
        AccountMeta::new(*bids_pk, false),
        AccountMeta::new(*asks_pk, false),
        AccountMeta::new_readonly(*mngo_vault_pk, false),
        AccountMeta::new_readonly(*admin_pk, true),
    ];

    let instr = MangoInstruction::AddPerpMarket {
        maint_leverage,
        init_leverage,
        liquidation_fee,
        maker_fee,
        taker_fee,
        base_lot_size,
        quote_lot_size,
        rate,
        max_depth_bps,
        target_period_length,
        mngo_per_period,
        exp: 2, // TODO add this to function signature
    };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn place_perp_order(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    owner_pk: &Pubkey,
    mango_cache_pk: &Pubkey,
    perp_market_pk: &Pubkey,
    bids_pk: &Pubkey,
    asks_pk: &Pubkey,
    event_queue_pk: &Pubkey,
    referrer_mango_account_pk: Option<&Pubkey>,
    open_orders_pks: &[Pubkey; MAX_PAIRS],
    side: Side,
    price: i64,
    quantity: i64,
    client_order_id: u64,
    order_type: OrderType,
    reduce_only: bool,
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new_readonly(*mango_cache_pk, false),
        AccountMeta::new(*perp_market_pk, false),
        AccountMeta::new(*bids_pk, false),
        AccountMeta::new(*asks_pk, false),
        AccountMeta::new(*event_queue_pk, false),
    ];
    accounts.extend(open_orders_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));
    if let Some(referrer_mango_account_pk) = referrer_mango_account_pk {
        accounts.push(AccountMeta::new(*referrer_mango_account_pk, false));
    }

    let instr = MangoInstruction::PlacePerpOrder {
        side,
        price,
        quantity,
        client_order_id,
        order_type,
        reduce_only,
    };
    let data = instr.pack();

    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn place_perp_order2(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    owner_pk: &Pubkey,
    mango_cache_pk: &Pubkey,
    perp_market_pk: &Pubkey,
    bids_pk: &Pubkey,
    asks_pk: &Pubkey,
    event_queue_pk: &Pubkey,
    referrer_mango_account_pk: Option<&Pubkey>,
    open_orders_pks: &[Pubkey],
    side: Side,
    price: i64,
    max_base_quantity: i64,
    max_quote_quantity: i64,
    client_order_id: u64,
    order_type: OrderType,
    reduce_only: bool,
    expiry_timestamp: Option<u64>, // Send 0 if you want to ignore time in force
    limit: u8,                     // maximum number of FillEvents before terminating
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new_readonly(*mango_cache_pk, false),
        AccountMeta::new(*perp_market_pk, false),
        AccountMeta::new(*bids_pk, false),
        AccountMeta::new(*asks_pk, false),
        AccountMeta::new(*event_queue_pk, false),
        AccountMeta::new(*referrer_mango_account_pk.unwrap_or(mango_account_pk), false),
    ];

    accounts.extend(open_orders_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));

    let instr = MangoInstruction::PlacePerpOrder2 {
        side,
        price,
        max_base_quantity,
        max_quote_quantity,
        client_order_id,
        order_type,
        reduce_only,
        expiry_timestamp: expiry_timestamp.unwrap_or(0),
        limit,
    };
    let data = instr.pack();

    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn cancel_perp_order_by_client_id(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,   // read
    mango_account_pk: &Pubkey, // write
    owner_pk: &Pubkey,         // read, signer
    perp_market_pk: &Pubkey,   // write
    bids_pk: &Pubkey,          // write
    asks_pk: &Pubkey,          // write
    client_order_id: u64,
    invalid_id_ok: bool,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new(*perp_market_pk, false),
        AccountMeta::new(*bids_pk, false),
        AccountMeta::new(*asks_pk, false),
    ];
    let instr = MangoInstruction::CancelPerpOrderByClientId { client_order_id, invalid_id_ok };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn cancel_perp_order(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,   // read
    mango_account_pk: &Pubkey, // write
    owner_pk: &Pubkey,         // read, signer
    perp_market_pk: &Pubkey,   // write
    bids_pk: &Pubkey,          // write
    asks_pk: &Pubkey,          // write
    order_id: i128,
    invalid_id_ok: bool,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new(*perp_market_pk, false),
        AccountMeta::new(*bids_pk, false),
        AccountMeta::new(*asks_pk, false),
    ];
    let instr = MangoInstruction::CancelPerpOrder { order_id, invalid_id_ok };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn cancel_all_perp_orders(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,   // read
    mango_account_pk: &Pubkey, // write
    owner_pk: &Pubkey,         // read, signer
    perp_market_pk: &Pubkey,   // write
    bids_pk: &Pubkey,          // write
    asks_pk: &Pubkey,          // write
    limit: u8,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new(*perp_market_pk, false),
        AccountMeta::new(*bids_pk, false),
        AccountMeta::new(*asks_pk, false),
    ];
    let instr = MangoInstruction::CancelAllPerpOrders { limit };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn cancel_perp_orders_side(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,   // read
    mango_account_pk: &Pubkey, // write
    owner_pk: &Pubkey,         // read, signer
    perp_market_pk: &Pubkey,   // write
    bids_pk: &Pubkey,          // write
    asks_pk: &Pubkey,          // write
    side: Side,
    limit: u8,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new(*perp_market_pk, false),
        AccountMeta::new(*bids_pk, false),
        AccountMeta::new(*asks_pk, false),
    ];
    let instr = MangoInstruction::CancelPerpOrdersSide { side, limit };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn force_cancel_perp_orders(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,         // read
    mango_cache_pk: &Pubkey,         // read
    perp_market_pk: &Pubkey,         // read
    bids_pk: &Pubkey,                // write
    asks_pk: &Pubkey,                // write
    liqee_mango_account_pk: &Pubkey, // write
    open_orders_pks: &[Pubkey],      // read
    limit: u8,
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new_readonly(*mango_cache_pk, false),
        AccountMeta::new_readonly(*perp_market_pk, false),
        AccountMeta::new(*bids_pk, false),
        AccountMeta::new(*asks_pk, false),
        AccountMeta::new(*liqee_mango_account_pk, false),
    ];
    accounts.extend(open_orders_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));
    let instr = MangoInstruction::ForceCancelPerpOrders { limit };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn init_advanced_orders(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,     // read
    mango_account_pk: &Pubkey,   // write
    owner_pk: &Pubkey,           // write & signer
    advanced_orders_pk: &Pubkey, // write
    system_prog_pk: &Pubkey,     // read
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new(*owner_pk, true),
        AccountMeta::new(*advanced_orders_pk, false),
        AccountMeta::new_readonly(*system_prog_pk, false),
    ];
    let instr = MangoInstruction::InitAdvancedOrders {};
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn close_advanced_orders(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    advanced_orders_pk: &Pubkey,
    owner_pk: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new(*owner_pk, true),
        AccountMeta::new(*advanced_orders_pk, false),
    ];

    let instr = MangoInstruction::CloseAdvancedOrders;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn add_perp_trigger_order(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,     // read
    mango_account_pk: &Pubkey,   // read
    owner_pk: &Pubkey,           // write & signer
    advanced_orders_pk: &Pubkey, // write
    mango_cache_pk: &Pubkey,     // read
    perp_market_pk: &Pubkey,     // read
    system_prog_pk: &Pubkey,     // read
    order_type: OrderType,
    side: Side,
    trigger_condition: TriggerCondition,
    reduce_only: bool,
    client_order_id: u64,
    price: i64,
    quantity: i64,
    trigger_price: I80F48,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new_readonly(*mango_account_pk, false),
        AccountMeta::new(*owner_pk, true),
        AccountMeta::new(*advanced_orders_pk, false),
        AccountMeta::new_readonly(*mango_cache_pk, false),
        AccountMeta::new_readonly(*perp_market_pk, false),
        AccountMeta::new_readonly(*system_prog_pk, false),
    ];
    let instr = MangoInstruction::AddPerpTriggerOrder {
        order_type,
        side,
        trigger_condition,
        reduce_only,
        client_order_id,
        price,
        quantity,
        trigger_price,
    };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn remove_advanced_order(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,     // read
    mango_account_pk: &Pubkey,   // read
    owner_pk: &Pubkey,           // write & signer
    advanced_orders_pk: &Pubkey, // write
    system_prog_pk: &Pubkey,     // read
    order_index: u8,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new_readonly(*mango_account_pk, false),
        AccountMeta::new(*owner_pk, true),
        AccountMeta::new(*advanced_orders_pk, false),
        AccountMeta::new_readonly(*system_prog_pk, false),
    ];
    let instr = MangoInstruction::RemoveAdvancedOrder { order_index };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn execute_perp_trigger_order(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,     // read
    mango_account_pk: &Pubkey,   // write
    advanced_orders_pk: &Pubkey, // write
    agent_pk: &Pubkey,           // write & signer
    mango_cache_pk: &Pubkey,     // read
    perp_market_pk: &Pubkey,     // write
    bids_pk: &Pubkey,            // write
    asks_pk: &Pubkey,            // write
    event_queue_pk: &Pubkey,     // write
    order_index: u8,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new(*advanced_orders_pk, false),
        AccountMeta::new(*agent_pk, true),
        AccountMeta::new_readonly(*mango_cache_pk, false),
        AccountMeta::new(*perp_market_pk, false),
        AccountMeta::new(*bids_pk, false),
        AccountMeta::new(*asks_pk, false),
        AccountMeta::new(*event_queue_pk, false),
    ];
    let instr = MangoInstruction::ExecutePerpTriggerOrder { order_index };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn consume_events(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,      // read
    mango_cache_pk: &Pubkey,      // read
    perp_market_pk: &Pubkey,      // read
    event_queue_pk: &Pubkey,      // write
    mango_acc_pks: &mut [Pubkey], // write
    limit: usize,
) -> Result<Instruction, ProgramError> {
    let fixed_accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new_readonly(*mango_cache_pk, false),
        AccountMeta::new(*perp_market_pk, false),
        AccountMeta::new(*event_queue_pk, false),
    ];
    mango_acc_pks.sort();
    let mango_accounts = mango_acc_pks.into_iter().map(|pk| AccountMeta::new(*pk, false));
    let accounts = fixed_accounts.into_iter().chain(mango_accounts).collect();
    let instr = MangoInstruction::ConsumeEvents { limit };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn settle_pnl(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,     // read
    mango_account_a_pk: &Pubkey, // write
    mango_account_b_pk: &Pubkey, // write
    mango_cache_pk: &Pubkey,     // read
    root_bank_pk: &Pubkey,       // read
    node_bank_pk: &Pubkey,       // write
    market_index: usize,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_a_pk, false),
        AccountMeta::new(*mango_account_b_pk, false),
        AccountMeta::new_readonly(*mango_cache_pk, false),
        AccountMeta::new_readonly(*root_bank_pk, false),
        AccountMeta::new(*node_bank_pk, false),
    ];
    let instr = MangoInstruction::SettlePnl { market_index };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn update_funding(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey, // read
    mango_cache_pk: &Pubkey, // write
    perp_market_pk: &Pubkey, // write
    bids_pk: &Pubkey,        // read
    asks_pk: &Pubkey,        // read
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_cache_pk, false),
        AccountMeta::new(*perp_market_pk, false),
        AccountMeta::new_readonly(*bids_pk, false),
        AccountMeta::new_readonly(*asks_pk, false),
    ];
    let instr = MangoInstruction::UpdateFunding {};
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn withdraw(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    owner_pk: &Pubkey,
    mango_cache_pk: &Pubkey,
    root_bank_pk: &Pubkey,
    node_bank_pk: &Pubkey,
    vault_pk: &Pubkey,
    token_account_pk: &Pubkey,
    signer_pk: &Pubkey,
    open_orders_pks: &[Pubkey],

    quantity: u64,
    allow_borrow: bool,
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new_readonly(*mango_cache_pk, false),
        AccountMeta::new_readonly(*root_bank_pk, false),
        AccountMeta::new(*node_bank_pk, false),
        AccountMeta::new(*vault_pk, false),
        AccountMeta::new(*token_account_pk, false),
        AccountMeta::new_readonly(*signer_pk, false),
        AccountMeta::new_readonly(spl_token::ID, false),
    ];

    accounts.extend(open_orders_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));

    let instr = MangoInstruction::Withdraw { quantity, allow_borrow };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn borrow(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    mango_cache_pk: &Pubkey,
    owner_pk: &Pubkey,
    root_bank_pk: &Pubkey,
    node_bank_pk: &Pubkey,
    open_orders_pks: &[Pubkey],

    quantity: u64,
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new_readonly(*mango_cache_pk, false),
        AccountMeta::new_readonly(*root_bank_pk, false),
        AccountMeta::new(*node_bank_pk, false),
    ];

    accounts.extend(open_orders_pks.iter().map(|pk| AccountMeta::new(*pk, false)));

    let instr = MangoInstruction::Borrow { quantity };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn cache_prices(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_cache_pk: &Pubkey,
    oracle_pks: &[Pubkey],
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_cache_pk, false),
    ];
    accounts.extend(oracle_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));
    let instr = MangoInstruction::CachePrices;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn cache_root_banks(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_cache_pk: &Pubkey,
    root_bank_pks: &[Pubkey],
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_cache_pk, false),
    ];
    accounts.extend(root_bank_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));
    let instr = MangoInstruction::CacheRootBanks;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn cache_perp_markets(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_cache_pk: &Pubkey,
    perp_market_pks: &[Pubkey],
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_cache_pk, false),
    ];
    accounts.extend(perp_market_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));
    let instr = MangoInstruction::CachePerpMarkets;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn init_spot_open_orders(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    owner_pk: &Pubkey,
    dex_prog_pk: &Pubkey,
    open_orders_pk: &Pubkey,
    spot_market_pk: &Pubkey,
    signer_pk: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new_readonly(*dex_prog_pk, false),
        AccountMeta::new(*open_orders_pk, false),
        AccountMeta::new_readonly(*spot_market_pk, false),
        AccountMeta::new_readonly(*signer_pk, false),
        AccountMeta::new_readonly(solana_program::sysvar::rent::ID, false),
    ];

    let instr = MangoInstruction::InitSpotOpenOrders;
    let data = instr.pack();

    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn create_spot_open_orders(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    owner_pk: &Pubkey,
    dex_prog_pk: &Pubkey,
    open_orders_pk: &Pubkey,
    spot_market_pk: &Pubkey,
    signer_pk: &Pubkey,
    payer_pk: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new_readonly(*dex_prog_pk, false),
        AccountMeta::new(*open_orders_pk, false),
        AccountMeta::new_readonly(*spot_market_pk, false),
        AccountMeta::new_readonly(*signer_pk, false),
        AccountMeta::new_readonly(solana_program::system_program::ID, false),
        AccountMeta::new(*payer_pk, true),
    ];

    let instr = MangoInstruction::CreateSpotOpenOrders;
    let data = instr.pack();

    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn close_spot_open_orders(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    owner_pk: &Pubkey,
    dex_prog_pk: &Pubkey,
    open_orders_pk: &Pubkey,
    spot_market_pk: &Pubkey,
    signer_pk: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new(*owner_pk, true),
        AccountMeta::new_readonly(*dex_prog_pk, false),
        AccountMeta::new(*open_orders_pk, false),
        AccountMeta::new_readonly(*spot_market_pk, false),
        AccountMeta::new_readonly(*signer_pk, false),
    ];

    let instr = MangoInstruction::CloseSpotOpenOrders;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn place_spot_order(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    owner_pk: &Pubkey,
    mango_cache_pk: &Pubkey,
    dex_prog_pk: &Pubkey,
    spot_market_pk: &Pubkey,
    bids_pk: &Pubkey,
    asks_pk: &Pubkey,
    dex_request_queue_pk: &Pubkey,
    dex_event_queue_pk: &Pubkey,
    dex_base_pk: &Pubkey,
    dex_quote_pk: &Pubkey,
    base_root_bank_pk: &Pubkey,
    base_node_bank_pk: &Pubkey,
    base_vault_pk: &Pubkey,
    quote_root_bank_pk: &Pubkey,
    quote_node_bank_pk: &Pubkey,
    quote_vault_pk: &Pubkey,
    signer_pk: &Pubkey,
    dex_signer_pk: &Pubkey,
    msrm_or_srm_vault_pk: &Pubkey,
    open_orders_pks: &[Pubkey],

    market_index: usize, // used to determine which of the open orders accounts should be passed in write
    order: serum_dex::instruction::NewOrderInstructionV3,
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new_readonly(*mango_cache_pk, false),
        AccountMeta::new_readonly(*dex_prog_pk, false),
        AccountMeta::new(*spot_market_pk, false),
        AccountMeta::new(*bids_pk, false),
        AccountMeta::new(*asks_pk, false),
        AccountMeta::new(*dex_request_queue_pk, false),
        AccountMeta::new(*dex_event_queue_pk, false),
        AccountMeta::new(*dex_base_pk, false),
        AccountMeta::new(*dex_quote_pk, false),
        AccountMeta::new_readonly(*base_root_bank_pk, false),
        AccountMeta::new(*base_node_bank_pk, false),
        AccountMeta::new(*base_vault_pk, false),
        AccountMeta::new_readonly(*quote_root_bank_pk, false),
        AccountMeta::new(*quote_node_bank_pk, false),
        AccountMeta::new(*quote_vault_pk, false),
        AccountMeta::new_readonly(spl_token::ID, false),
        AccountMeta::new_readonly(*signer_pk, false),
        AccountMeta::new_readonly(solana_program::sysvar::rent::ID, false),
        AccountMeta::new_readonly(*dex_signer_pk, false),
        AccountMeta::new_readonly(*msrm_or_srm_vault_pk, false),
    ];

    accounts.extend(open_orders_pks.iter().enumerate().map(|(i, pk)| {
        if i == market_index {
            AccountMeta::new(*pk, false)
        } else {
            AccountMeta::new_readonly(*pk, false)
        }
    }));

    let instr = MangoInstruction::PlaceSpotOrder { order };
    let data = instr.pack();

    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn settle_funds(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_cache_pk: &Pubkey,
    owner_pk: &Pubkey,
    mango_account_pk: &Pubkey,
    dex_prog_pk: &Pubkey,
    spot_market_pk: &Pubkey,
    open_orders_pk: &Pubkey,
    signer_pk: &Pubkey,
    dex_base_pk: &Pubkey,
    dex_quote_pk: &Pubkey,
    base_root_bank_pk: &Pubkey,
    base_node_bank_pk: &Pubkey,
    quote_root_bank_pk: &Pubkey,
    quote_node_bank_pk: &Pubkey,
    base_vault_pk: &Pubkey,
    quote_vault_pk: &Pubkey,
    dex_signer_pk: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new_readonly(*mango_cache_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*dex_prog_pk, false),
        AccountMeta::new(*spot_market_pk, false),
        AccountMeta::new(*open_orders_pk, false),
        AccountMeta::new_readonly(*signer_pk, false),
        AccountMeta::new(*dex_base_pk, false),
        AccountMeta::new(*dex_quote_pk, false),
        AccountMeta::new_readonly(*base_root_bank_pk, false),
        AccountMeta::new(*base_node_bank_pk, false),
        AccountMeta::new_readonly(*quote_root_bank_pk, false),
        AccountMeta::new(*quote_node_bank_pk, false),
        AccountMeta::new(*base_vault_pk, false),
        AccountMeta::new(*quote_vault_pk, false),
        AccountMeta::new_readonly(*dex_signer_pk, false),
        AccountMeta::new_readonly(spl_token::ID, false),
    ];

    let instr = MangoInstruction::SettleFunds;
    let data = instr.pack();

    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn add_oracle(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    oracle_pk: &Pubkey,
    admin_pk: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*mango_group_pk, false),
        AccountMeta::new(*oracle_pk, false),
        AccountMeta::new_readonly(*admin_pk, true),
    ];

    let instr = MangoInstruction::AddOracle;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn update_root_bank(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_cache_pk: &Pubkey,
    root_bank_pk: &Pubkey,
    node_bank_pks: &[Pubkey],
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_cache_pk, false),
        AccountMeta::new(*root_bank_pk, false),
    ];

    accounts.extend(node_bank_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));

    let instr = MangoInstruction::UpdateRootBank;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn set_oracle(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    oracle_pk: &Pubkey,
    admin_pk: &Pubkey,
    price: I80F48,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*oracle_pk, false),
        AccountMeta::new_readonly(*admin_pk, true),
    ];

    let instr = MangoInstruction::SetOracle { price };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn liquidate_token_and_token(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_cache_pk: &Pubkey,
    liqee_mango_account_pk: &Pubkey,
    liqor_mango_account_pk: &Pubkey,
    liqor_pk: &Pubkey,
    asset_root_bank_pk: &Pubkey,
    asset_node_bank_pk: &Pubkey,
    liab_root_bank_pk: &Pubkey,
    liab_node_bank_pk: &Pubkey,
    liqee_open_orders_pks: &[Pubkey],
    liqor_open_orders_pks: &[Pubkey],
    max_liab_transfer: I80F48,
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new_readonly(*mango_cache_pk, false),
        AccountMeta::new(*liqee_mango_account_pk, false),
        AccountMeta::new(*liqor_mango_account_pk, false),
        AccountMeta::new_readonly(*liqor_pk, true),
        AccountMeta::new_readonly(*asset_root_bank_pk, false),
        AccountMeta::new(*asset_node_bank_pk, false),
        AccountMeta::new_readonly(*liab_root_bank_pk, false),
        AccountMeta::new(*liab_node_bank_pk, false),
    ];

    accounts.extend(liqee_open_orders_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));
    accounts.extend(liqor_open_orders_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));

    let instr = MangoInstruction::LiquidateTokenAndToken { max_liab_transfer };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn liquidate_token_and_perp(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_cache_pk: &Pubkey,
    liqee_mango_account_pk: &Pubkey,
    liqor_mango_account_pk: &Pubkey,
    liqor_pk: &Pubkey,
    root_bank_pk: &Pubkey,
    node_bank_pk: &Pubkey,
    liqee_open_orders_pks: &[Pubkey],
    liqor_open_orders_pks: &[Pubkey],
    asset_type: AssetType,
    asset_index: usize,
    liab_type: AssetType,
    liab_index: usize,
    max_liab_transfer: I80F48,
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new_readonly(*mango_cache_pk, false),
        AccountMeta::new(*liqee_mango_account_pk, false),
        AccountMeta::new(*liqor_mango_account_pk, false),
        AccountMeta::new_readonly(*liqor_pk, true),
        AccountMeta::new_readonly(*root_bank_pk, false),
        AccountMeta::new(*node_bank_pk, false),
    ];

    accounts.extend(liqee_open_orders_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));
    accounts.extend(liqor_open_orders_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));

    let instr = MangoInstruction::LiquidateTokenAndPerp {
        asset_type,
        asset_index,
        liab_type,
        liab_index,
        max_liab_transfer,
    };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn liquidate_perp_market(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    mango_cache_pk: &Pubkey,
    perp_market_pk: &Pubkey,
    event_queue_pk: &Pubkey,
    liqee_mango_account_pk: &Pubkey,
    liqor_mango_account_pk: &Pubkey,
    liqor_pk: &Pubkey,
    liqee_open_orders_pks: &[Pubkey],
    liqor_open_orders_pks: &[Pubkey],
    base_transfer_request: i64,
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new_readonly(*mango_cache_pk, false),
        AccountMeta::new(*perp_market_pk, false),
        AccountMeta::new(*event_queue_pk, false),
        AccountMeta::new(*liqee_mango_account_pk, false),
        AccountMeta::new(*liqor_mango_account_pk, false),
        AccountMeta::new_readonly(*liqor_pk, true),
    ];

    accounts.extend(liqee_open_orders_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));
    accounts.extend(liqor_open_orders_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));

    let instr = MangoInstruction::LiquidatePerpMarket { base_transfer_request };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn change_spot_market_params(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    spot_market_pk: &Pubkey,
    root_bank_pk: &Pubkey,
    admin_pk: &Pubkey,
    maint_leverage: Option<I80F48>,
    init_leverage: Option<I80F48>,
    liquidation_fee: Option<I80F48>,
    optimal_util: Option<I80F48>,
    optimal_rate: Option<I80F48>,
    max_rate: Option<I80F48>,
    version: Option<u8>,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*mango_group_pk, false),
        AccountMeta::new(*spot_market_pk, false),
        AccountMeta::new(*root_bank_pk, false),
        AccountMeta::new_readonly(*admin_pk, true),
    ];

    let instr = MangoInstruction::ChangeSpotMarketParams {
        maint_leverage,
        init_leverage,
        liquidation_fee,
        optimal_util,
        optimal_rate,
        max_rate,
        version,
    };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

/// Serialize Option<T> as (bool, T). This gives the binary representation
/// a fixed width, instead of it becoming one byte for None.
fn serialize_option_fixed_width<S: serde::Serializer, T: Sized + Default + Serialize>(
    opt: &Option<T>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeTuple;
    let mut tup = serializer.serialize_tuple(2)?;
    match opt {
        Some(value) => {
            tup.serialize_element(&true)?;
            tup.serialize_element(&value)?;
        }
        None => {
            tup.serialize_element(&false)?;
            tup.serialize_element(&T::default())?;
        }
    };
    tup.end()
}
