use arrayref::{array_ref, array_refs};
use serde::{Deserialize, Serialize};
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;

#[repr(C)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MerpsInstruction {
    InitMerpsGroup {
        valid_interval: u8,
    },

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
    Deposit {
        quantity: u64,
    },
}

impl MerpsInstruction {
    pub fn unpack(input: &[u8]) -> Option<Self> {
        let (&discrim, data) = array_refs![input, 4; ..;];
        let discrim = u32::from_le_bytes(discrim);
        match discrim {
            0 => {
                let valid_interval = array_ref![data, 0, 1];

                Some(MerpsInstruction::InitMerpsGroup {
                    valid_interval: u8::from_le_bytes(*valid_interval),
                })
            }
            _ => None,
        }
    }
    pub fn pack(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap()
    }
}
