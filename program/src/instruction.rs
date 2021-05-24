use arrayref::{array_ref, array_refs};
use serde::{Deserialize, Serialize};
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;

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
    /// 6. `[writable]` merps_cache_ai - Account to cache prices, root banks, and perp markets
    /// 6. `[writable]` quote_node_bank_ai - TODO
    /// 7. `[writable]` quote_root_bank_ai - TODO
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
    /// 0. `[writable]` merps_group_ai - MerpsGroup that this merps account is for
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
    /// TODO
    Withdraw { quantity: u64 },

    /// Add a token to a merps group
    ///
    /// Accounts expected by this instruction (7):
    ///
    /// 0. `[writable]` merps_group_ai - TODO
    /// 1. `[]` mint_ai - TODO
    /// 2. `[writable]` node_bank_ai - TODO
    /// 3. `[]` vault_ai - TODO
    /// 4. `[writable]` root_bank_ai - TODO
    /// 5. `[]` oracle_ai - TODO
    /// 6. `[signer]` admin_ai - TODO
    AddAsset,

    /// Add a spot market to a merps group
    ///
    /// Accounts expected by this instruction (4)
    ///
    /// 0. `[writable]` merps_group_ai - TODO
    /// 1. `[]` spot_market_ai - TODO
    /// 2. `[]` dex_program_ai - TODO
    /// 3. `[signer]` admin_ai - TODO
    AddSpotMarket,

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
    /// 3. `[writable]` root_bank_ai - Root bank owned by MerpsGroup
    /// 4. `[writable]` node_bank_ai - Node bank owned by RootBank
    /// 5. `[]` clock_ai - Clock sysvar account
    Borrow { quantity: u64 },
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
                let data = array_ref![data, 0, 8];
                MerpsInstruction::Withdraw { quantity: u64::from_le_bytes(*data) }
            }
            4 => MerpsInstruction::AddAsset,
            5 => MerpsInstruction::AddSpotMarket,
            6 => MerpsInstruction::AddToBasket,
            7 => {
                let quantity = array_ref![data, 0, 8];
                MerpsInstruction::Borrow { quantity: u64::from_le_bytes(*quantity) }
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

pub fn init_merps_group(
    program_id: &Pubkey,
    merps_group_pk: &Pubkey,
    signer_pk: &Pubkey,
    admin_pk: &Pubkey,
    quote_mint_pk: &Pubkey,
    quote_vault_pk: &Pubkey,
    quote_node_bank_pk: &Pubkey,
    quote_root_bank_pk: &Pubkey,
    dex_program_pk: &Pubkey,

    signer_nonce: u64,
    valid_interval: u8,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*merps_group_pk, false),
        AccountMeta::new_readonly(solana_program::sysvar::rent::ID, false),
        AccountMeta::new_readonly(*signer_pk, false),
        AccountMeta::new_readonly(*admin_pk, true),
        AccountMeta::new_readonly(*quote_mint_pk, false),
        AccountMeta::new_readonly(*quote_vault_pk, false),
        AccountMeta::new(*quote_node_bank_pk, false),
        AccountMeta::new(*quote_root_bank_pk, false),
        AccountMeta::new(*dex_program_pk, false),
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
        AccountMeta::new(*merps_group_pk, false),
        AccountMeta::new(*merps_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new(*root_bank_pk, false),
        AccountMeta::new(*node_bank_pk, false),
        AccountMeta::new(*vault_pk, false),
        AccountMeta::new_readonly(spl_token::ID, false),
        AccountMeta::new(*owner_token_account_pk, false),
    ];

    let instr = MerpsInstruction::Deposit { quantity };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn add_asset(
    program_id: &Pubkey,
    merps_group_pk: &Pubkey,
    token_mint_pk: &Pubkey,
    node_bank_pk: &Pubkey,
    vault_pk: &Pubkey,
    root_bank_pk: &Pubkey,
    oracle_pk: &Pubkey,
    admin_pk: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*merps_group_pk, false),
        AccountMeta::new_readonly(*token_mint_pk, false),
        AccountMeta::new(*node_bank_pk, false),
        AccountMeta::new_readonly(*vault_pk, false),
        AccountMeta::new(*root_bank_pk, false),
        AccountMeta::new_readonly(*oracle_pk, false),
        AccountMeta::new_readonly(*admin_pk, true),
    ];

    let instr = MerpsInstruction::AddAsset;
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn add_spot_market(
    program_id: &Pubkey,
    merps_group_pk: &Pubkey,
    spot_market_pk: &Pubkey,
    dex_program_pk: &Pubkey,
    admin_pk: &Pubkey,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*merps_group_pk, false),
        AccountMeta::new_readonly(*spot_market_pk, false),
        AccountMeta::new_readonly(*dex_program_pk, false),
        AccountMeta::new_readonly(*admin_pk, true),
    ];

    let instr = MerpsInstruction::AddSpotMarket;
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

pub fn borrow(
    program_id: &Pubkey,
    merps_group_pk: &Pubkey,
    merps_account_pk: &Pubkey,
    owner_pk: &Pubkey,
    root_bank_pk: &Pubkey,
    node_bank_pk: &Pubkey,

    quantity: u64,
) -> Result<Instruction, ProgramError> {
    let accounts = vec![
        AccountMeta::new(*merps_group_pk, false),
        AccountMeta::new(*merps_account_pk, false),
        AccountMeta::new_readonly(*owner_pk, true),
        AccountMeta::new(*root_bank_pk, false),
        AccountMeta::new(*node_bank_pk, false),
        AccountMeta::new_readonly(solana_program::sysvar::clock::ID, false),
    ];

    let instr = MerpsInstruction::Borrow { quantity };
    let data = instr.pack();
    Ok(Instruction { program_id: *program_id, accounts, data })
}
