use std::cmp;
use std::mem::size_of;

use arrayref::{array_ref, array_refs};
use fixed::types::I80F48;
use flux_aggregator::borsh_state::InitBorshState;

use serum_dex::matching::Side as SerumSide;
use serum_dex::state::ToAlignedBytes;

use solana_program::account_info::AccountInfo;
use solana_program::clock::Clock;
use solana_program::entrypoint::ProgramResult;
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::msg;
use solana_program::program_error::ProgramError;
use solana_program::program_pack::{IsInitialized, Pack};
use solana_program::pubkey::Pubkey;
use solana_program::rent::Rent;
use solana_program::sysvar::Sysvar;
use spl_token::state::{Account, Mint};

use crate::error::{check_assert, MerpsError, MerpsErrorCode, MerpsResult, SourceFileId};
use crate::instruction::MerpsInstruction;
use crate::matching::{Book, BookSide, OrderType, Side};
use crate::queue::EventQueue;
use crate::state::{
    load_market_state, DataType, MerpsAccount, MerpsCache, MerpsGroup, MetaData, NodeBank,
    PerpAccount, PerpMarket, PerpMarketInfo, PriceCache, RootBank, RootBankCache, SpotMarketInfo,
    TokenInfo, MAX_PAIRS, ONE_I80F48, QUOTE_INDEX, ZERO_I80F48,
};
use crate::utils::{gen_signer_key, gen_signer_seeds};
use mango_common::Loadable;
use std::convert::identity;

declare_check_assert_macros!(SourceFileId::Processor);

pub struct Processor {}

impl Processor {
    fn init_merps_group(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        signer_nonce: u64,
        valid_interval: u8,
    ) -> ProgramResult {
        const NUM_FIXED: usize = 9;
        let accounts = array_ref![accounts, 0, NUM_FIXED];

        let [
            merps_group_ai,     // write
            signer_ai,          // read
            admin_ai,           // read
            quote_mint_ai,      // read
            quote_vault_ai,     // read
            quote_node_bank_ai, // write
            quote_root_bank_ai, // write
            merps_cache_ai,     // write
            dex_prog_ai         // read
        ] = accounts;
        check_eq!(merps_group_ai.owner, program_id, MerpsErrorCode::InvalidGroupOwner)?;
        let rent = Rent::get()?;
        check!(
            rent.is_exempt(merps_group_ai.lamports(), size_of::<MerpsGroup>()),
            MerpsErrorCode::GroupNotRentExempt
        )?;
        let mut merps_group = MerpsGroup::load_mut(merps_group_ai)?;
        check!(!merps_group.meta_data.is_initialized, MerpsErrorCode::Default)?;

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
            &rent,
        )?;

        let mint = Mint::unpack(&quote_mint_ai.try_borrow_data()?)?;
        merps_group.tokens[QUOTE_INDEX] = TokenInfo {
            mint: *quote_mint_ai.key,
            root_bank: *quote_root_bank_ai.key,
            decimals: mint.decimals,
            padding: [0u8; 7],
        };

        check!(admin_ai.is_signer, MerpsErrorCode::Default)?;
        merps_group.admin = *admin_ai.key;

        merps_group.meta_data = MetaData::new(DataType::MerpsGroup, 0, true);

        // init MerpsCache
        let mut merps_cache = MerpsCache::load_mut(&merps_cache_ai)?;
        check!(!merps_cache.meta_data.is_initialized, MerpsErrorCode::Default)?;
        merps_cache.meta_data = MetaData::new(DataType::MerpsCache, 0, true);
        merps_group.merps_cache = *merps_cache_ai.key;

        // check size
        Ok(())
    }

    /// TODO figure out how to do docs for functions with link to instruction.rs instruction documentation
    /// TODO make the merps account a derived address
    fn init_merps_account(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        const NUM_FIXED: usize = 3;
        let accounts = array_ref![accounts, 0, NUM_FIXED];

        let [
            merps_group_ai,     // read 
            merps_account_ai,   // write
            owner_ai            // read, signer
        ] = accounts;

        let rent = Rent::get()?;
        check!(
            rent.is_exempt(merps_account_ai.lamports(), size_of::<MerpsAccount>()),
            MerpsErrorCode::Default
        )?;
        check!(owner_ai.is_signer, MerpsErrorCode::Default)?;

        let _merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;

        let mut merps_account = MerpsAccount::load_mut(merps_account_ai)?;
        check_eq!(&merps_account_ai.owner, &program_id, MerpsErrorCode::InvalidOwner)?;
        check!(!merps_account.meta_data.is_initialized, MerpsErrorCode::Default)?;

        merps_account.merps_group = *merps_group_ai.key;
        merps_account.owner = *owner_ai.key;
        merps_account.meta_data = MetaData::new(DataType::MerpsAccount, 0, true);

        Ok(())
    }

    /// Add asset and spot market to merps group
    /// Initialize a root bank and add it to the merps group
    /// Requires a price oracle for this asset priced in quote currency
    /// Only allow admin to add to MerpsGroup
    // TODO think about how to remove an asset. Maybe this just can't be done?
    fn add_spot_market(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        market_index: usize,
        maint_asset_weight: I80F48,
        init_asset_weight: I80F48,
    ) -> MerpsResult<()> {
        const NUM_FIXED: usize = 8;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            merps_group_ai, // write
            spot_market_ai, // read
            dex_program_ai, // read
            mint_ai,        // read
            node_bank_ai,   // write
            vault_ai,       // read
            root_bank_ai,   // write
            admin_ai        // read
        ] = accounts;

        let mut merps_group = MerpsGroup::load_mut_checked(merps_group_ai, program_id)?;

        check!(admin_ai.is_signer, MerpsErrorCode::Default)?;
        check_eq!(admin_ai.key, &merps_group.admin, MerpsErrorCode::Default)?;

        check!(market_index < merps_group.num_oracles, MerpsErrorCode::Default)?;

        // Make sure there is an oracle at this index -- probably unnecessary because add_oracle is only place that modifies num_oracles
        check!(merps_group.oracles[market_index] != Pubkey::default(), MerpsErrorCode::Default)?;

        // Make sure spot market at this index not already initialized
        check!(merps_group.spot_markets[market_index].is_empty(), MerpsErrorCode::Default)?;

        // Make sure token at this index not already initialized
        check!(merps_group.tokens[market_index].is_empty(), MerpsErrorCode::Default)?;
        let _root_bank = init_root_bank(
            program_id,
            &merps_group,
            mint_ai,
            vault_ai,
            root_bank_ai,
            node_bank_ai,
            &Rent::get()?,
        )?;

        let mint = Mint::unpack(&mint_ai.try_borrow_data()?)?;
        merps_group.tokens[market_index] = TokenInfo {
            mint: *mint_ai.key,
            root_bank: *root_bank_ai.key,
            decimals: mint.decimals,
            padding: [0u8; 7],
        };

        // check that maint weight is lower than init asset weight
        check!(maint_asset_weight < init_asset_weight, MerpsErrorCode::Default)?;
        check!(maint_asset_weight > ZERO_I80F48, MerpsErrorCode::Default)?;
        check!(maint_asset_weight <= 1, MerpsErrorCode::Default)?;

        merps_group.spot_markets[market_index] = SpotMarketInfo {
            spot_market: *spot_market_ai.key,
            maint_asset_weight,
            init_asset_weight,
            maint_liab_weight: ONE_I80F48 / maint_asset_weight,
            init_liab_weight: ONE_I80F48 / init_asset_weight,
        };

        // TODO needs to be moved into add_oracle
        // let _oracle = flux_aggregator::state::Aggregator::load_initialized(&oracle_ai)?;
        // merps_group.oracles[token_index] = *oracle_ai.key;

        let spot_market = load_market_state(spot_market_ai, dex_program_ai.key)?;

        check_eq!(
            identity(spot_market.coin_mint),
            mint_ai.key.to_aligned_bytes(),
            MerpsErrorCode::Default
        )?;
        check_eq!(
            identity(spot_market.pc_mint),
            merps_group.tokens[QUOTE_INDEX].mint.to_aligned_bytes(),
            MerpsErrorCode::Default
        )?;

        Ok(())
    }

    /// Add an oracle to the MerpsGroup
    /// This must be called first before `add_spot_market` or `add_perp_market`
    /// There will never be a gap in the merps_group.oracles array
    #[allow(unused)]
    fn add_oracle(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        const NUM_FIXED: usize = 3;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            merps_group_ai, // write
            oracle_ai,      // read
            admin_ai        // read
        ] = accounts;

        let mut merps_group = MerpsGroup::load_mut_checked(merps_group_ai, program_id)?;
        check!(admin_ai.is_signer, MerpsErrorCode::Default)?;
        check_eq!(admin_ai.key, &merps_group.admin, MerpsErrorCode::Default)?;

        // TODO allow more oracle types including purely on chain price feeds
        let _oracle = flux_aggregator::state::Aggregator::load_initialized(&oracle_ai)?;
        let oracle_index = merps_group.num_oracles;
        merps_group.oracles[oracle_index] = *oracle_ai.key;
        merps_group.num_oracles += 1;

        Ok(())
    }

    /// Initialize perp market including orderbooks and queues
    //  Requires a contract_size for the asset
    #[allow(unused)]
    fn add_perp_market(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        market_index: usize,
        maint_asset_weight: I80F48,
        init_asset_weight: I80F48,
        base_lot_size: i64,
        quote_lot_size: i64,
    ) -> MerpsResult<()> {
        // TODO
        const NUM_FIXED: usize = 6;
        let accounts = array_ref![accounts, 0, NUM_FIXED];

        let [
            merps_group_ai, // write
            perp_market_ai, // write
            event_queue_ai, // write
            bids_ai,        // write
            asks_ai,        // write

            admin_ai        // read, signer
        ] = accounts;

        let rent = Rent::get()?; // dynamically load rent sysvar

        let mut merps_group = MerpsGroup::load_mut_checked(merps_group_ai, program_id)?;

        check!(admin_ai.is_signer, MerpsErrorCode::Default)?;
        check_eq!(admin_ai.key, &merps_group.admin, MerpsErrorCode::Default)?;

        check!(market_index < merps_group.num_oracles, MerpsErrorCode::Default)?;

        // Make sure there is an oracle at this index -- probably unnecessary because add_oracle is only place that modifies num_oracles
        check!(merps_group.oracles[market_index] != Pubkey::default(), MerpsErrorCode::Default)?;

        // Make sure perp market at this index not already initialized
        check!(merps_group.perp_markets[market_index].is_empty(), MerpsErrorCode::Default)?;

        check!(maint_asset_weight < init_asset_weight, MerpsErrorCode::Default)?;
        check!(maint_asset_weight > ZERO_I80F48, MerpsErrorCode::Default)?;
        merps_group.perp_markets[market_index] = PerpMarketInfo {
            perp_market: *perp_market_ai.key,
            maint_asset_weight,
            init_asset_weight,
            maint_liab_weight: ONE_I80F48 / maint_asset_weight,
            init_liab_weight: ONE_I80F48 / init_asset_weight,
            base_lot_size,
            quote_lot_size,
        };

        // Initialize the Bids
        let _bids = BookSide::load_and_init(bids_ai, program_id, DataType::Bids, &rent)?;

        // Initialize the Asks
        let _asks = BookSide::load_and_init(asks_ai, program_id, DataType::Asks, &rent)?;

        // Initialize the EventQueue
        let _event_queue = EventQueue::load_and_init(event_queue_ai, program_id, &rent)?;

        // Now initialize the PerpMarket itself
        let _perp_market = PerpMarket::load_and_init(
            perp_market_ai,
            program_id,
            merps_group_ai,
            bids_ai,
            asks_ai,
            event_queue_ai,
            &merps_group,
            &rent,
            market_index,
            base_lot_size,
            quote_lot_size,
        )?;
        Ok(())
    }

    /// Deposit instruction
    fn deposit(program_id: &Pubkey, accounts: &[AccountInfo], quantity: u64) -> ProgramResult {
        const NUM_FIXED: usize = 8;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            merps_group_ai,         // read
            merps_account_ai,       // write
            owner_ai,               // read
            root_bank_ai,           // read
            node_bank_ai,           // write
            vault_ai,               // write
            token_prog_ai,          // read
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
        const NUM_FIXED: usize = 2;
        let (fixed_ais, oracle_ais) = array_refs![accounts, NUM_FIXED; ..;];
        let [
            merps_group_ai,     // read
            merps_cache_ai,     // write
        ] = fixed_ais;

        let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;
        let mut merps_cache =
            MerpsCache::load_mut_checked(merps_cache_ai, program_id, &merps_group)?;
        let clock = Clock::get()?;
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
        const NUM_FIXED: usize = 2;
        let (fixed_ais, root_bank_ais) = array_refs![accounts, NUM_FIXED; ..;];
        let [
            merps_group_ai,     // read
            merps_cache_ai,     // write
        ] = fixed_ais;

        let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;
        let mut merps_cache =
            MerpsCache::load_mut_checked(merps_cache_ai, program_id, &merps_group)?;
        let clock = Clock::get()?;
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
    fn cache_perp_market(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        // TODO
        Ok(())
    }

    #[allow(unused_variables)]
    fn borrow(program_id: &Pubkey, accounts: &[AccountInfo], quantity: u64) -> MerpsResult<()> {
        // TODO don't allow borrow of infinite amount of quote currency
        const NUM_FIXED: usize = 6;
        let accounts = array_ref![accounts, 0, NUM_FIXED + MAX_PAIRS];
        let (fixed_accs, open_orders_ais) = array_refs![accounts, NUM_FIXED, MAX_PAIRS];
        let [
            merps_group_ai,     // read 
            merps_account_ai,   // write
            owner_ai,           // read
            merps_cache_ai,     // read 
            root_bank_ai,       // read 
            node_bank_ai,       // write  
        ] = fixed_accs;

        let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;

        let mut merps_account =
            MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;
        check_eq!(&merps_account.owner, owner_ai.key, MerpsErrorCode::InvalidOwner)?;
        check!(owner_ai.is_signer, MerpsErrorCode::Default)?;

        let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
        let mut node_bank = NodeBank::load_mut_checked(node_bank_ai, program_id)?;

        // Make sure the root bank is in the merps group
        let token_index = merps_group
            .find_root_bank_index(root_bank_ai.key)
            .ok_or(throw_err!(MerpsErrorCode::InvalidToken))?;

        // TODO is this correct? skip check if token_index is quote currency
        if token_index != QUOTE_INDEX {
            check!(merps_account.in_basket[token_index], MerpsErrorCode::InvalidToken)?;
        }

        // First check all caches to make sure valid
        let clock = Clock::get()?;
        let now_ts = clock.unix_timestamp as u64;

        let valid_interval = merps_group.valid_interval as u64;
        check!(now_ts > root_bank.last_updated + valid_interval, MerpsErrorCode::Default)?;

        let merps_cache = MerpsCache::load_checked(merps_cache_ai, program_id, &merps_group)?;
        check!(
            merps_cache.check_caches_valid(&merps_group, &merps_account, now_ts),
            MerpsErrorCode::InvalidCache
        )?;

        let deposit: I80F48 = I80F48::from_num(quantity) / root_bank.deposit_index;
        let borrow: I80F48 = I80F48::from_num(quantity) / root_bank.borrow_index;

        checked_add_deposit(&mut node_bank, &mut merps_account, token_index, deposit)?;
        checked_add_borrow(&mut node_bank, &mut merps_account, token_index, borrow)?;

        let coll_ratio =
            merps_account.get_coll_ratio(&merps_group, &merps_cache, open_orders_ais)?;

        // TODO fix coll_ratio checks
        check!(coll_ratio >= ONE_I80F48, MerpsErrorCode::InsufficientFunds)?;
        check!(node_bank.has_valid_deposits_borrows(&root_bank), MerpsErrorCode::Default)?;

        Ok(())
    }

    /// Withdraw a token from the bank if collateral ratio permits
    fn withdraw(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        quantity: u64,
        allow_borrow: bool, // TODO only borrow if true
    ) -> MerpsResult<()> {
        const NUM_FIXED: usize = 10;
        let accounts = array_ref![accounts, 0, NUM_FIXED + MAX_PAIRS];
        let (fixed_accs, open_orders_ais) = array_refs![accounts, NUM_FIXED, MAX_PAIRS];
        let [
            merps_group_ai,     // read
            merps_account_ai,   // write
            owner_ai,           // read
            merps_cache_ai,     // read
            root_bank_ai,       // read
            node_bank_ai,       // write
            vault_ai,           // write
            token_account_ai,   // write
            signer_ai,          // read
            token_prog_ai,      // read
        ] = fixed_accs;
        let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;

        let mut merps_account =
            MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;
        check!(&merps_account.owner == owner_ai.key, MerpsErrorCode::InvalidOwner)?;

        let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
        let mut node_bank = NodeBank::load_mut_checked(node_bank_ai, program_id)?;
        let clock = Clock::get()?;
        let now_ts = clock.unix_timestamp as u64;

        let token_index = merps_group
            .find_root_bank_index(root_bank_ai.key)
            .ok_or(throw_err!(MerpsErrorCode::InvalidToken))?;

        // Make sure the asset is in basket
        // TODO is this necessary? skip check if token_index is quote currency
        if token_index != QUOTE_INDEX {
            check!(merps_account.in_basket[token_index], MerpsErrorCode::InvalidToken)?;
        }

        // Safety checks
        check_eq!(&node_bank.vault, vault_ai.key, MerpsErrorCode::InvalidVault)?;
        check_eq!(&spl_token::ID, token_prog_ai.key, MerpsErrorCode::InvalidProgramId)?;

        // First check all caches to make sure valid
        let merps_cache = MerpsCache::load_checked(merps_cache_ai, program_id, &merps_group)?;
        check!(
            merps_cache.check_caches_valid(&merps_group, &merps_account, now_ts),
            MerpsErrorCode::InvalidCache
        )?;
        let valid_interval = merps_group.valid_interval as u64;
        check!(now_ts > root_bank.last_updated + valid_interval, MerpsErrorCode::Default)?;

        // Borrow if withdrawing more than deposits
        let native_deposit = merps_account.get_native_deposit(&root_bank, token_index);
        let rem_to_borrow = quantity - native_deposit;
        if allow_borrow && rem_to_borrow > 0 {
            let avail_deposit = merps_account.deposits[token_index];
            checked_sub_deposit(&mut node_bank, &mut merps_account, token_index, avail_deposit)?;
            checked_add_borrow(
                &mut node_bank,
                &mut merps_account,
                token_index,
                I80F48::from_num(rem_to_borrow),
            )?;
        } else {
            checked_sub_deposit(
                &mut node_bank,
                &mut merps_account,
                token_index,
                I80F48::from_num(quantity) / root_bank.deposit_index,
            )?;
        }

        let coll_ratio =
            merps_account.get_coll_ratio(&merps_group, &merps_cache, open_orders_ais)?;
        check!(coll_ratio >= ONE_I80F48, MerpsErrorCode::InsufficientFunds)?;

        // invoke_transfer()
        // TODO think about whether this is a security risk. This is basically one signer for all merps
        // let signers_seeds = [bytes_of(&merps_group.signer_nonce)];
        let signers_seeds = gen_signer_seeds(&merps_group.signer_nonce, merps_group_ai.key);

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
        let [
            merps_group_ai,     // read
            merps_account_ai,   // write
            owner_ai,           // read
            spot_market_ai      // read
        ] = accounts;

        let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;

        let mut merps_account =
            MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;
        check_eq!(&merps_account.owner, owner_ai.key, MerpsErrorCode::Default)?;
        // TODO make it so you can add_to_basket when adding a perp market without corresponding spot market -- just ask user to send in market_index
        let spot_market_index = merps_group
            .find_spot_market_index(spot_market_ai.key)
            .ok_or(throw_err!(MerpsErrorCode::InvalidMarket))?;

        merps_account.in_basket[spot_market_index] = true;

        Ok(())
    }

    #[allow(unused)]
    fn place_spot_order(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        order: serum_dex::instruction::NewOrderInstructionV3,
    ) -> MerpsResult<()> {
        const NUM_FIXED: usize = 22;
        let accounts = array_ref![accounts, 0, NUM_FIXED + MAX_PAIRS];

        let (fixed_accs, open_orders_ais) = array_refs![accounts, NUM_FIXED, MAX_PAIRS];
        let [
            merps_group_ai,         // read
            merps_account_ai,       // write
            owner_ai,               // read
            merps_cache_ai,         // read
            dex_program_ai,         // read
            spot_market_ai,         // write
            bids_ai,                // write
            asks_ai,                // write
            dex_request_queue_ai,   // write
            dex_event_queue_ai,     // write
            dex_base_ai,            // write
            dex_quote_ai,           // write
            base_root_bank_ai,      // read
            base_node_bank_ai,      // write
            quote_root_bank_ai,     // write
            quote_node_bank_ai,     // write
            quote_vault_ai,         // write
            base_vault_ai,          // write
            token_program_ai,       // read
            signer_ai,              // read
            rent_ai,                // read
            dex_signer_ai,          // read
        ] = fixed_accs;

        let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;
        let mut merps_account =
            MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;

        let clock = Clock::get()?;
        let now_ts = clock.unix_timestamp as u64;

        check!(&merps_account.owner == owner_ai.key, MerpsErrorCode::InvalidOwner)?;
        check!(owner_ai.is_signer, MerpsErrorCode::InvalidSignerKey)?;

        let mut base_root_bank = RootBank::load_mut_checked(base_root_bank_ai, program_id)?;
        let mut base_node_bank = NodeBank::load_mut_checked(base_node_bank_ai, program_id)?;

        let mut quote_root_bank = RootBank::load_mut_checked(quote_root_bank_ai, program_id)?;
        let mut quote_node_bank = NodeBank::load_mut_checked(quote_node_bank_ai, program_id)?;

        // First check all caches to make sure valid
        let merps_cache = MerpsCache::load_checked(merps_cache_ai, program_id, &merps_group)?;
        check!(
            merps_cache.check_caches_valid(&merps_group, &merps_account, now_ts),
            MerpsErrorCode::Default
        )?;

        let coll_ratio =
            merps_account.get_coll_ratio(&merps_group, &merps_cache, open_orders_ais)?;
        let reduce_only = coll_ratio < ONE_I80F48;

        // Make sure the root bank is in the merps group
        let _token_index = merps_group
            .find_root_bank_index(base_root_bank_ai.key)
            .ok_or(throw_err!(MerpsErrorCode::InvalidToken))?;

        // Check that root banks have been updated by Keeper
        let valid_interval = merps_group.valid_interval as u64;
        check!(now_ts > base_root_bank.last_updated + valid_interval, MerpsErrorCode::Default);
        check!(now_ts > quote_root_bank.last_updated + valid_interval, MerpsErrorCode::Default);

        let spot_market_index = merps_group
            .find_spot_market_index(spot_market_ai.key)
            .ok_or(throw_err!(MerpsErrorCode::InvalidMarket))?;

        let side = order.side;
        // TODO maybe merge this with match on banks below
        let (in_token_i, out_token_i, vault_ai) = match side {
            SerumSide::Bid => (spot_market_index, QUOTE_INDEX, quote_vault_ai),
            SerumSide::Ask => (QUOTE_INDEX, spot_market_index, base_vault_ai),
        };

        check_eq!(&base_node_bank.vault, base_vault_ai.key, MerpsErrorCode::Default)?;
        check_eq!(&quote_node_bank.vault, quote_vault_ai.key, MerpsErrorCode::Default)?;
        check_eq!(token_program_ai.key, &spl_token::id(), MerpsErrorCode::Default)?;
        check_eq!(dex_program_ai.key, &merps_group.dex_program_id, MerpsErrorCode::Default)?;

        // this is to keep track of the amount of funds transferred
        let (pre_base, pre_quote) = {
            (
                Account::unpack(&base_vault_ai.try_borrow_data()?)?.amount,
                Account::unpack(&quote_vault_ai.try_borrow_data()?)?.amount,
            )
        };

        let data = serum_dex::instruction::MarketInstruction::NewOrderV3(order).pack();
        let instruction = Instruction {
            program_id: *dex_program_ai.key,
            data,
            accounts: vec![
                AccountMeta::new(*spot_market_ai.key, false),
                AccountMeta::new(*open_orders_ais[spot_market_index].key, false),
                AccountMeta::new(*dex_request_queue_ai.key, false),
                AccountMeta::new(*dex_event_queue_ai.key, false),
                AccountMeta::new(*bids_ai.key, false),
                AccountMeta::new(*asks_ai.key, false),
                AccountMeta::new(*vault_ai.key, false),
                AccountMeta::new_readonly(*signer_ai.key, true),
                AccountMeta::new(*dex_base_ai.key, false),
                AccountMeta::new(*dex_quote_ai.key, false),
                AccountMeta::new_readonly(*token_program_ai.key, false),
                AccountMeta::new_readonly(*rent_ai.key, false),
            ],
        };
        let account_infos = [
            dex_program_ai.clone(), // Have to add account of the program id
            spot_market_ai.clone(),
            open_orders_ais[spot_market_index].clone(),
            dex_request_queue_ai.clone(),
            dex_event_queue_ai.clone(),
            bids_ai.clone(),
            asks_ai.clone(),
            vault_ai.clone(),
            signer_ai.clone(),
            dex_base_ai.clone(),
            dex_quote_ai.clone(),
            token_program_ai.clone(),
            rent_ai.clone(),
        ];

        let signer_seeds = gen_signer_seeds(&merps_group.signer_nonce, merps_group_ai.key);
        solana_program::program::invoke_signed(&instruction, &account_infos, &[&signer_seeds])?;

        // Settle funds for this market
        invoke_settle_funds(
            dex_program_ai,
            spot_market_ai,
            &open_orders_ais[spot_market_index],
            signer_ai,
            dex_base_ai,
            dex_quote_ai,
            base_vault_ai,
            quote_vault_ai,
            dex_signer_ai,
            token_program_ai,
            &[&signer_seeds],
        )?;

        let (post_base, post_quote) = {
            (
                Account::unpack(&base_vault_ai.try_borrow_data()?)?.amount,
                Account::unpack(&quote_vault_ai.try_borrow_data()?)?.amount,
            )
        };

        let (pre_in, pre_out, post_in, post_out) = match side {
            SerumSide::Bid => (pre_base, pre_quote, post_base, post_quote),
            SerumSide::Ask => (pre_quote, pre_base, post_quote, post_base),
        };

        let (mut out_root_bank, mut out_node_bank, mut in_root_bank, mut in_node_bank) = match side
        {
            SerumSide::Bid => (quote_root_bank, quote_node_bank, base_root_bank, base_node_bank),
            SerumSide::Ask => (base_root_bank, base_node_bank, quote_root_bank, quote_node_bank),
        };

        // if out token was net negative, then you may need to borrow more
        if post_out < pre_out {
            let total_out = pre_out.checked_sub(post_out).unwrap();
            let native_deposit = merps_account.get_native_deposit(&out_root_bank, out_token_i);
            if native_deposit < total_out {
                // need to borrow
                let avail_deposit = merps_account.deposits[out_token_i];
                checked_sub_deposit(
                    &mut out_node_bank,
                    &mut merps_account,
                    out_token_i,
                    avail_deposit,
                )?;
                let rem_spend = I80F48::from_num(total_out - native_deposit);

                check!(!reduce_only, MerpsErrorCode::Default)?; // Cannot borrow more in reduce only mode
                checked_add_borrow(
                    &mut out_node_bank,
                    &mut merps_account,
                    out_token_i,
                    rem_spend / out_root_bank.borrow_index,
                )?;
            } else {
                // just spend user deposits
                let merps_spent = I80F48::from_num(total_out) / out_root_bank.deposit_index;
                checked_sub_deposit(
                    &mut out_node_bank,
                    &mut merps_account,
                    out_token_i,
                    merps_spent,
                )?;
            }
        } else {
            // Add out token deposit
            let deposit = I80F48::from_num(post_out.checked_sub(pre_out).unwrap())
                / out_root_bank.deposit_index;
            checked_add_deposit(&mut out_node_bank, &mut merps_account, out_token_i, deposit)?;
        }

        let total_in =
            I80F48::from_num(post_in.checked_sub(pre_in).unwrap()) / in_root_bank.deposit_index;
        checked_add_deposit(&mut in_node_bank, &mut merps_account, in_token_i, total_in)?;

        // Settle borrow
        // TODO only do ops on tokens that have borrows and deposits
        settle_borrow_full_unchecked(
            &mut out_root_bank,
            &mut out_node_bank,
            &mut merps_account,
            out_token_i,
        )?;
        settle_borrow_full_unchecked(
            &mut in_root_bank,
            &mut in_node_bank,
            &mut merps_account,
            in_token_i,
        )?;

        let coll_ratio =
            merps_account.get_coll_ratio(&merps_group, &merps_cache, open_orders_ais)?;
        check!(reduce_only || coll_ratio >= ONE_I80F48, MerpsErrorCode::InsufficientFunds)?;
        check!(out_node_bank.has_valid_deposits_borrows(&out_root_bank), MerpsErrorCode::Default)?;

        Ok(())
    }

    #[allow(unused)]
    fn cancel_spot_order(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        data: Vec<u8>,
    ) -> MerpsResult<()> {
        const NUM_FIXED: usize = 10;
        let accounts = array_ref![accounts, 0, NUM_FIXED];

        let [
            merps_group_ai,
            owner_ai,  // signer
            merps_account_ai,
            dex_prog_ai,
            spot_market_ai,
            bids_ai,
            asks_ai,
            open_orders_ai,
            signer_ai,
            dex_event_queue_ai,
        ] = accounts;

        let mut merps_group = MerpsGroup::load_mut_checked(merps_group_ai, program_id)?;
        let merps_account =
            MerpsAccount::load_checked(merps_account_ai, program_id, merps_group_ai.key)?;
        let clock = Clock::get()?;

        check_eq!(dex_prog_ai.key, &merps_group.dex_program_id, MerpsErrorCode::Default)?;
        check!(owner_ai.is_signer, MerpsErrorCode::Default)?;
        check_eq!(&merps_account.owner, owner_ai.key, MerpsErrorCode::Default)?;

        let market_i = merps_group.find_spot_market_index(spot_market_ai.key).unwrap();
        check_eq!(
            &merps_account.spot_open_orders[market_i],
            open_orders_ai.key,
            MerpsErrorCode::Default
        )?;

        let signer_seeds = gen_signer_seeds(&merps_group.signer_nonce, merps_group_ai.key);
        invoke_cancel_order(
            dex_prog_ai,
            spot_market_ai,
            bids_ai,
            asks_ai,
            open_orders_ai,
            signer_ai,
            dex_event_queue_ai,
            data,
            &[&signer_seeds],
        )?;
        Ok(())
    }

    #[allow(unused)]
    fn place_perp_order(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        side: Side,
        price: i64,
        quantity: i64,
        client_order_id: u64,
        order_type: OrderType,
    ) -> MerpsResult<()> {
        const NUM_FIXED: usize = 8;
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
        ] = accounts;
        let merps_group = MerpsGroup::load_checked(merps_group_ai, program_id)?;

        let mut merps_account =
            MerpsAccount::load_mut_checked(merps_account_ai, program_id, merps_group_ai.key)?;

        let clock = Clock::get()?;
        let now_ts = clock.unix_timestamp as u64;

        check!(&merps_account.owner == owner_ai.key, MerpsErrorCode::InvalidOwner)?;
        // TODO could also make class PosI64 but it gets ugly when doing computations. Maybe have to do this with a large enough dev team
        check!(price > 0, MerpsErrorCode::Default)?;
        check!(quantity > 0, MerpsErrorCode::Default)?;

        let merps_cache = MerpsCache::load_checked(merps_cache_ai, program_id, &merps_group)?;

        check!(
            merps_cache.check_caches_valid(&merps_group, &merps_account, now_ts),
            MerpsErrorCode::Default
        )?;

        let mut perp_market =
            PerpMarket::load_mut_checked(perp_market_ai, program_id, merps_group_ai.key)?;

        let mut book = Book::load_checked(program_id, bids_ai, asks_ai, &perp_market)?;
        let mut event_queue = EventQueue::load_mut_checked(event_queue_ai, program_id)?;
        let market_index = merps_group.find_perp_market_index(perp_market_ai.key).unwrap();
        book.new_order(
            &mut event_queue,
            &mut perp_market,
            &mut merps_account,
            merps_account_ai.key,
            market_index,
            side,
            price,
            quantity,
            order_type,
            client_order_id,
        )?;

        let coll_ratio = merps_account.get_coll_ratio(&merps_group, &merps_cache, &[])?;
        check!(coll_ratio >= ONE_I80F48, MerpsErrorCode::InsufficientFunds)?;

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
        // TODO - take two accounts
        unimplemented!()
    }

    /// Liquidate an account similar to Mango
    #[allow(unused)]
    fn liquidate() -> MerpsResult<()> {
        // TODO - still need to figure out how liquidations for perps will work, but
        // liqor passes in his own account and the liqee merps account
        // position is transfered to the liqor at favorable rate
        unimplemented!()
    }

    /// *** Keeper Related Instructions ***

    /// Update the deposit and borrow index on passed in RootBanks
    #[allow(unused)]
    fn update_banks(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        // TODO - just copy the interest functions from Mango v1 and v2
        unimplemented!()
    }

    /// similar to serum dex, but also need to do some extra magic with funding
    #[allow(unused)]
    fn consume_event_queue(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        // TODO - similar to serum dex event queue
        // TODO - @Daffy needs to determine how the maker's merps account is updated and
        unimplemented!()
    }

    /// Update the `funding_earned` of a `PerpMarket` using the current book price, spot index price
    /// and time since last update
    #[allow(unused)]
    fn update_funding(program_id: &Pubkey, accounts: &[AccountInfo]) -> MerpsResult<()> {
        // TODO - unpack accounts and call perp_market.update_funding()
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
            MerpsInstruction::Withdraw { quantity, allow_borrow } => {
                msg!("Merps: Withdraw");
                Self::withdraw(program_id, accounts, quantity, allow_borrow)?;
            }
            MerpsInstruction::AddSpotMarket {
                market_index,
                maint_asset_weight,
                init_asset_weight,
            } => {
                Self::add_spot_market(
                    program_id,
                    accounts,
                    market_index,
                    maint_asset_weight,
                    init_asset_weight,
                )?;
            }
            MerpsInstruction::AddToBasket => {
                Self::add_to_basket(program_id, accounts)?;
            }
            MerpsInstruction::Borrow { quantity } => {
                Self::borrow(program_id, accounts, quantity)?;
            }
            MerpsInstruction::CachePrices => {
                Self::cache_prices(program_id, accounts)?;
            }
            MerpsInstruction::CacheRootBanks => {
                Self::cache_root_banks(program_id, accounts)?;
            }
            MerpsInstruction::PlaceSpotOrder { order } => {
                Self::place_spot_order(program_id, accounts, order)?;
            }
            MerpsInstruction::AddOracle => Self::add_oracle(program_id, accounts)?,
        }

        Ok(())
    }
}

/// Take two MerpsAccounts and settle unrealized trading pnl and funding pnl between them
#[allow(unused)]
fn settle_trading_pnl(
    a: &mut PerpAccount,
    b: &mut PerpAccount,
    price: I80F48, // TODO price usually comes in I80F48
    contract_size: i64,
) -> MerpsResult<()> {
    /*
    TODO consider rule: Can only settle if both accounts remain above bankruptcy
     */

    let new_quote_pos_a = I80F48::from_num(-a.base_position * contract_size) * price;
    let new_quote_pos_b = I80F48::from_num(-b.base_position * contract_size) * price;
    let a_pnl = a.quote_position - new_quote_pos_a;
    let b_pnl = b.quote_position - new_quote_pos_b;

    // pnl must be opposite signs for there to be a settlement

    if a_pnl > 0 && b_pnl < 0 {
        let settlement = a_pnl.min(b_pnl.abs());
    } else if a_pnl < 0 && b_pnl > 0 {
    } else {
        return Err(throw!());
    }
    Ok(())
}

fn init_root_bank(
    program_id: &Pubkey,
    merps_group: &MerpsGroup,
    mint_ai: &AccountInfo,
    vault_ai: &AccountInfo,
    root_bank_ai: &AccountInfo,
    node_bank_ai: &AccountInfo,
    rent: &Rent,
) -> MerpsResult<RootBank> {
    let vault = Account::unpack(&vault_ai.try_borrow_data()?)?;
    check!(vault.is_initialized(), MerpsErrorCode::Default)?;
    check_eq!(vault.owner, merps_group.signer_key, MerpsErrorCode::Default)?;
    check_eq!(&vault.mint, mint_ai.key, MerpsErrorCode::Default)?;
    check_eq!(vault_ai.owner, &spl_token::id(), MerpsErrorCode::Default)?;

    let mut _node_bank = NodeBank::load_and_init(&node_bank_ai, &program_id, &vault_ai, rent)?;
    let root_bank = RootBank::load_and_init(&root_bank_ai, &program_id, node_bank_ai, rent)?;

    Ok(*root_bank)
}

fn invoke_settle_funds<'a>(
    dex_prog_acc: &AccountInfo<'a>,
    spot_market_acc: &AccountInfo<'a>,
    open_orders_acc: &AccountInfo<'a>,
    signer_acc: &AccountInfo<'a>,
    dex_base_acc: &AccountInfo<'a>,
    dex_quote_acc: &AccountInfo<'a>,
    base_vault_acc: &AccountInfo<'a>,
    quote_vault_acc: &AccountInfo<'a>,
    dex_signer_acc: &AccountInfo<'a>,
    token_prog_acc: &AccountInfo<'a>,
    signers_seeds: &[&[&[u8]]],
) -> ProgramResult {
    let data = serum_dex::instruction::MarketInstruction::SettleFunds.pack();
    let instruction = Instruction {
        program_id: *dex_prog_acc.key,
        data,
        accounts: vec![
            AccountMeta::new(*spot_market_acc.key, false),
            AccountMeta::new(*open_orders_acc.key, false),
            AccountMeta::new_readonly(*signer_acc.key, true),
            AccountMeta::new(*dex_base_acc.key, false),
            AccountMeta::new(*dex_quote_acc.key, false),
            AccountMeta::new(*base_vault_acc.key, false),
            AccountMeta::new(*quote_vault_acc.key, false),
            AccountMeta::new_readonly(*dex_signer_acc.key, false),
            AccountMeta::new_readonly(*token_prog_acc.key, false),
        ],
    };

    let account_infos = [
        dex_prog_acc.clone(),
        spot_market_acc.clone(),
        open_orders_acc.clone(),
        signer_acc.clone(),
        dex_base_acc.clone(),
        dex_quote_acc.clone(),
        base_vault_acc.clone(),
        quote_vault_acc.clone(),
        dex_signer_acc.clone(),
        token_prog_acc.clone(),
    ];
    solana_program::program::invoke_signed(&instruction, &account_infos, signers_seeds)
}

fn invoke_cancel_order<'a>(
    dex_prog_ai: &AccountInfo<'a>,
    spot_market_ai: &AccountInfo<'a>,
    bids_ai: &AccountInfo<'a>,
    asks_ai: &AccountInfo<'a>,
    open_orders_ai: &AccountInfo<'a>,
    signer_ai: &AccountInfo<'a>,
    dex_event_queue_ai: &AccountInfo<'a>,
    data: Vec<u8>,
    signers_seeds: &[&[&[u8]]],
) -> ProgramResult {
    let instruction = Instruction {
        program_id: *dex_prog_ai.key,
        data,
        accounts: vec![
            AccountMeta::new(*spot_market_ai.key, false),
            AccountMeta::new(*bids_ai.key, false),
            AccountMeta::new(*asks_ai.key, false),
            AccountMeta::new(*open_orders_ai.key, false),
            AccountMeta::new_readonly(*signer_ai.key, true),
            AccountMeta::new(*dex_event_queue_ai.key, false),
        ],
    };

    let account_infos = [
        dex_prog_ai.clone(),
        spot_market_ai.clone(),
        bids_ai.clone(),
        asks_ai.clone(),
        open_orders_ai.clone(),
        signer_ai.clone(),
        dex_event_queue_ai.clone(),
    ];
    solana_program::program::invoke_signed(&instruction, &account_infos, signers_seeds)
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
    merps_account: &mut MerpsAccount,
    token_index: usize,
    quantity: I80F48,
) -> MerpsResult<()> {
    merps_account.checked_add_borrow(token_index, quantity)?;
    node_bank.checked_add_borrow(quantity)
}

fn checked_sub_borrow(
    node_bank: &mut NodeBank,
    merps_account: &mut MerpsAccount,
    token_index: usize,
    quantity: I80F48,
) -> MerpsResult<()> {
    merps_account.checked_sub_borrow(token_index, quantity)?;
    node_bank.checked_sub_borrow(quantity)
}

fn settle_borrow_full_unchecked(
    root_bank: &mut RootBank,
    node_bank: &mut NodeBank,
    merps_account: &mut MerpsAccount,
    token_index: usize,
) -> MerpsResult<()> {
    let native_borrow = merps_account.get_native_borrow(root_bank, token_index);
    let native_deposit = merps_account.get_native_deposit(root_bank, token_index);

    let quantity = cmp::min(native_borrow, native_deposit);

    let borr_settle = I80F48::from_num(quantity) / root_bank.borrow_index;
    let dep_settle = I80F48::from_num(quantity) / root_bank.deposit_index;

    checked_sub_deposit(node_bank, merps_account, token_index, dep_settle)?;
    checked_sub_borrow(node_bank, merps_account, token_index, borr_settle)?;

    // No need to check collateralization ratio or deposits/borrows validity

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
