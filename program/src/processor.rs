use arrayref::{array_ref, array_refs};
use solana_program::account_info::{Account, AccountInfo};
use solana_program::clock::Clock;
use solana_program::entrypoint::ProgramResult;
use solana_program::msg;
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;
use solana_program::sysvar::Sysvar;

use crate::error::{MerpsResult, check_assert, MerpsErrorCode, MerpsError, SourceFileId};
use crate::instruction::MerpsInstruction;
use crate::state::{Loadable, MAX_TOKENS, MerpsAccount, MerpsGroup, NodeBank, RootBank, ZERO_U64F64, PriceInfo};
use fixed::types::U64F64;

macro_rules! check {
    ($cond:expr, $err:expr) => {
        check_assert($cond, $err, line!(), SourceFileId::Processor)
    }
}

macro_rules! check_eq {
    ($x:expr, $y:expr, $err:expr) => {
        check_assert($x == $y, $err, line!(), SourceFileId::Processor)
    }
}


fn init_merps_group(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    valid_interval: u8
) -> ProgramResult {
    const NUM_FIXED: usize = 1;
    let accounts = array_ref![accounts, 0, NUM_FIXED];
    let [
        merps_group_ai,
    ] = accounts;

    let mut merps_group = MerpsGroup::load_mut_checked(merps_group_ai, program_id)?;

    merps_group.valid_interval = valid_interval;
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
    let mut merps_group = MerpsGroup::load_mut_checked(merps_group_ai, program_id)?;
    let clock = Clock::from_account_info(clock_ai)?;
    let curr_time = clock.unix_timestamp as u64;
    merps_group.last_updated[index as usize] = curr_time;


    msg!("{} {}", index, clock.unix_timestamp);
    // last mut
    let valid_interval = merps_group.valid_interval as u64;
    for i in 0..MAX_TOKENS {
        // if all are within certain bounds and last_mut (last time some function that changed state was called)
        // is before all updating

        if merps_group.last_updated[i] < curr_time - valid_interval {
            msg!("Failed");
            return Ok(())
        }
    }

    msg!("Success");
    Ok(())
}

/// TODO figure out how to do docs for functions with link to instruction.rs instruction documentation
fn init_merps_account(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> MerpsResult<()> {
    Ok(())
}

/// Deposit instruction
fn deposit(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    quantity: u64
) -> ProgramResult {
    const NUM_FIXED: usize = 7;
    let accounts = array_ref![accounts, 0, NUM_FIXED];
    let [
        merps_group_ai,  // read
        merps_account_ai,  // write
        owner_ai,  // read
        root_bank_ai,  // read
        node_bank_ai,  // write
        vault_ai,  //
        token_prog_ai,
    ] = accounts;

    // TODO perform account checks

    let merps_group = MerpsGroup::load(merps_group_ai)?;
    let merps_account = MerpsAccount::load_mut(merps_account_ai)?;

    let root_bank = RootBank::load(root_bank_ai)?;
    // find the index of the root bank pubkey in merps_group
    // if not found, error

    let node_bank = NodeBank::load_mut(node_bank_ai)?;

    // Find the node_bank pubkey in root_bank, if not found error

    // deposit into node bank token vault using invoke_transfer
    // increment merps account

    Ok(())
}


/// To be called to write oracle prices onto MerpsAccount before calling a value-dep instruction (e.g. Withdraw)
fn update_prices(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> MerpsResult<()> {
    const NUM_FIXED: usize = 3;
    let (fixed_ais, oracle_ais) = array_refs![accounts, NUM_FIXED; ..;];
    let [
        merps_group_ai,     // read
        merps_account_ai,   // write
        clock_ai,           // read
    ] = fixed_ais;

    let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;
    let mut merps_account = MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;
    let clock = Clock::from_account_info(clock_ai)?;
    let now_ts = clock.unix_timestamp as u64;
    for oracle_ai in oracle_ais.iter() {
        let index = merps_group.find_oracle_index(oracle_ai.key).unwrap();
        merps_account.prices[index] = PriceInfo {
            price: read_oracle(oracle_ai)?,
            last_update: now_ts
        };
    }

    Ok(())
}

/// Withdraw a token from the bank if collateral ratio permits
fn withdraw(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    token_index: usize,  // maybe make this u8 to reduce transaction size
    quantity: u64
) -> MerpsResult<()> {

    const NUM_FIXED: usize = 8;
    let accounts = array_ref![accounts, 0, NUM_FIXED];
    let [
        merps_group_ai,     // read
        merps_account_ai,   // write
        owner_ai,           // read
        root_bank_ai,       // read
        node_bank_ai,       // write
        vault_ai,           // write
        token_prog_acc,     // read
        clock_acc,          // read
    ] = accounts;

    // need a list of root banks for tokens in basket
    // need a list of open orders for tokens in basket
    // need a list of

    let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;

    let merps_account = MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;
    check_eq!(&merps_account.owner, owner_ai.key, MerpsErrorCode::InvalidOwner)?;

    let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
    let node_bank = NodeBank::load_mut_checked(node_bank_ai, program_id)?;

    for i in 0..merps_group.num_markets {
        // If this asset is not in user basket, then there are no deposits, borrows or perp positions to calculate value of
        if !merps_account.in_basket[i] {
            continue
        }

    }

    // iterate through all the oracle prices and see if it was updated recently

    /*
        Find value of all the tokens that have a borrow or withdraw balance
            To get the value, need to convert each deposit and withdraw into native terms
            need to pass in the root bank for each of the tokens
            need to pass in the oracle for each token
            also do the calculation?

            TODO: consider putting root banks inside the MerpsGroup
                pro: fewer tokens to pass in
                con: Perhaps we might decide to update index on withdraw, liquidate etc., but then that'll become single threaded
                maybe it makes the lending pools more dependent on Mango (?)

        Find value of all perp positions
            multiply

    1.
     */

    Ok(())
}

fn invoke_transfer<'a>(
    token_prog_acc: &AccountInfo<'a>,
    source_acc: &AccountInfo<'a>,
    dest_acc: &AccountInfo<'a>,
    authority_acc: &AccountInfo<'a>,
    signers_seeds: &[&[&[u8]]],
    quantity: u64
) -> ProgramResult {
    let transfer_instruction = spl_token::instruction::transfer(
        &spl_token::ID,
        source_acc.key,
        dest_acc.key,
        authority_acc.key,
        &[],
        quantity
    )?;
    let accs = [
        token_prog_acc.clone(),  // TODO check if this is necessary
        source_acc.clone(),
        dest_acc.clone(),
        authority_acc.clone()
    ];

    solana_program::program::invoke_signed(&transfer_instruction, &accs, signers_seeds)
}

/// Cranks should update all indexes in root banks TODO maybe update oracle prices as well?
fn update_indexes(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    quantity: u64
) -> MerpsResult<()> {

    Ok(())
}

fn read_oracle(
    oracle_ai: &AccountInfo
) -> MerpsResult<U64F64> {
    Ok(ZERO_U64F64)  // TODO
}

pub fn process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8]
) -> MerpsResult<()> {
    let instruction = MerpsInstruction::unpack(data).ok_or(ProgramError::InvalidInstructionData)?;
    match instruction {
        MerpsInstruction::InitMerpsGroup {
            valid_interval
        } => {
            init_merps_group(program_id, accounts, valid_interval)?;
        }
        MerpsInstruction::TestMultiTx {
            index
        } => {
            test_multi_tx(program_id, accounts, index)?;
        }
    }

    Ok(())
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


NOTE: inform users the more tokens they use with cross margin, worse performance
 */