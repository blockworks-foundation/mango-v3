use anchor_lang::prelude::*;
declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

// This is a dummy program to take advantage of Anchor events

#[program]
pub mod mango_logs {}

#[event]
pub struct FillLog {
    pub taker_side: u8, // side from the taker's POV
    pub maker_slot: u8,
    pub maker_out: bool, // true if maker order quantity == 0
    pub timestamp: u64,
    pub seq_num: u64, // note: usize same as u64

    pub maker: Pubkey,
    pub maker_order_id: i128,
    pub maker_client_order_id: u64,
    pub maker_fee: i128,

    // The best bid/ask at the time the maker order was placed. Used for liquidity incentives
    pub best_initial: i64,

    // Timestamp of when the maker order was placed; copied over from the LeafNode
    pub maker_timestamp: u64,

    pub taker: Pubkey,
    pub taker_order_id: i128,
    pub taker_client_order_id: u64,
    pub taker_fee: i128,

    pub price: i64,
    pub quantity: i64, // number of quote lots
}

#[event]
pub struct TokenBalanceLog {
    pub mango_group: Pubkey,
    pub mango_account: Pubkey,
    pub token_index: u64, // IDL doesn't support usize
    pub deposit: i128, // on client convert i128 to I80F48 easily by passing in the BN to I80F48 ctor
    pub borrow: i128,
}

#[event]
pub struct CachePricesLog {
    // TODO
}

#[event]
pub struct CacheRootBanksLog {
    // TODO
}
#[event]
pub struct CachePerpMarketsLog {
    // TODO
}
#[event]
pub struct SettlePnlLog {
    // TODO
}
#[event]
pub struct SettleFeesLog {
    // TODO
}
#[event]
pub struct LiquidateTokenAndTokenLog {
    // TODO
}
#[event]
pub struct LiquidateTokenAndPerpLog {
    // TODO
}
#[event]
pub struct LiquidatePerpMarketLog {
    // TODO
}
#[event]
pub struct PerpBankruptcyLog {
    // TODO
}
#[event]
pub struct PerpSocializedLossLog {
    // TODO
}
#[event]
pub struct TokenBankruptcyLog {
    // TODO
}
#[event]
pub struct UpdateRootBankLog {
    // TODO
}
