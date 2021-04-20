use solana_program::account_info::AccountInfo;
use solana_program::pubkey::Pubkey;
use crate::state::{MerpsGroup, Loadable, MAX_TOKENS};
use arrayref::{array_ref};
use solana_program::entrypoint::ProgramResult;
use crate::instruction::MerpsInstruction;
use solana_program::program_error::ProgramError;
use solana_program::clock::Clock;
use solana_program::sysvar::Sysvar;
use solana_program::msg;

pub struct Processor {}
impl Processor {
    ///
    fn init_merps_group(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
    ) -> ProgramResult {
        const NUM_FIXED: usize = 1;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            merps_group_ai,
        ] = accounts;

        let merps_group = MerpsGroup::load(merps_group_ai)?;

        // check size
        // check rent
        Ok(())

    }

    fn test_multi_tx(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        index: u8
    ) -> ProgramResult {
        const NUM_FIXED: usize = 2;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            merps_group_ai,
            clock_ai
        ] = accounts;
        let mut merps_group = MerpsGroup::load_mut(merps_group_ai)?;
        let clock = Clock::from_account_info(clock_ai)?;
        let curr_time = clock.unix_timestamp as u64;
        merps_group.last_updated[index as usize] = curr_time;

        // 10 open orders accounts
        // 10

        msg!("{} {}", index, clock.unix_timestamp);
        // last mut
        for i in 0..MAX_TOKENS {
            // if all are within certain bounds and last_mut (last time some function that changed state was called)
            // is before all updating
            if merps_group.last_updated[i] < curr_time - 2 {
                msg!("Failed");
                return Ok(())
            }
        }

        msg!("Success");
        Ok(())
    }

    pub fn process(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        data: &[u8]
    ) -> ProgramResult {
        let instruction = MerpsInstruction::unpack(data).ok_or(ProgramError::InvalidInstructionData)?;
        match instruction {
            MerpsInstruction::InitMerpsGroup => {
                Self::init_merps_group(program_id, accounts)?;
            }
            MerpsInstruction::TestMultiTx {
                index
            } => {
                Self::test_multi_tx(program_id, accounts, index)?;
            }
        }

        Ok(())
    }
}

/*
TODO list
1. mark price
2. oracle
3. liquidator
4. order book
5. crank
6. market makers
7. insurance fund
8. Basic DAO
9. Token Sale
10.

Crank keeps the oracle prices updated
Make adding perp markets very easy

Designs
Single Margin-Perp Cross
A perp market crossed with the equivalent serum dex spot market with lending pools for margin

find a way to combine multiple of these into one cross margined group

Write an arbitrageur to transfer USDC between different markets based on interest rate



Multi Perp Cross
Multiple perp markets cross margined with each other
Pros:

Cons:
1. Have to get liquidity across all markets at once (maybe doable if limited to 6 markets)
2. Can't do the carry trade easily
3.

 */