mod program_test;
use fixed::types::I80F48;
use mango::{instruction::*, matching::*, state::*};
use program_test::cookies::*;
use program_test::*;

#[test]
fn test_instruction_serialization() {
    use std::num::NonZeroU64;
    let cases = vec![
        MangoInstruction::InitMangoGroup {
            signer_nonce: 126,
            valid_interval: 79846,
            quote_optimal_util: I80F48::from_num(1.0),
            quote_optimal_rate: I80F48::from_num(7897891.12310),
            quote_max_rate: I80F48::from_num(1546.0),
        },
        MangoInstruction::InitMangoAccount {},
        MangoInstruction::Deposit { quantity: 1234567 },
        MangoInstruction::Withdraw { quantity: 1234567, allow_borrow: true },
        MangoInstruction::AddSpotMarket {
            maint_leverage: I80F48::from_num(1546.0),
            init_leverage: I80F48::from_num(1546789.0),
            liquidation_fee: I80F48::from_num(1546.789470),
            optimal_util: I80F48::from_num(6156.0),
            optimal_rate: I80F48::from_num(8791.150),
            max_rate: I80F48::from_num(46.9870),
        },
        MangoInstruction::AddToBasket { market_index: 156489 },
        MangoInstruction::Borrow { quantity: 1264 },
        MangoInstruction::AddPerpMarket {
            maint_leverage: I80F48::from_num(1546.0),
            init_leverage: I80F48::from_num(1546789.0),
            liquidation_fee: I80F48::from_num(1546.789470),
            maker_fee: I80F48::from_num(6156.0),
            taker_fee: I80F48::from_num(8791.150),
            base_lot_size: -4597,
            quote_lot_size: 45644597,
            rate: I80F48::from_num(8791.150),
            max_depth_bps: I80F48::from_num(87.99),
            target_period_length: 1234,
            mngo_per_period: 987,
            exp: 5,
        },
        MangoInstruction::PlacePerpOrder {
            price: 898726,
            quantity: 54689789456,
            client_order_id: 42,
            side: Side::Ask,
            order_type: OrderType::PostOnly,
            reduce_only: true,
        },
        MangoInstruction::CancelPerpOrderByClientId { client_order_id: 78, invalid_id_ok: true },
        MangoInstruction::CancelPerpOrder { order_id: 497894561564897, invalid_id_ok: true },
        MangoInstruction::ConsumeEvents { limit: 77 },
        MangoInstruction::SetOracle { price: I80F48::from_num(6156.0) },
        MangoInstruction::PlaceSpotOrder {
            order: serum_dex::instruction::NewOrderInstructionV3 {
                side: serum_dex::matching::Side::Bid,
                limit_price: NonZeroU64::new(456789).unwrap(),
                max_coin_qty: NonZeroU64::new(789456).unwrap(),
                max_native_pc_qty_including_fees: NonZeroU64::new(42).unwrap(),
                order_type: serum_dex::matching::OrderType::PostOnly,
                self_trade_behavior: serum_dex::instruction::SelfTradeBehavior::CancelProvide,
                client_order_id: 8941,
                limit: 1597,
                max_ts: i64::MAX,
            },
        },
        MangoInstruction::PlaceSpotOrder2 {
            order: serum_dex::instruction::NewOrderInstructionV3 {
                side: serum_dex::matching::Side::Ask,
                limit_price: NonZeroU64::new(456789).unwrap(),
                max_coin_qty: NonZeroU64::new(789456).unwrap(),
                max_native_pc_qty_including_fees: NonZeroU64::new(42).unwrap(),
                order_type: serum_dex::matching::OrderType::ImmediateOrCancel,
                self_trade_behavior: serum_dex::instruction::SelfTradeBehavior::CancelProvide,
                client_order_id: 8941,
                limit: 1597,
                max_ts: i64::MAX,
            },
        },
        MangoInstruction::CancelSpotOrder {
            order: serum_dex::instruction::CancelOrderInstructionV2 {
                side: serum_dex::matching::Side::Ask,
                order_id: 587945166,
            },
        },
        MangoInstruction::SettlePnl { market_index: 7897 },
        MangoInstruction::SettleBorrow { token_index: 25, quantity: 8979846 },
        MangoInstruction::ForceCancelSpotOrders { limit: 254 },
        MangoInstruction::ForceCancelPerpOrders { limit: 254 },
        MangoInstruction::LiquidateTokenAndToken { max_liab_transfer: I80F48::from_num(6156.33) },
        MangoInstruction::LiquidateTokenAndPerp {
            asset_type: AssetType::Perp,
            asset_index: 1234,
            liab_type: AssetType::Token,
            liab_index: 598789,
            max_liab_transfer: I80F48::from_num(6156.33),
        },
        MangoInstruction::LiquidatePerpMarket { base_transfer_request: -8974 },
        MangoInstruction::ResolvePerpBankruptcy {
            liab_index: 254,
            max_liab_transfer: I80F48::from_num(6156.33),
        },
        MangoInstruction::ResolveTokenBankruptcy { max_liab_transfer: I80F48::from_num(6156.33) },
        MangoInstruction::AddMangoAccountInfo { info: [7u8; INFO_LEN] },
        MangoInstruction::DepositMsrm { quantity: 15 },
        MangoInstruction::WithdrawMsrm { quantity: 98784615 },
        MangoInstruction::ChangePerpMarketParams {
            maint_leverage: Some(I80F48::from_num(6156.33)),
            init_leverage: None,
            liquidation_fee: Some(I80F48::from_num(6156.33)),
            maker_fee: None,
            taker_fee: Some(I80F48::from_num(999.73)),
            rate: None,
            max_depth_bps: None,
            target_period_length: Some(87985461),
            mngo_per_period: None,
            exp: Some(7),
        },
        MangoInstruction::CancelAllPerpOrders { limit: 7 },
        MangoInstruction::AddPerpTriggerOrder {
            order_type: OrderType::Limit,
            side: Side::Ask,
            trigger_condition: TriggerCondition::Above,
            reduce_only: true,
            client_order_id: 42,
            price: 898726,
            quantity: 54689789456,
            trigger_price: I80F48::from_num(45643.45645646),
        },
        MangoInstruction::AddPerpTriggerOrder {
            order_type: OrderType::PostOnly,
            side: Side::Bid,
            trigger_condition: TriggerCondition::Below,
            reduce_only: false,
            client_order_id: 4242,
            price: 898,
            quantity: 897894561,
            trigger_price: I80F48::from_num(1.0),
        },
        MangoInstruction::RemoveAdvancedOrder { order_index: 42 },
        MangoInstruction::ExecutePerpTriggerOrder { order_index: 249 },
    ];
    for case in cases {
        assert!(MangoInstruction::unpack(&case.pack()).unwrap() == case);
    }
}
