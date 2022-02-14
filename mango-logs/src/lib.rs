use anchor_lang::prelude::*;
use anchor_lang::Discriminator;
use std::io::Write;
declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

/// Log to Program Log with a prologue so transaction scraper knows following line is valid mango log
///
/// Warning: This will allocate large buffers on the heap which will never be released as Solana
/// uses a simple bump allocator where free() is a noop. Since the max heap size is limited
// (32k currently), calling this multiple times can lead to memory allocation failures.
#[macro_export]
macro_rules! mango_emit_heap {
    ($e:expr) => {
        msg!("mango-log");
        emit!($e);
    };
}

/// Log to Program Log with a prologue so transaction scraper knows following line is valid mango log
///
/// Warning: This stores intermediate results on the stack, which must have 2*N+ free bytes.
/// This function will panic if the generated event does not fit the buffer of size N.
pub fn mango_emit_stack<T: AnchorSerialize + Discriminator, const N: usize>(event: T) {
    let mut data_buf = [0u8; N];
    let mut out_buf = [0u8; N];

    mango_emit_buffers(event, &mut data_buf[..], &mut out_buf[..])
}

/// Log to Program Log with a prologue so transaction scraper knows following line is valid mango log
///
/// This function will write intermediate data to data_buf and out_buf. The buffers must be
/// large enough to hold this data, or the function will panic.
pub fn mango_emit_buffers<T: AnchorSerialize + Discriminator>(
    event: T,
    data_buf: &mut [u8],
    out_buf: &mut [u8],
) {
    let mut data_writer = std::io::Cursor::new(data_buf);
    data_writer.write_all(&<T as Discriminator>::discriminator()).unwrap();
    borsh::to_writer(&mut data_writer, &event).unwrap();
    let data_len = data_writer.position() as usize;

    let out_len = base64::encode_config_slice(
        &data_writer.into_inner()[0..data_len],
        base64::STANDARD,
        out_buf,
    );

    let msg_bytes = &out_buf[0..out_len];
    let msg_str = unsafe { std::str::from_utf8_unchecked(&msg_bytes) };

    msg!("mango-log");
    msg!(msg_str);
}

// This is a dummy program to take advantage of Anchor events
#[program]
pub mod mango_logs {}

#[event]
pub struct FillLog {
    pub mango_group: Pubkey,
    pub market_index: u64,
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
    pub quantity: i64, // number of base lots
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
    pub mango_group: Pubkey,
    pub oracle_indexes: Vec<u64>,
    pub oracle_prices: Vec<i128>, // I80F48 format
}
#[event]
pub struct CacheRootBanksLog {
    pub mango_group: Pubkey,
    pub token_indexes: Vec<u64>,    // usize
    pub deposit_indexes: Vec<i128>, // I80F48
    pub borrow_indexes: Vec<i128>,  // I80F48
}

#[event]
pub struct CachePerpMarketsLog {
    pub mango_group: Pubkey,
    pub market_indexes: Vec<u64>,
    pub long_fundings: Vec<i128>,  // I80F48
    pub short_fundings: Vec<i128>, // I80F48
}

#[event]
pub struct SettlePnlLog {
    pub mango_group: Pubkey,
    pub mango_account_a: Pubkey,
    pub mango_account_b: Pubkey,
    pub market_index: u64,
    pub settlement: i128, // I80F48
}

#[event]
pub struct SettleFeesLog {
    pub mango_group: Pubkey,
    pub mango_account: Pubkey,
    pub market_index: u64,
    pub settlement: i128, // I80F48
}

#[event]
pub struct LiquidateTokenAndTokenLog {
    pub mango_group: Pubkey,
    pub liqee: Pubkey,
    pub liqor: Pubkey,
    pub asset_index: u64,
    pub liab_index: u64,
    pub asset_transfer: i128, // I80F48
    pub liab_transfer: i128,  // I80F48
    pub asset_price: i128,    // I80F48
    pub liab_price: i128,     // I80F48
    pub bankruptcy: bool,
}

#[event]
pub struct LiquidateTokenAndPerpLog {
    pub mango_group: Pubkey,
    pub liqee: Pubkey,
    pub liqor: Pubkey,
    pub asset_index: u64,
    pub liab_index: u64,
    pub asset_type: u8,
    pub liab_type: u8,
    pub asset_price: i128,    // I80F48
    pub liab_price: i128,     // I80F48
    pub asset_transfer: i128, // I80F48
    pub liab_transfer: i128,  // I80F48
    pub bankruptcy: bool,
}

#[event]
pub struct LiquidatePerpMarketLog {
    pub mango_group: Pubkey,
    pub liqee: Pubkey,
    pub liqor: Pubkey,
    pub market_index: u64,
    pub price: i128, // I80F48
    pub base_transfer: i64,
    pub quote_transfer: i128, // I80F48
    pub bankruptcy: bool,
}

#[event]
pub struct PerpBankruptcyLog {
    pub mango_group: Pubkey,
    pub liqee: Pubkey,
    pub liqor: Pubkey,
    pub liab_index: u64,
    pub insurance_transfer: u64,
    pub socialized_loss: i128,     // I80F48
    pub cache_long_funding: i128,  // I80F48
    pub cache_short_funding: i128, // I80F48
}

#[event]
pub struct TokenBankruptcyLog {
    pub mango_group: Pubkey,
    pub liqee: Pubkey,
    pub liqor: Pubkey,
    pub liab_index: u64,
    pub insurance_transfer: u64,
    /// This is in native units for the liab token NOT static units
    pub socialized_loss: i128, // I80F48
    pub percentage_loss: i128,     // I80F48
    pub cache_deposit_index: i128, // I80F48
}

#[event]
pub struct UpdateRootBankLog {
    pub mango_group: Pubkey,
    pub token_index: u64,
    pub deposit_index: i128, // I80F48
    pub borrow_index: i128,  // I80F48
}

#[event]
pub struct UpdateFundingLog {
    pub mango_group: Pubkey,
    pub market_index: u64,
    pub long_funding: i128,  // I80F48
    pub short_funding: i128, // I80F48
}

#[event]
pub struct OpenOrdersBalanceLog {
    pub mango_group: Pubkey,
    pub mango_account: Pubkey,
    pub market_index: u64,
    pub base_total: u64,
    pub base_free: u64,
    /// this field does not include the referrer_rebates; need to add that in to get true total
    pub quote_total: u64,
    pub quote_free: u64,
    pub referrer_rebates_accrued: u64,
}

#[event]
pub struct MngoAccrualLog {
    pub mango_group: Pubkey,
    pub mango_account: Pubkey,
    pub market_index: u64,
    /// incremental mngo accrual from canceling/filling this order or set of orders
    pub mngo_accrual: u64,
}

#[event]
pub struct WithdrawLog {
    pub mango_group: Pubkey,
    pub mango_account: Pubkey,
    pub owner: Pubkey,
    pub token_index: u64,
    pub quantity: u64,
}

#[event]
pub struct DepositLog {
    pub mango_group: Pubkey,
    pub mango_account: Pubkey,
    pub owner: Pubkey,
    pub token_index: u64,
    pub quantity: u64,
}

#[event]
pub struct RedeemMngoLog {
    pub mango_group: Pubkey,
    pub mango_account: Pubkey,
    pub market_index: u64,
    pub redeemed_mngo: u64,
}

#[event]
pub struct CancelAllPerpOrdersLog {
    pub mango_group: Pubkey,
    pub mango_account: Pubkey,
    pub market_index: u64,
    pub all_order_ids: Vec<i128>,
    pub canceled_order_ids: Vec<i128>,
}

#[event]
pub struct PerpBalanceLog {
    pub mango_group: Pubkey,
    pub mango_account: Pubkey,
    pub market_index: u64, // IDL doesn't support usize
    pub base_position: i64,
    pub quote_position: i128,        // I80F48
    pub long_settled_funding: i128,  // I80F48
    pub short_settled_funding: i128, // I80F48

    pub long_funding: i128,  // I80F48
    pub short_funding: i128, // I80F48
}

#[event]
pub struct ReferralFeeAccrualLog {
    pub mango_group: Pubkey,
    pub referrer_mango_account: Pubkey,
    pub referree_mango_account: Pubkey,
    pub market_index: u64,
    pub referral_fee_accrual: i128, // I80F48
}
