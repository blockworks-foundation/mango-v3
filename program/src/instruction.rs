use crate::matching::{OrderType, Side};
use crate::state::MAX_PAIRS;
use arrayref::{array_ref, array_refs};
use fixed::types::I80F48;
use num_enum::TryFromPrimitive;
use serde::{Deserialize, Serialize};
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;
use std::convert::TryInto;
use std::num::NonZeroU64;
use solana_program::msg;

#[repr(C)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MangoInstruction {
    /// Initialize a group of lending pools that can be cross margined
    ///
    /// Accounts expected by this instruction (9):
    ///
    /// 0. `[writable]` mango_group_ai - TODO
    /// 1. `[]` rent_ai - TODO
    /// 2. `[]` signer_ai - TODO
    /// 3. `[]` admin_ai - TODO
    /// 4. `[]` quote_mint_ai - TODO
    /// 5. `[]` quote_vault_ai - TODO
    /// 6. `[writable]` quote_node_bank_ai - TODO
    /// 7. `[writable]` quote_root_bank_ai - TODO
    /// 6. `[writable]` mango_cache_ai - Account to cache prices, root banks, and perp markets
    /// 8. `[]` dex_prog_ai - TODO
    InitMangoGroup {
        signer_nonce: u64,
        valid_interval: u64,
    },

    /// Initialize a mango account for a user
    ///
    /// Accounts expected by this instruction (4):
    ///
    /// 0. `[]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[writable]` mango_account_ai - the mango account data
    /// 2. `[signer]` owner_ai - Solana account of owner of the mango account
    /// 3. `[]` rent_ai - Rent sysvar account
    InitMangoAccount,

    /// Deposit funds into mango account
    ///
    /// Accounts expected by this instruction (8):
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
    /// 10. `[read]` clock_ai,         -
    /// 11..+ `[]` open_orders_accs - open orders for each of the spot market
    Withdraw {
        quantity: u64,
        allow_borrow: bool,
    },

    /// Add a token to a mango group
    ///
    /// Accounts expected by this instruction (7):
    ///
    /// 0. `[writable]` mango_group_ai - TODO
    /// 1. `[]` spot_market_ai - TODO
    /// 2. `[]` dex_program_ai - TODO
    /// 1. `[]` mint_ai - TODO
    /// 2. `[writable]` node_bank_ai - TODO
    /// 3. `[]` vault_ai - TODO
    /// 4. `[writable]` root_bank_ai - TODO
    /// 5. `[signer]` admin_ai - TODO
    AddSpotMarket {
        market_index: usize,
        maint_leverage: I80F48,
        init_leverage: I80F48,
    },

    /// Add a spot market to a mango account basket
    ///
    /// Accounts expected by this instruction (6)
    ///
    /// 0. `[]` mango_group_ai - TODO
    /// 1. `[writable]` mango_account_ai - TODO
    /// 2. `[signer]` owner_ai - Solana account of owner of the mango account
    /// 3. `[]` spot_market_ai - TODO
    AddToBasket {
        market_index: usize,
    },

    /// Borrow by incrementing MangoAccount.borrows given collateral ratio is below init_coll_rat
    ///
    /// Accounts expected by this instruction (4 + 2 * NUM_MARKETS):
    ///
    /// 0. `[]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[writable]` mango_account_ai - the mango account for this user
    /// 2. `[signer]` owner_ai - Solana account of owner of the MangoAccount
    /// 3. `[]` mango_cache_ai - TODO
    /// 4. `[]` root_bank_ai - Root bank owned by MangoGroup
    /// 5. `[writable]` node_bank_ai - Node bank owned by RootBank
    /// 6. `[]` clock_ai - Clock sysvar account
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

    /// Cache root banks
    ///
    /// Accounts expected: 2 + Root Banks
    /// 0. `[]` mango_group_ai
    /// 1. `[writable]` mango_cache_ai
    CacheRootBanks,

    /// Place an order on the Serum Dex using Mango account
    ///
    /// Accounts expected by this instruction (19 + MAX_PAIRS):
    ///
    PlaceSpotOrder {
        order: serum_dex::instruction::NewOrderInstructionV3,
    },

    /// Add oracle
    ///
    /// Accounts expected: 3
    /// 0. `[writable]` mango_group_ai - MangoGroup
    /// 1. `[]` oracle_ai - oracle
    /// 2. `[signer]` admin_ai - admin
    AddOracle, // = 10

    /// Add a perp market to a mango group
    ///
    /// Accounts expected by this instruction (6):
    ///
    /// 0. `[writable]` mango_group_ai - TODO
    /// 1. `[writable]` perp_market_ai - TODO
    /// 2. `[writable]` event_queue_ai - TODO
    /// 3. `[writable]` bids_ai - TODO
    /// 4. `[writable]` asks_ai - TODO
    /// 5. `[signer]` admin_ai - TODO
    AddPerpMarket {
        market_index: usize,
        maint_leverage: I80F48,
        init_leverage: I80F48,
        base_lot_size: i64,
        quote_lot_size: i64,
    },

    /// Place an order on a perp market
    /// Accounts expected by this instruction (6):
    /// 0. `[]` mango_group_ai - TODO
    /// 1. `[writable]` mango_account_ai - TODO
    /// 2. `[signer]` owner_ai - TODO
    /// 3. `[]` mango_cache_ai - TODO
    /// 4. `[writable]` perp_market_ai - TODO
    /// 5. `[writable]` bids_ai - TODO
    /// 6. `[writable]` asks_ai - TODO
    /// 7. `[writable]` event_queue_ai - TODO
    PlacePerpOrder {
        price: i64,
        quantity: i64,
        client_order_id: u64,
        side: Side,
        order_type: OrderType,
    },

    CancelPerpOrderByClientId {
        client_order_id: u64,
    },

    CancelPerpOrder {
        order_id: i128,
        side: Side,
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

    UpdateFunding,

    // TODO - remove this instruction before mainnet
    SetOracle {
        price: I80F48,
    },

    /// Settle all funds from serum dex open orders
    ///
    /// Accounts expected by this instruction (14):
    ///
    /// 0. `[]` mango_group_ai - MangoGroup that this mango account is for
    /// 1. `[signer]` owner_ai - MangoAccount owner
    /// 2. `[writable]` mango_account_ai - MangoAccount
    /// 3. `[]` dex_prog_ai - program id of serum dex
    /// 4.  `[writable]` spot_market_ai - dex MarketState account
    /// 5.  `[writable]` open_orders_ai - open orders for this market for this MangoAccount
    /// 6. `[]` signer_ai - MangoGroup signer key
    /// 7. `[writable]` dex_base_ai - base vault for dex MarketState
    /// 8. `[writable]` dex_quote_ai - quote vault for dex MarketState
    /// 9. `[]` base_root_bank_ai - MangoGroup base vault acc
    /// 10. `[writable]` base_node_bank_ai - MangoGroup quote vault acc
    /// 11. `[]` quote_root_bank_ai - MangoGroup quote vault acc
    /// 12. `[writable]` quote_node_bank_ai - MangoGroup quote vault acc
    /// 13. `[writable]` base_vault_ai - MangoGroup base vault acc
    /// 14. `[writable]` quote_vault_ai - MangoGroup quote vault acc
    /// 15. `[]` dex_signer_ai - dex Market signer account
    /// 16. `[]` spl token program
    SettleFunds,

    /// Cancel an order using dex instruction
    ///
    /// Accounts expected by this instruction ():
    ///
    CancelSpotOrder {
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
    /// Accounts expected: 6
    SettlePnl {
        market_index: usize,
    },

    /// Use this token's position and deposit to reduce borrows
    ///
    /// Accounts expected by this instruction: 5
    SettleBorrow {
        token_index: usize,
        quantity: u64,
    },
}

impl MangoInstruction {
    pub fn unpack(input: &[u8]) -> Option<Self> {
        msg!("Merps: Unpacking");
        let (&discrim, data) = array_refs![input, 4; ..;];
        let discrim = u32::from_le_bytes(discrim);
        msg!("Discrim: {}", discrim);
        Some(match discrim {
            0 => {
                let data = array_ref![data, 0, 16];
                let (signer_nonce, valid_interval) = array_refs![data, 8, 8];

                MangoInstruction::InitMangoGroup {
                    signer_nonce: u64::from_le_bytes(*signer_nonce),
                    valid_interval: u64::from_le_bytes(*valid_interval),
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
                MangoInstruction::Withdraw {
                    quantity: u64::from_le_bytes(*quantity),
                    allow_borrow: allow_borrow,
                }
            }
            4 => {
                let data = array_ref![data, 0, 40];
                let (market_index, maint_leverage, init_leverage) = array_refs![data, 8, 16, 16];
                MangoInstruction::AddSpotMarket {
                    market_index: usize::from_le_bytes(*market_index),
                    maint_leverage: I80F48::from_le_bytes(*maint_leverage),
                    init_leverage: I80F48::from_le_bytes(*init_leverage),
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
                let data_arr = array_ref![data, 0, 56];
                let (market_index, maint_leverage, init_leverage, base_lot_size, quote_lot_size) =
                    array_refs![data_arr, 8, 16, 16, 8, 8];
                MangoInstruction::AddPerpMarket {
                    market_index: usize::from_le_bytes(*market_index),
                    maint_leverage: I80F48::from_le_bytes(*maint_leverage),
                    init_leverage: I80F48::from_le_bytes(*init_leverage),
                    base_lot_size: i64::from_le_bytes(*base_lot_size),
                    quote_lot_size: i64::from_le_bytes(*quote_lot_size),
                }
            }
            12 => {
                let data_arr = array_ref![data, 0, 26];
                let (price, quantity, client_order_id, side, order_type) =
                    array_refs![data_arr, 8, 8, 8, 1, 1];
                MangoInstruction::PlacePerpOrder {
                    price: i64::from_le_bytes(*price),
                    quantity: i64::from_le_bytes(*quantity),
                    client_order_id: u64::from_le_bytes(*client_order_id),
                    side: Side::try_from_primitive(side[0]).ok()?,
                    order_type: OrderType::try_from_primitive(order_type[0]).ok()?,
                }
            }
            13 => {
                let data_arr = array_ref![data, 0, 8];
                MangoInstruction::CancelPerpOrderByClientId {
                    client_order_id: u64::from_le_bytes(*data_arr),
                }
            }
            14 => {
                let data_arr = array_ref![data, 0, 17];
                let (order_id, side) = array_refs![data_arr, 16, 1];
                MangoInstruction::CancelPerpOrder {
                    order_id: i128::from_le_bytes(*order_id),
                    side: Side::try_from_primitive(side[0]).ok()?,
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
            _ => {
                return None;
            }
        })
    }
    pub fn pack(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap()
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
    mango_cache_ai: &Pubkey,
    dex_program_pk: &Pubkey,

    signer_nonce: u64,
    valid_interval: u64,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*mango_group_pk, false),
        AccountMeta::new_readonly(*signer_pk, false),
        AccountMeta::new_readonly(*admin_pk, true),
        AccountMeta::new_readonly(*quote_mint_pk, false),
        AccountMeta::new_readonly(*quote_vault_pk, false),
        AccountMeta::new(*quote_node_bank_pk, false),
        AccountMeta::new(*quote_root_bank_pk, false),
        AccountMeta::new(*mango_cache_ai, false),
        AccountMeta::new_readonly(*dex_program_pk, false),
    ];

    let instr = MangoInstruction::InitMangoGroup { signer_nonce, valid_interval };

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
        AccountMeta::new_readonly(solana_program::sysvar::rent::ID, false),
    ];

    let instr = MangoInstruction::InitMangoAccount;
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
    spot_market_pk: &Pubkey,
    dex_program_pk: &Pubkey,
    token_mint_pk: &Pubkey,
    node_bank_pk: &Pubkey,
    vault_pk: &Pubkey,
    root_bank_pk: &Pubkey,
    admin_pk: &Pubkey,

    market_index: usize,
    maint_leverage: I80F48,
    init_leverage: I80F48,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*mango_group_pk, false),
        AccountMeta::new_readonly(*spot_market_pk, false),
        AccountMeta::new_readonly(*dex_program_pk, false),
        AccountMeta::new_readonly(*token_mint_pk, false),
        AccountMeta::new(*node_bank_pk, false),
        AccountMeta::new_readonly(*vault_pk, false),
        AccountMeta::new(*root_bank_pk, false),
        AccountMeta::new_readonly(*admin_pk, true),
    ];

    let instr = MangoInstruction::AddSpotMarket { market_index, maint_leverage, init_leverage };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn add_perp_market(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    perp_market_pk: &Pubkey,
    event_queue_pk: &Pubkey,
    bids_pk: &Pubkey,
    asks_pk: &Pubkey,
    admin_pk: &Pubkey,

    market_index: usize,
    maint_leverage: I80F48,
    init_leverage: I80F48,
    base_lot_size: i64,
    quote_lot_size: i64,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*mango_group_pk, false),
        AccountMeta::new(*perp_market_pk, false),
        AccountMeta::new(*event_queue_pk, false),
        AccountMeta::new(*bids_pk, false),
        AccountMeta::new(*asks_pk, false),
        AccountMeta::new_readonly(*admin_pk, true),
    ];

    let instr = MangoInstruction::AddPerpMarket {
        market_index,
        maint_leverage,
        init_leverage,
        base_lot_size,
        quote_lot_size,
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
    open_orders_pks: &[Pubkey; MAX_PAIRS],
    side: Side,
    price: i64,
    quantity: i64,
    client_order_id: u64,
    order_type: OrderType,
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

    let instr =
        MangoInstruction::PlacePerpOrder { side, price, quantity, client_order_id, order_type };
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
    event_queue_pk: &Pubkey,   // write
    client_order_id: u64,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new(*perp_market_pk, false),
        AccountMeta::new(*bids_pk, false),
        AccountMeta::new(*asks_pk, false),
        AccountMeta::new(*event_queue_pk, false),
    ];
    let instr = MangoInstruction::CancelPerpOrderByClientId { client_order_id };
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
    event_queue_pk: &Pubkey,   // write
    order_id: i128,
    side: Side,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new(*mango_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new(*perp_market_pk, false),
        AccountMeta::new(*bids_pk, false),
        AccountMeta::new(*asks_pk, false),
        AccountMeta::new(*event_queue_pk, false),
    ];
    let instr = MangoInstruction::CancelPerpOrder { order_id, side };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn consume_events(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,      // read
    perp_market_pk: &Pubkey,      // read
    event_queue_pk: &Pubkey,      // write
    mango_acc_pks: &mut [Pubkey], // write
    limit: usize,
) -> Result<Instruction, ProgramError> {
    let fixed_accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new_readonly(*perp_market_pk, false),
        AccountMeta::new(*event_queue_pk, false),
    ];
    mango_acc_pks.sort();
    let mango_accounts = mango_acc_pks.into_iter().map(|pk| AccountMeta::new(*pk, false));
    let accounts = fixed_accounts.into_iter().chain(mango_accounts).collect();
    let instr = MangoInstruction::ConsumeEvents { limit };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn update_funding(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey, // read
    mango_cache_pk: &Pubkey, // read
    perp_market_pk: &Pubkey, // write
    bids_pk: &Pubkey,        // read
    asks_pk: &Pubkey,        // read
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
        AccountMeta::new_readonly(*mango_cache_pk, false),
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
        AccountMeta::new(*mango_group_pk, false),
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

pub fn add_oracle(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    oracle_pk: &Pubkey,
    admin_pk: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*mango_group_pk, false),
        AccountMeta::new_readonly(*oracle_pk, false),
        AccountMeta::new_readonly(*admin_pk, true),
    ];

    let instr = MangoInstruction::AddOracle;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn update_root_bank(
    program_id: &Pubkey,
    mango_group_pk: &Pubkey,
    root_bank_pk: &Pubkey,
    node_bank_pks: &[Pubkey],
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new_readonly(*mango_group_pk, false),
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
