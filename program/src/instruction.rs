use arrayref::{array_ref, array_refs};
use serde::{Deserialize, Serialize};
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;

#[repr(C)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MerpsInstruction {
    InitMerpsGroup {
        valid_interval: u8
    },
    TestMultiTx {
        index: u8,
    }
}

impl MerpsInstruction {
    pub fn unpack(input: &[u8]) -> Option<Self> {
        let (&discrim, data) = array_refs![input, 4; ..;];
        let discrim = u32::from_le_bytes(discrim);
        match discrim {
            0 => {
                let valid_interval = array_ref![data, 0, 1];

                Some(MerpsInstruction::InitMerpsGroup {
                    valid_interval: u8::from_le_bytes(*valid_interval)
                })
            }
            1 => {
                let index = array_ref![data, 0, 1];
                Some(MerpsInstruction::TestMultiTx {
                    index: u8::from_le_bytes(*index)
                })
            }
            _ => None
        }
    }
    pub fn pack(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap()
    }

}