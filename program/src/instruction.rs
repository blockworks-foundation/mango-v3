use arrayref::{array_ref, array_refs};
use fixed::types::I80F48;
use num_enum::TryFromPrimitive;
use serde::{Deserialize, Serialize};
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;
use std::convert::TryInto;
use std::num::NonZeroU64;

#[repr(C)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MerpsInstruction {
    /// Initialize a group of lending pools that can be cross margined
    ///
    /// Accounts expected by this instruction (9):
    ///
    /// 0. `[writable]` merps_group_ai - TODO
    /// 1. `[]` rent_ai - TODO
    /// 2. `[]` signer_ai - TODO
    /// 3. `[]` admin_ai - TODO
    /// 4. `[]` quote_mint_ai - TODO
    /// 5. `[]` quote_vault_ai - TODO
    /// 6. `[writable]` quote_node_bank_ai - TODO
    /// 7. `[writable]` quote_root_bank_ai - TODO
    /// 6. `[writable]` merps_cache_ai - Account to cache prices, root banks, and perp markets
    /// 8. `[]` dex_prog_ai - TODO
    InitMerpsGroup { signer_nonce: u64, valid_interval: u8 },

    /// Initialize a merps account for a user
    ///
    /// Accounts expected by this instruction (4):
    ///
    /// 0. `[]` merps_group_ai - MerpsGroup that this merps account is for
    /// 1. `[writable]` merps_account_ai - the merps account data
    /// 2. `[signer]` owner_ai - Solana account of owner of the merps account
    /// 3. `[]` rent_ai - Rent sysvar account
    InitMerpsAccount,

    /// Deposit funds into merps account
    ///
    /// Accounts expected by this instruction (8):
    ///
    /// 0. `[]` merps_group_ai - MerpsGroup that this merps account is for
    /// 1. `[writable]` merps_account_ai - the merps account for this user
    /// 2. `[signer]` owner_ai - Solana account of owner of the merps account
    /// 3. `[]` root_bank_ai - RootBank owned by MerpsGroup
    /// 4. `[writable]` node_bank_ai - NodeBank owned by RootBank
    /// 5. `[writable]` vault_ai - TokenAccount owned by MerpsGroup
    /// 6. `[]` token_prog_ai - acc pointed to by SPL token program id
    /// 7. `[writable]` owner_token_account_ai - TokenAccount owned by user which will be sending the funds
    Deposit { quantity: u64 },

    /// Withdraw funds that were deposited earlier.
    ///
    /// Accounts expected by this instruction (10):
    ///
    /// 0. `[read]` merps_group_ai,   -
    /// 1. `[write]` merps_account_ai, -
    /// 2. `[read]` owner_ai,         -
    /// 3. `[read]` merps_cache_ai,   -
    /// 4. `[read]` root_bank_ai,     -
    /// 5. `[write]` node_bank_ai,     -
    /// 6. `[write]` vault_ai,         -
    /// 7. `[write]` token_account_ai, -
    /// 8. `[read]` signer_ai,        -
    /// 9. `[read]` token_prog_ai,    -
    /// 10. `[read]` clock_ai,         -
    /// 11..+ `[]` open_orders_accs - open orders for each of the spot market
    Withdraw { quantity: u64, allow_borrow: bool },

    /// Add a token to a merps group
    ///
    /// Accounts expected by this instruction (7):
    ///
    /// 0. `[writable]` merps_group_ai - TODO
    /// 1. `[]` spot_market_ai - TODO
    /// 2. `[]` dex_program_ai - TODO
    /// 1. `[]` mint_ai - TODO
    /// 2. `[writable]` node_bank_ai - TODO
    /// 3. `[]` vault_ai - TODO
    /// 4. `[writable]` root_bank_ai - TODO
    /// 5. `[]` oracle_ai - TODO
    /// 6. `[signer]` admin_ai - TODO
    AddSpotMarket { maint_asset_weight: I80F48, init_asset_weight: I80F48 },

    /// Add a spot market to a merps account basket
    ///
    /// Accounts expected by this instruction (6)
    ///
    /// 0. `[]` merps_group_ai - TODO
    /// 1. `[writable]` merps_account_ai - TODO
    /// 2. `[signer]` owner_ai - Solana account of owner of the merps account
    /// 3. `[]` spot_market_ai - TODO
    AddToBasket,

    /// Borrow by incrementing MerpsAccount.borrows given collateral ratio is below init_coll_rat
    ///
    /// Accounts expected by this instruction (4 + 2 * NUM_MARKETS):
    ///
    /// 0. `[]` merps_group_ai - MerpsGroup that this merps account is for
    /// 1. `[writable]` merps_account_ai - the merps account for this user
    /// 2. `[signer]` owner_ai - Solana account of owner of the MerpsAccount
    /// 3. `[]` merps_cache_ai - TODO
    /// 4. `[]` root_bank_ai - Root bank owned by MerpsGroup
    /// 5. `[writable]` node_bank_ai - Node bank owned by RootBank
    /// 6. `[]` clock_ai - Clock sysvar account
    Borrow { quantity: u64 },

    /// Cache prices
    ///
    /// Accounts expected: 3 + Oracles
    /// 0. `[]` merps_group_ai -
    /// 1. `[writable]` merps_cache_ai -
    /// 2. `[]` clock_ai -
    /// 3+... `[]` oracle_ais - flux aggregator feed accounts
    CachePrices,

    /// Cache root banks
    ///
    /// Accounts expected: 3 + Root Banks
    CacheRootBanks,

    /// Place an order on the Serum Dex using Merps account
    ///
    /// Accounts expected by this instruction (19 + MAX_PAIRS):
    ///
    PlaceSpotOrder { order: serum_dex::instruction::NewOrderInstructionV3 },
}

impl MerpsInstruction {
    pub fn unpack(input: &[u8]) -> Option<Self> {
        let (&discrim, data) = array_refs![input, 4; ..;];
        let discrim = u32::from_le_bytes(discrim);
        Some(match discrim {
            0 => {
                let data = array_ref![data, 0, 9];
                let (signer_nonce, valid_interval) = array_refs![data, 8, 1];

                MerpsInstruction::InitMerpsGroup {
                    signer_nonce: u64::from_le_bytes(*signer_nonce),
                    valid_interval: u8::from_le_bytes(*valid_interval),
                }
            }
            1 => MerpsInstruction::InitMerpsAccount,
            2 => {
                let quantity = array_ref![data, 0, 8];
                MerpsInstruction::Deposit { quantity: u64::from_le_bytes(*quantity) }
            }
            3 => {
                let data = array_ref![data, 0, 9];
                let (quantity, allow_borrow) = array_refs![data, 8, 1];

                let allow_borrow = match allow_borrow {
                    [0] => false,
                    [1] => true,
                    _ => return None,
                };
                MerpsInstruction::Withdraw {
                    quantity: u64::from_le_bytes(*quantity),
                    allow_borrow: allow_borrow,
                }
            }
            4 => {
                let data = array_ref![data, 0, 32];
                let (maint_asset_weight, init_asset_weight) = array_refs![data, 16, 16];
                MerpsInstruction::AddSpotMarket {
                    maint_asset_weight: I80F48::from_le_bytes(*maint_asset_weight),
                    init_asset_weight: I80F48::from_le_bytes(*init_asset_weight),
                }
            }
            5 => MerpsInstruction::AddToBasket,
            6 => {
                let quantity = array_ref![data, 0, 8];
                MerpsInstruction::Borrow { quantity: u64::from_le_bytes(*quantity) }
            }
            7 => MerpsInstruction::CachePrices,
            8 => MerpsInstruction::CacheRootBanks,
            9 => {
                let data_arr = array_ref![data, 0, 46];
                let order = unpack_dex_new_order_v3(data_arr)?;
                MerpsInstruction::PlaceSpotOrder { order }
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

pub fn init_merps_group(
    program_id: &Pubkey,
    merps_group_pk: &Pubkey,
    signer_pk: &Pubkey,
    admin_pk: &Pubkey,
    quote_mint_pk: &Pubkey,
    quote_vault_pk: &Pubkey,
    quote_node_bank_pk: &Pubkey,
    quote_root_bank_pk: &Pubkey,
    merps_cache_ai: &Pubkey,
    dex_program_pk: &Pubkey,

    signer_nonce: u64,
    valid_interval: u8,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*merps_group_pk, false),
        AccountMeta::new_readonly(*signer_pk, false),
        AccountMeta::new_readonly(*admin_pk, true),
        AccountMeta::new_readonly(*quote_mint_pk, false),
        AccountMeta::new_readonly(*quote_vault_pk, false),
        AccountMeta::new(*quote_node_bank_pk, false),
        AccountMeta::new(*quote_root_bank_pk, false),
        AccountMeta::new(*merps_cache_ai, false),
        AccountMeta::new_readonly(*dex_program_pk, false),
    ];

    let instr = MerpsInstruction::InitMerpsGroup { signer_nonce, valid_interval };

    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn init_merps_account(
    program_id: &Pubkey,
    merps_group_pk: &Pubkey,
    merps_account_pk: &Pubkey,
    owner_pk: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*merps_group_pk, false),
        AccountMeta::new(*merps_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new_readonly(solana_program::sysvar::rent::ID, false),
    ];

    let instr = MerpsInstruction::InitMerpsAccount;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn deposit(
    program_id: &Pubkey,
    merps_group_pk: &Pubkey,
    merps_account_pk: &Pubkey,
    owner_pk: &Pubkey,
    root_bank_pk: &Pubkey,
    node_bank_pk: &Pubkey,
    vault_pk: &Pubkey,
    owner_token_account_pk: &Pubkey,

    quantity: u64,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*merps_group_pk, false),
        AccountMeta::new(*merps_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new_readonly(*root_bank_pk, false),
        AccountMeta::new(*node_bank_pk, false),
        AccountMeta::new(*vault_pk, false),
        AccountMeta::new_readonly(spl_token::ID, false),
        AccountMeta::new(*owner_token_account_pk, false),
    ];

    let instr = MerpsInstruction::Deposit { quantity };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn add_spot_market(
    program_id: &Pubkey,
    merps_group_pk: &Pubkey,
    spot_market_pk: &Pubkey,
    dex_program_pk: &Pubkey,
    token_mint_pk: &Pubkey,
    node_bank_pk: &Pubkey,
    vault_pk: &Pubkey,
    root_bank_pk: &Pubkey,
    oracle_pk: &Pubkey,
    admin_pk: &Pubkey,

    maint_asset_weight: I80F48,
    init_asset_weight: I80F48,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*merps_group_pk, false),
        AccountMeta::new_readonly(*spot_market_pk, false),
        AccountMeta::new_readonly(*dex_program_pk, false),
        AccountMeta::new_readonly(*token_mint_pk, false),
        AccountMeta::new(*node_bank_pk, false),
        AccountMeta::new_readonly(*vault_pk, false),
        AccountMeta::new(*root_bank_pk, false),
        AccountMeta::new_readonly(*oracle_pk, false),
        AccountMeta::new_readonly(*admin_pk, true),
    ];

    let instr = MerpsInstruction::AddSpotMarket { maint_asset_weight, init_asset_weight };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn add_to_basket(
    program_id: &Pubkey,
    merps_group_pk: &Pubkey,
    merps_account_pk: &Pubkey,
    owner_pk: &Pubkey,
    spot_market_pk: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new_readonly(*merps_group_pk, false),
        AccountMeta::new(*merps_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new_readonly(*spot_market_pk, false),
    ];

    let instr = MerpsInstruction::AddToBasket;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn withdraw(
    program_id: &Pubkey,
    merps_group_pk: &Pubkey,
    merps_account_pk: &Pubkey,
    owner_pk: &Pubkey,
    merps_cache_pk: &Pubkey,
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
        AccountMeta::new(*merps_group_pk, false),
        AccountMeta::new(*merps_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new_readonly(*merps_cache_pk, false),
        AccountMeta::new_readonly(*root_bank_pk, false),
        AccountMeta::new(*node_bank_pk, false),
        AccountMeta::new(*vault_pk, false),
        AccountMeta::new(*token_account_pk, false),
        AccountMeta::new_readonly(*signer_pk, false),
        AccountMeta::new_readonly(spl_token::ID, false),
        AccountMeta::new_readonly(solana_program::sysvar::clock::ID, false),
    ];

    accounts.extend(open_orders_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));

    let instr = MerpsInstruction::Withdraw { quantity, allow_borrow };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn borrow(
    program_id: &Pubkey,
    merps_group_pk: &Pubkey,
    merps_account_pk: &Pubkey,
    merps_cache_pk: &Pubkey,
    owner_pk: &Pubkey,
    root_bank_pk: &Pubkey,
    node_bank_pk: &Pubkey,
    open_orders_pks: &[Pubkey],

    quantity: u64,
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new(*merps_group_pk, false),
        AccountMeta::new(*merps_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new_readonly(*merps_cache_pk, false),
        AccountMeta::new_readonly(*root_bank_pk, false),
        AccountMeta::new(*node_bank_pk, false),
    ];

    accounts.extend(open_orders_pks.iter().map(|pk| AccountMeta::new(*pk, false)));

    let instr = MerpsInstruction::Borrow { quantity };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn cache_prices(
    program_id: &Pubkey,
    merps_group_pk: &Pubkey,
    merps_cache_pk: &Pubkey,
    oracle_pks: &[Pubkey],
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new_readonly(*merps_group_pk, false),
        AccountMeta::new(*merps_cache_pk, false),
    ];
    accounts.extend(oracle_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));
    let instr = MerpsInstruction::CachePrices;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn cache_root_banks(
    program_id: &Pubkey,
    merps_group_pk: &Pubkey,
    merps_cache_pk: &Pubkey,
    root_bank_pks: &[Pubkey],
) -> Result<Instruction, ProgramError> {
    let mut accounts = vec![
        AccountMeta::new_readonly(*merps_group_pk, false),
        AccountMeta::new(*merps_cache_pk, false),
    ];
    accounts.extend(root_bank_pks.iter().map(|pk| AccountMeta::new_readonly(*pk, false)));
    let instr = MerpsInstruction::CacheRootBanks;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}
