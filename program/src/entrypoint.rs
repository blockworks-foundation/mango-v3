use crate::processor::process;
use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult, msg, pubkey::Pubkey,
};

entrypoint!(process_instruction);
fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    process(program_id, accounts, instruction_data).map_err(|e| {
        msg!("{}", e); // log the error
        e.into() // convert MerpsError to generic ProgramError
    })
}
