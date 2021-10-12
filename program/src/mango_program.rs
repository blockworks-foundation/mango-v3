use solana_program::declare_id;
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;

#[cfg(feature = "devnet")]
declare_id!("5fP7Z7a87ZEVsKr2tQPApdtq83GcTW4kz919R6ou5h5E");
#[cfg(not(feature = "devnet"))]
declare_id!("mv3ekLzLbnVPNxjSKvqBpU3ZeZXPQdEC3bp5MDEBG68");

#[derive(Clone)]
pub struct Mango;

impl anchor_lang::AccountDeserialize for Mango {
    fn try_deserialize(buf: &mut &[u8]) -> Result<Self, ProgramError> {
        Mango::try_deserialize_unchecked(buf)
    }

    fn try_deserialize_unchecked(_buf: &mut &[u8]) -> Result<Self, ProgramError> {
        Ok(Mango)
    }
}

impl anchor_lang::Id for Mango {
    fn id() -> Pubkey {
        ID
    }
}
