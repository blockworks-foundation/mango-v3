use arrayref::{array_ref, array_refs};
use solana_program::account_info::{Account, AccountInfo};
use solana_program::clock::Clock;
use solana_program::entrypoint::ProgramResult;
use solana_program::msg;
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;
use solana_program::sysvar::Sysvar;

use crate::error::{check_assert, MerpsError, MerpsErrorCode, MerpsResult, SourceFileId};
use crate::instruction::MerpsInstruction;
use crate::state::{
    Loadable, MerpsAccount, MerpsGroup, NodeBank, PriceCache, RootBank, RootBankCache, MAX_TOKENS,
    ZERO_U64F64,
};
use fixed::types::U64F64;

macro_rules! check {
    ($cond:expr, $err:expr) => {
        check_assert($cond, $err, line!(), SourceFileId::Processor)
    };
}

macro_rules! check_eq {
    ($x:expr, $y:expr, $err:expr) => {
        check_assert($x == $y, $err, line!(), SourceFileId::Processor)
    };
}

macro_rules! check_eq_default {
    ($x:expr, $y:expr) => {
        check_assert($x == $y, MerpsErrorCode::Default, line!(), SourceFileId::Processor)
    };
}

macro_rules! throw {
    () => {
        MerpsError::MerpsErrorCode {
            merps_error_code: MerpsErrorCode::Default,
            line: line!(),
            source_file_id: SourceFileId::State,
        }
    };
}

fn init_merps_group(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    valid_interval: u8,
) -> ProgramResult {
    const NUM_FIXED: usize = 1;
    let accounts = array_ref![accounts, 0, NUM_FIXED];
    let [merps_group_ai] = accounts;

    let mut merps_group = MerpsGroup::load_mut_checked(merps_group_ai, program_id)?;

    merps_group.valid_interval = valid_interval;
    // check size
    // check rent
    Ok(())
}

fn test_multi_tx(program_id: &Pubkey, accounts: &[AccountInfo], index: u8) -> ProgramResult {
    const NUM_FIXED: usize = 2;
    let accounts = array_ref![accounts, 0, NUM_FIXED];
    let [merps_group_ai, clock_ai] = accounts;
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
            return Ok(());
        }
    }

    msg!("Success");
    Ok(())
}

/// TODO figure out how to do docs for functions with link to instruction.rs instruction documentation
fn init_merps_account(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
    Ok(())
}

/// Deposit instruction
fn deposit(program_id: &Pubkey, accounts: &[AccountInfo], quantity: u64) -> ProgramResult {
    const NUM_FIXED: usize = 8;
    let accounts = array_ref![accounts, 0, NUM_FIXED];
    let [
        merps_group_ai,  // read
        merps_account_ai,  // write
        owner_ai,  // read
        root_bank_ai,  // read
        node_bank_ai,  // write
        vault_ai,  //
        token_prog_ai,
        owner_token_account_ai,
    ] = accounts;
    // TODO perform account checks

    let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;
    let mut merps_account =
        MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;
    check_eq!(&merps_account.owner, owner_ai.key, MerpsErrorCode::InvalidOwner)?;

    // find the index of the root bank pubkey in merps_group
    // if not found, error
    let token_index = merps_group.find_root_bank_index(root_bank_ai.key).ok_or(throw!())?;
    let mut node_bank = NodeBank::load_mut_checked(node_bank_ai, program_id)?;

    // Find the node_bank pubkey in root_bank, if not found error
    let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
    check!(root_bank.node_banks.contains(node_bank_ai.key), MerpsErrorCode::Default)?;

    // deposit into node bank token vault using invoke_transfer
    check_eq_default!(token_prog_ai.key, &spl_token::id())?;
    let deposit_instruction = spl_token::instruction::transfer(
        &spl_token::id(),
        owner_token_account_ai.key,
        vault_ai.key,
        owner_ai.key,
        &[],
        quantity,
    )?;

    let deposit_accs =
        [owner_token_account_ai.clone(), vault_ai.clone(), owner_ai.clone(), token_prog_ai.clone()];

    solana_program::program::invoke_signed(&deposit_instruction, &deposit_accs, &[])?;

    // increment merps account
    let deposit: U64F64 = U64F64::from_num(quantity) / root_bank.deposit_index;
    checked_add_deposit(&mut node_bank, &mut merps_account, token_index, deposit)?;

    Ok(())
}

/// Write oracle prices onto MerpsAccount before calling a value-dep instruction (e.g. Withdraw)    
fn cache_prices(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
    const NUM_FIXED: usize = 3;
    let (fixed_ais, oracle_ais) = array_refs![accounts, NUM_FIXED; ..;];
    let [
        merps_group_ai,     // read
        merps_account_ai,   // write
        clock_ai,           // read
    ] = fixed_ais;

    let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;
    let mut merps_account =
        MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;

    let clock = Clock::from_account_info(clock_ai)?;
    let now_ts = clock.unix_timestamp as u64;
    for oracle_ai in oracle_ais.iter() {
        let index = merps_group.find_oracle_index(oracle_ai.key).unwrap();
        merps_account.price_cache[index] =
            PriceCache { price: read_oracle(oracle_ai)?, last_update: now_ts };
    }
    Ok(())
}

fn cache_root_banks(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
    const NUM_FIXED: usize = 3;
    let (fixed_ais, root_bank_ais) = array_refs![accounts, NUM_FIXED; ..;];
    let [
        merps_group_ai,     // read
        merps_account_ai,   // write
        clock_ai,           // read
    ] = fixed_ais;

    let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;
    let mut merps_account =
        MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;
    let clock = Clock::from_account_info(clock_ai)?;
    let now_ts = clock.unix_timestamp as u64;
    for root_bank_ai in root_bank_ais.iter() {
        let index = merps_group.find_root_bank_index(root_bank_ai.key).unwrap();
        let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
        merps_account.root_bank_cache[index] = RootBankCache {
            deposit_index: root_bank.deposit_index,
            borrow_index: root_bank.borrow_index,
            last_update: now_ts,
        };
    }
    Ok(())
}

fn cache_open_orders(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
    // TODO
    Ok(())
}

fn cache_perp_market(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
    // TODO
    Ok(())
}

/// Withdraw a token from the bank if collateral ratio permits
fn withdraw(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    token_index: usize, // TODO: maybe make this u8 to reduce transaction size
    quantity: u64,
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
        token_prog_ai,      // read
        clock_ai,           // read
    ] = accounts;
    let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;

    let merps_account =
        MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;
    check!(&merps_account.owner == owner_ai.key, MerpsErrorCode::InvalidOwner)?;

    let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
    let node_bank = NodeBank::load_mut_checked(node_bank_ai, program_id)?;
    let clock = Clock::from_account_info(clock_ai)?;
    let now_ts = clock.unix_timestamp as u64;
    let valid_interval = merps_group.valid_interval as u64;

    // Verify there is root_bank_cache for the quote currency
    if now_ts > merps_account.root_bank_cache[MAX_TOKENS - 1].last_update + valid_interval {
        return Ok(());
    }
    for i in 0..merps_group.num_markets {
        // If this asset is not in user basket, then there are no deposits, borrows or perp positions to calculate value of
        if !merps_account.in_basket[i] {
            continue;
        }

        if now_ts > merps_account.price_cache[i].last_update + valid_interval {
            // TODO log or write to buffer that this transaction did not complete due to stale cache
            return Ok(());
        }
        if now_ts > merps_account.root_bank_cache[i].last_update + valid_interval {
            return Ok(());
        }
        if merps_account.open_orders[i] != Pubkey::default()
            && now_ts > merps_account.open_orders_cache[i].last_update + valid_interval
        {
            return Ok(());
        }
        if merps_group.perp_markets[i] != Pubkey::default()
            && now_ts > merps_account.perp_market_cache[i].last_update + valid_interval
        {
            return Ok(());
        }
    }

    // Now calculate the collateral ratio of the account using the various caches
    // Allow withdrawal if sufficient funds in deposits and collateral ratio above initial

    Ok(())
}

fn invoke_transfer<'a>(
    token_prog_acc: &AccountInfo<'a>,
    source_acc: &AccountInfo<'a>,
    dest_acc: &AccountInfo<'a>,
    authority_acc: &AccountInfo<'a>,
    signers_seeds: &[&[&[u8]]],
    quantity: u64,
) -> ProgramResult {
    let transfer_instruction = spl_token::instruction::transfer(
        &spl_token::ID,
        source_acc.key,
        dest_acc.key,
        authority_acc.key,
        &[],
        quantity,
    )?;
    let accs = [
        token_prog_acc.clone(), // TODO check if this is necessary
        source_acc.clone(),
        dest_acc.clone(),
        authority_acc.clone(),
    ];

    solana_program::program::invoke_signed(&transfer_instruction, &accs, signers_seeds)
}

/// Cranks should update all indexes in root banks TODO maybe update oracle prices as well?
fn update_indexes(program_id: &Pubkey, accounts: &[AccountInfo], quantity: u64) -> MerpsResult<()> {
    Ok(())
}

fn read_oracle(oracle_ai: &AccountInfo) -> MerpsResult<U64F64> {
    Ok(ZERO_U64F64) // TODO
}

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> MerpsResult<()> {
    let instruction = MerpsInstruction::unpack(data).ok_or(ProgramError::InvalidInstructionData)?;
    match instruction {
        MerpsInstruction::InitMerpsGroup { valid_interval } => {
            init_merps_group(program_id, accounts, valid_interval)?;
        }
        MerpsInstruction::TestMultiTx { index } => {
            test_multi_tx(program_id, accounts, index)?;
        }
        MerpsInstruction::Deposit { quantity } => {
            msg!("Merps: Deposit");
            deposit(program_id, accounts, quantity)?;
        }
    }

    Ok(())
}

fn checked_add_deposit(
    node_bank: &mut NodeBank,
    merps_account: &mut MerpsAccount,
    token_index: usize,
    quantity: U64F64,
) -> MerpsResult<()> {
    merps_account.checked_add_deposit(token_index, quantity)?;
    node_bank.checked_add_deposit(quantity)
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
