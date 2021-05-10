use solana_program::{
    msg, account_info::AccountInfo, entrypoint::ProgramResult, entrypoint, pubkey::Pubkey,
};
use crate::processor::process;


entrypoint!(process_instruction);
fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    process(program_id, accounts, instruction_data).map_err(
        |e| {
            msg!("{}", e);  // log the error
            e.into()  // convert MerpsError to generic ProgramError
        }
    )
}
