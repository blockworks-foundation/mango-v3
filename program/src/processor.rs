use std::mem::size_of;

use arrayref::{array_ref, array_refs};
use bytemuck::bytes_of;
use fixed::types::I80F48;
use flux_aggregator::borsh_state::InitBorshState;

use serum_dex::state::ToAlignedBytes;
use solana_program::account_info::AccountInfo;
use solana_program::clock::Clock;
use solana_program::entrypoint::ProgramResult;
use solana_program::msg;
use solana_program::program_error::ProgramError;
use solana_program::program_pack::{IsInitialized, Pack};
use solana_program::pubkey::Pubkey;
use solana_program::rent::Rent;
use solana_program::sysvar::Sysvar;
use spl_token::state::Account;

use crate::error::{check_assert, MerpsError, MerpsErrorCode, MerpsResult, SourceFileId};
use crate::instruction::MerpsInstruction;
use crate::matching::Side;
use crate::state::{
    load_market_state, DataType, MerpsAccount, MerpsCache, MerpsGroup, NodeBank, PriceCache,
    RootBank, RootBankCache, ONE_I80F48, QUOTE_INDEX, ZERO_I80F48,
};
use crate::utils::gen_signer_key;
use mango_common::Loadable;

declare_check_assert_macros!(SourceFileId::Processor);

pub struct Processor {}

impl Processor {
    fn init_merps_group(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        signer_nonce: u64,
        valid_interval: u8,
    ) -> ProgramResult {
        const NUM_FIXED: usize = 10;
        let accounts = array_ref![accounts, 0, NUM_FIXED];

        let [merps_group_ai, rent_ai, signer_ai, admin_ai, quote_mint_ai, quote_vault_ai, merps_cache_ai, quote_node_bank_ai, quote_root_bank_ai, dex_prog_ai] =
            accounts;
        // Q: do we need the dex_program_id stored on merps group?

        check_eq!(merps_group_ai.owner, program_id, MerpsErrorCode::InvalidGroupOwner)?;
        let rent = Rent::from_account_info(rent_ai)?;
        check!(
            rent.is_exempt(merps_group_ai.lamports(), size_of::<MerpsGroup>()),
            MerpsErrorCode::GroupNotRentExempt
        )?;

        let mut merps_group = MerpsGroup::load_mut_checked(merps_group_ai, program_id)?;
        // TODO is there a security concern if we remove the merps_group_ai.key?
        check!(
            gen_signer_key(signer_nonce, merps_group_ai.key, program_id)? == *signer_ai.key,
            MerpsErrorCode::InvalidSignerKey
        )?;
        merps_group.signer_nonce = signer_nonce;
        merps_group.signer_key = *signer_ai.key;
        merps_group.valid_interval = valid_interval;
        merps_group.dex_program_id = *dex_prog_ai.key;

        let _root_bank = init_root_bank(
            program_id,
            &merps_group,
            quote_mint_ai,
            quote_vault_ai,
            quote_root_bank_ai,
            quote_node_bank_ai,
        )?;

        merps_group.tokens[QUOTE_INDEX] = *quote_mint_ai.key;
        merps_group.root_banks[QUOTE_INDEX] = *quote_root_bank_ai.key;
        merps_group.num_tokens = 1;
        merps_group.num_markets = 0;

        check!(admin_ai.is_signer, MerpsErrorCode::Default)?;
        merps_group.admin = *admin_ai.key;

        merps_group.meta_data.data_type = DataType::MerpsGroup as u8;
        merps_group.meta_data.is_initialized = true;
        merps_group.meta_data.version = 0;

        // init MerpsCache
        merps_group.merps_cache = *merps_cache_ai.key;
        let mut merps_cache = MerpsCache::load_mut(&merps_cache_ai)?;
        merps_cache.meta_data.data_type = DataType::MerpsCache as u8;
        merps_cache.meta_data.is_initialized = true;
        merps_cache.meta_data.version = 0;

        // check size
        Ok(())
    }

    /// TODO figure out how to do docs for functions with link to instruction.rs instruction documentation
    /// TODO make the merps account a derived address
    fn init_merps_account(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        const NUM_FIXED: usize = 4;
        let accounts = array_ref![accounts, 0, NUM_FIXED];

        let [merps_group_ai, merps_account_ai, owner_ai, rent_ai] = accounts;

        let rent = Rent::from_account_info(rent_ai)?;
        check!(
            rent.is_exempt(merps_account_ai.lamports(), size_of::<MerpsAccount>()),
            MerpsErrorCode::Default
        )?;
        check!(owner_ai.is_signer, MerpsErrorCode::Default)?;

        #[allow(unused_variables)]
        let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;

        let mut merps_account = MerpsAccount::load_mut(merps_account_ai)?;
        check_eq!(&merps_account_ai.owner, &program_id, MerpsErrorCode::InvalidOwner)?;

        merps_account.merps_group = *merps_group_ai.key;
        merps_account.owner = *owner_ai.key;
        merps_account.meta_data.data_type = DataType::MerpsAccount as u8;
        merps_account.meta_data.is_initialized = true;
        merps_account.meta_data.version = 0;

        Ok(())
    }

    /// Initialize a root bank and add it to the merps group
    /// add_asset only adds the borrowing and lending functionality for an asset
    /// Requires a price oracle for this asset priced in quote currency
    /// Only allow admin to add to MerpsGroup
    fn add_asset(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        const NUM_FIXED: usize = 7;
        let accounts = array_ref![accounts, 0, NUM_FIXED];

        let [merps_group_ai, mint_ai, node_bank_ai, vault_ai, root_bank_ai, oracle_ai, admin_ai] =
            accounts;

        check!(admin_ai.is_signer, MerpsErrorCode::Default)?;
        let mut merps_group = MerpsGroup::load_mut_checked(merps_group_ai, program_id)?;
        check_eq!(admin_ai.key, &merps_group.admin, MerpsErrorCode::Default)?;

        let token_index = merps_group.num_tokens;

        let _root_bank = init_root_bank(
            program_id,
            &merps_group,
            mint_ai,
            vault_ai,
            root_bank_ai,
            node_bank_ai,
        )?;

        merps_group.tokens[token_index] = *mint_ai.key;
        merps_group.root_banks[token_index] = *root_bank_ai.key;

        let _oracle = flux_aggregator::state::Aggregator::load_initialized(&oracle_ai)?;
        merps_group.oracles[token_index] = *oracle_ai.key;
        merps_group.num_tokens += 1;

        Ok(())
    }

    // TODO think about how to remove an asset. Maybe this just can't be done?
    /// Add spot market to merps group
    fn add_spot_market(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        const NUM_FIXED: usize = 4;
        let accounts = array_ref![accounts, 0, NUM_FIXED];

        let [merps_group_ai, spot_market_ai, dex_program_ai, admin_ai] = accounts;

        let mut merps_group = MerpsGroup::load_mut_checked(merps_group_ai, program_id)?;

        check!(admin_ai.is_signer, MerpsErrorCode::Default)?;
        check_eq!(admin_ai.key, &merps_group.admin, MerpsErrorCode::Default)?;

        // TODO check the base asset for this market has already been added
        // TODO check the oracle for this market has already been added

        let market_index = merps_group.num_markets;
        let token_index = merps_group.num_markets + 1;

        let spot_market = load_market_state(spot_market_ai, dex_program_ai.key)?;
        let sm_base_mint = spot_market.coin_mint;
        let sm_quote_mint = spot_market.pc_mint;
        check_eq!(
            sm_base_mint,
            merps_group.tokens[token_index].to_aligned_bytes(),
            MerpsErrorCode::Default
        )?;
        check_eq!(
            sm_quote_mint,
            merps_group.tokens[QUOTE_INDEX].to_aligned_bytes(),
            MerpsErrorCode::Default
        )?;
        // check!(merps_group.oracles[market_index], MerpsErrorCode::Default)?;

        merps_group.spot_markets[market_index] = *spot_market_ai.key;
        merps_group.num_markets += 1;

        Ok(())
    }

    /// Initialize perp market including orderbooks and queues
    //  Requires a contract_size for the asset
    #[allow(unused)]
    fn add_perp_market() -> MerpsResult<()> {
        // TODO
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
            owner_token_account_ai, // write
        ] = accounts;
        let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;
        let mut merps_account =
            MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;
        check_eq!(&merps_account.owner, owner_ai.key, MerpsErrorCode::InvalidOwner)?;

        let token_index = merps_group
            .find_root_bank_index(root_bank_ai.key)
            .ok_or(throw_err!(MerpsErrorCode::InvalidToken))?;

        // TODO does a token pair need to be in basket to deposit? what about USDC deposits?
        // check!(merps_account.in_basket[token_index], MerpsErrorCode::InvalidToken)?;

        let mut node_bank = NodeBank::load_mut_checked(node_bank_ai, program_id)?;

        // Find the node_bank pubkey in root_bank, if not found error
        let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
        check!(root_bank.node_banks.contains(node_bank_ai.key), MerpsErrorCode::Default)?;
        check_eq!(&node_bank.vault, vault_ai.key, MerpsErrorCode::InvalidVault)?;

        // deposit into node bank token vault using invoke_transfer
        check_eq!(token_prog_ai.key, &spl_token::ID, MerpsErrorCode::Default)?;

        invoke_transfer(token_prog_ai, owner_token_account_ai, vault_ai, owner_ai, &[], quantity)?;

        // increment merps account
        let deposit: I80F48 = I80F48::from_num(quantity) / root_bank.deposit_index;
        checked_add_deposit(&mut node_bank, &mut merps_account, token_index, deposit)?;

        Ok(())
    }

    /// Write oracle prices onto MerpsAccount before calling a value-dep instruction (e.g. Withdraw)    
    #[allow(unused)]
    fn cache_prices(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        const NUM_FIXED: usize = 3;
        let (fixed_ais, oracle_ais) = array_refs![accounts, NUM_FIXED; ..;];
        let [
            merps_group_ai,     // read
            merps_cache_ai,   // write
            clock_ai,           // read
        ] = fixed_ais;

        let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;
        let mut merps_cache =
            MerpsCache::load_mut_checked(merps_cache_ai, program_id, &merps_group)?;
        let clock = Clock::from_account_info(clock_ai)?;
        let now_ts = clock.unix_timestamp as u64;
        for oracle_ai in oracle_ais.iter() {
            let i = merps_group.find_oracle_index(oracle_ai.key).ok_or(throw!())?;

            merps_cache.price_cache[i] =
                PriceCache { price: read_oracle(oracle_ai)?, last_update: now_ts };
        }
        Ok(())
    }

    #[allow(unused)]
    fn cache_root_banks(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        const NUM_FIXED: usize = 3;
        let (fixed_ais, root_bank_ais) = array_refs![accounts, NUM_FIXED; ..;];
        let [
            merps_group_ai,     // read
            merps_cache_ai,   // write
            clock_ai,           // read
        ] = fixed_ais;

        let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;
        let mut merps_cache =
            MerpsCache::load_mut_checked(merps_cache_ai, program_id, &merps_group)?;
        let clock = Clock::from_account_info(clock_ai)?;
        let now_ts = clock.unix_timestamp as u64;
        for root_bank_ai in root_bank_ais.iter() {
            let index = merps_group.find_root_bank_index(root_bank_ai.key).unwrap();
            let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
            merps_cache.root_bank_cache[index] = RootBankCache {
                deposit_index: root_bank.deposit_index,
                borrow_index: root_bank.borrow_index,
                last_update: now_ts,
            };
        }
        Ok(())
    }

    #[allow(unused)]
    fn cache_open_orders(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        // TODO
        Ok(())
    }

    #[allow(unused)]
    fn cache_perp_market(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        // TODO
        Ok(())
    }

    #[allow(unused_variables)]
    fn borrow(program_id: &Pubkey, accounts: &[AccountInfo], quantity: u64) -> MerpsResult<()> {
        const NUM_FIXED: usize = 7;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [merps_group_ai, merps_account_ai, owner_ai, merps_cache_ai, root_bank_ai, node_bank_ai, clock_ai] =
            accounts;

        let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;

        let mut merps_account =
            MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;
        check_eq!(&merps_account.owner, owner_ai.key, MerpsErrorCode::InvalidOwner)?;
        check!(owner_ai.is_signer, MerpsErrorCode::Default)?;

        // TODO check node_bank indexes have been updated via the Keeper

        let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
        let mut node_bank = NodeBank::load_mut_checked(node_bank_ai, program_id)?;

        // Make sure the root bank is in the merps group
        let token_index = merps_group
            .find_root_bank_index(root_bank_ai.key)
            .ok_or(throw_err!(MerpsErrorCode::InvalidToken))?;

        // TODO is this correct? skip check if token_index is quote currency
        if token_index > 0 {
            check!(merps_account.in_basket[token_index - 1], MerpsErrorCode::InvalidToken)?;
        }

        // First check all caches to make sure valid
        let clock = Clock::from_account_info(clock_ai)?;
        let now_ts = clock.unix_timestamp as u64;

        // TODO implement caches valid in tests
        let merps_cache = MerpsCache::load_checked(merps_cache_ai, program_id, &merps_group)?;
        if !merps_cache.check_caches_valid(&merps_group, &merps_account, now_ts) {
            // TODO log or write to buffer that this transaction did not complete due to stale cache
            return Ok(());
        }

        let deposit: I80F48 = I80F48::from_num(quantity) / root_bank.deposit_index;
        let borrow: I80F48 = I80F48::from_num(quantity) / root_bank.borrow_index;

        checked_add_deposit(&mut node_bank, &mut merps_account, token_index, deposit)?;
        checked_add_borrow(&mut node_bank, &mut merps_account, token_index, borrow)?;

        // let coll_ratio = merps_account.get_coll_ratio(&merps_group)?;

        // TODO fix coll_ratio check
        // check!(coll_ratio >= ONE_I80F48, MerpsErrorCode::InsufficientFunds)?;
        // check!(node_bank.has_valid_deposits_borrows(&root_bank), MerpsErrorCode::Default)?;

        Ok(())
    }

    /// Withdraw a token from the bank if collateral ratio permits
    fn withdraw(program_id: &Pubkey, accounts: &[AccountInfo], quantity: u64) -> MerpsResult<()> {
        const NUM_FIXED: usize = 11;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            merps_group_ai,     // read
            merps_account_ai,   // write
            owner_ai,           // read
            merps_cache_ai,
            root_bank_ai,       // read
            node_bank_ai,       // write
            vault_ai,           // write
            token_account_ai,   // write
            signer_ai,          // read
            token_prog_ai,      // read
            clock_ai,           // read
        ] = accounts;
        let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;

        let mut merps_account =
            MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;
        check!(&merps_account.owner == owner_ai.key, MerpsErrorCode::InvalidOwner)?;

        let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
        let mut node_bank = NodeBank::load_mut_checked(node_bank_ai, program_id)?;
        let clock = Clock::from_account_info(clock_ai)?;
        let now_ts = clock.unix_timestamp as u64;

        // Make sure the asset is in basket
        let token_index = merps_group
            .find_root_bank_index(root_bank_ai.key)
            .ok_or(throw_err!(MerpsErrorCode::InvalidToken))?;

        // TODO is this correct? skip check if token_index is quote currency
        if token_index > 0 {
            check!(merps_account.in_basket[token_index - 1], MerpsErrorCode::InvalidToken)?;
        }

        // Safety checks
        check_eq!(&node_bank.vault, vault_ai.key, MerpsErrorCode::InvalidVault)?;
        check_eq!(&spl_token::ID, token_prog_ai.key, MerpsErrorCode::InvalidProgramId)?;

        // First check all caches to make sure valid
        let merps_cache = MerpsCache::load_checked(merps_cache_ai, program_id, &merps_group)?;
        if !merps_cache.check_caches_valid(&merps_group, &merps_account, now_ts) {
            // TODO log or write to buffer that this transaction did not complete due to stale cache
            return Ok(());
        }

        // Subtract the amount from merps account
        // TODO borrow first if possible
        checked_sub_deposit(
            &mut node_bank,
            &mut merps_account,
            token_index,
            I80F48::from_num(quantity) / root_bank.deposit_index,
        )?;

        let coll_ratio = merps_account.get_coll_ratio(&merps_group, &merps_cache)?;
        check!(coll_ratio >= ONE_I80F48, MerpsErrorCode::InsufficientFunds)?;

        // invoke_transfer()
        // TODO think about whether this is a security risk. This is basically one signer for all merps
        let signers_seeds = [bytes_of(&merps_group.signer_nonce)];
        invoke_transfer(
            token_prog_ai,
            vault_ai,
            token_account_ai,
            signer_ai,
            &[&signers_seeds],
            quantity,
        )?;

        Ok(())
    }

    fn add_to_basket(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        const NUM_FIXED: usize = 4;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [merps_group_ai, merps_account_ai, owner_ai, spot_market_ai] = accounts;

        let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;

        let mut merps_account =
            MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;
        check_eq!(&merps_account.owner, owner_ai.key, MerpsErrorCode::Default)?;

        let spot_market_index = merps_group
            .find_spot_market_index(spot_market_ai.key)
            .ok_or(throw_err!(MerpsErrorCode::InvalidMarket))?;

        merps_account.in_basket[spot_market_index] = true;

        Ok(())
    }

    /// Same idea as Mango margin
    #[allow(unused)]
    fn place_spot_order() -> MerpsResult<()> {
        // TODO
        unimplemented!()
    }

    #[allow(unused)]
    fn cancel_spot_order() -> MerpsResult<()> {
        // TODO
        unimplemented!()
    }

    #[allow(unused)]
    fn place_perp_order(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        side: Side,
        price: i64,
        quantity: i64,
        client_order_id: u64,
    ) -> MerpsResult<()> {
        const NUM_FIXED: usize = 9;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            merps_group_ai,     // read
            merps_account_ai,   // write
            owner_ai,           // read
            merps_cache_ai,     // read
            perp_market_ai,     // write
            bids_ai,            // write
            asks_ai,            // write
            event_queue_ai,     // write
            clock_ai,           // read
        ] = accounts;
        let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;

        let mut merps_account =
            MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;

        let clock = Clock::from_account_info(clock_ai)?;
        let now_ts = clock.unix_timestamp as u64;

        check!(&merps_account.owner == owner_ai.key, MerpsErrorCode::InvalidOwner)?;
        check!(price > 0, MerpsErrorCode::Default)?;
        check!(quantity > 0, MerpsErrorCode::Default)?;

        let merps_cache = MerpsCache::load_checked(merps_cache_ai, program_id, &merps_group)?;
        if !merps_cache.check_caches_valid(&merps_group, &merps_account, now_ts) {
            return Ok(());
        }

        /*
           How to adjust the funding settled
           FS_t = (FS_t-1 - FE) * C_t-1 / C_t + FE
        */

        /*
           1. First match against the book

           funding settled

           2. Determine if account still above coll ratio
        */

        // put the order onto the book

        Ok(())
    }

    #[allow(unused)]
    fn cancel_perp_order() -> MerpsResult<()> {
        // TODO
        unimplemented!()
    }

    /// Take two MerpsAccount and settle quote currency pnl between them
    #[allow(unused)]
    fn settle() -> MerpsResult<()> {
        // TODO
        unimplemented!()
    }

    /// Liquidate an account similar to Mango
    #[allow(unused)]
    fn liquidate() -> MerpsResult<()> {
        // TODO - still need to figure out how liquidations for perps will work, but
        unimplemented!()
    }

    /// *** Keeper Related Instructions ***

    /// Update the deposit and borrow index on passed in RootBanks
    #[allow(unused)]
    fn update_banks(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        // TODO
        unimplemented!()
    }

    /// similar to serum dex, but also need to do some extra magic with funding
    #[allow(unused)]
    fn consume_event_queue(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        // TODO
        unimplemented!()
    }

    /// Update the `funding_earned` of a `PerpMarket` using the current book price, spot index price
    /// and time since last update
    #[allow(unused)]
    fn update_funding(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        // TODO
        unimplemented!()
    }

    pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> MerpsResult<()> {
        let instruction =
            MerpsInstruction::unpack(data).ok_or(ProgramError::InvalidInstructionData)?;
        match instruction {
            MerpsInstruction::InitMerpsGroup { signer_nonce, valid_interval } => {
                Self::init_merps_group(program_id, accounts, signer_nonce, valid_interval)?;
            }
            MerpsInstruction::InitMerpsAccount => {
                Self::init_merps_account(program_id, accounts)?;
            }
            MerpsInstruction::Deposit { quantity } => {
                msg!("Merps: Deposit");
                Self::deposit(program_id, accounts, quantity)?;
            }
            MerpsInstruction::Withdraw { quantity } => {
                msg!("Merps: Withdraw");
                Self::withdraw(program_id, accounts, quantity)?;
            }
            MerpsInstruction::AddAsset => {
                Self::add_asset(program_id, accounts)?;
            }
            MerpsInstruction::AddSpotMarket => {
                Self::add_spot_market(program_id, accounts)?;
            }
            MerpsInstruction::AddToBasket => {
                Self::add_to_basket(program_id, accounts)?;
            }
            MerpsInstruction::Borrow { quantity } => {
                Self::borrow(program_id, accounts, quantity)?;
            }
        }

        Ok(())
    }
}

fn init_root_bank(
    program_id: &Pubkey,
    merps_group: &MerpsGroup,
    mint_ai: &AccountInfo,
    vault_ai: &AccountInfo,
    root_bank_ai: &AccountInfo,
    node_bank_ai: &AccountInfo,
) -> MerpsResult<RootBank> {
    let vault = Account::unpack(&vault_ai.try_borrow_data()?)?;
    check!(vault.is_initialized(), MerpsErrorCode::Default)?;
    check_eq!(vault.owner, merps_group.signer_key, MerpsErrorCode::Default)?;
    check_eq!(&vault.mint, mint_ai.key, MerpsErrorCode::Default)?;
    check_eq!(vault_ai.owner, &spl_token::id(), MerpsErrorCode::Default)?;

    let mut node_bank = NodeBank::load_mut(&node_bank_ai)?;
    check_eq!(node_bank_ai.owner, program_id, MerpsErrorCode::InvalidOwner)?;

    node_bank.meta_data.data_type = DataType::NodeBank as u8;
    node_bank.meta_data.is_initialized = true;
    node_bank.meta_data.version = 0;
    node_bank.deposits = ZERO_I80F48;
    node_bank.borrows = ZERO_I80F48;
    node_bank.vault = *vault_ai.key;

    let mut root_bank = RootBank::load_mut(&root_bank_ai)?;
    check_eq!(root_bank_ai.owner, program_id, MerpsErrorCode::InvalidOwner)?;

    root_bank.meta_data.data_type = DataType::RootBank as u8;
    root_bank.meta_data.is_initialized = true;
    root_bank.node_banks[0] = *node_bank_ai.key;
    root_bank.num_node_banks = 1;
    root_bank.deposit_index = ONE_I80F48;
    root_bank.borrow_index = ONE_I80F48;

    Ok(*root_bank)
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
        token_prog_acc.clone(), // TODO check if passing in program_id is necessary
        source_acc.clone(),
        dest_acc.clone(),
        authority_acc.clone(),
    ];

    solana_program::program::invoke_signed(&transfer_instruction, &accs, signers_seeds)
}

#[allow(unused)]
fn read_oracle(oracle_ai: &AccountInfo) -> MerpsResult<I80F48> {
    Ok(ZERO_I80F48) // TODO
}

fn checked_add_deposit(
    node_bank: &mut NodeBank,
    merps_account: &mut MerpsAccount,
    token_index: usize,
    quantity: I80F48,
) -> MerpsResult<()> {
    merps_account.checked_add_deposit(token_index, quantity)?;
    node_bank.checked_add_deposit(quantity)
}

fn checked_sub_deposit(
    node_bank: &mut NodeBank,
    merps_account: &mut MerpsAccount,
    token_index: usize,
    quantity: I80F48,
) -> MerpsResult<()> {
    merps_account.checked_sub_deposit(token_index, quantity)?;
    node_bank.checked_sub_deposit(quantity)
}

fn checked_add_borrow(
    node_bank: &mut NodeBank,
    margin_account: &mut MerpsAccount,
    token_index: usize,
    quantity: I80F48,
) -> MerpsResult<()> {
    margin_account.checked_add_borrow(token_index, quantity)?;
    node_bank.checked_add_borrow(quantity)
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
