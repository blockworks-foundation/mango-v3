use std::cell::RefMut;
use std::cmp::min;
use std::convert::{identity, TryFrom};
use std::mem::size_of;
use std::vec;

use arrayref::{array_ref, array_refs};
use bytemuck::{cast, cast_ref};
use fixed::types::I80F48;
use mango_common::Loadable;
use serum_dex::instruction::NewOrderInstructionV3;
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

use crate::error::{check_assert, MangoError, MangoErrorCode, MangoResult, SourceFileId};
use crate::ids::msrm_token;
use crate::ids::srm_token;
use crate::instruction::MangoInstruction;
use crate::matching::{Book, BookSide, OrderType, Side};
use crate::oracle::{determine_oracle_type, OracleType, Price, StubOracle};
use crate::queue::{EventQueue, EventType, FillEvent, LiquidateEvent, OutEvent};
use crate::state::{
    load_asks_mut, load_bids_mut, load_market_state, load_open_orders, AssetType, DataType,
    HealthCache, HealthType, MangoAccount, MangoCache, MangoGroup, MetaData, NodeBank, PerpMarket,
    PerpMarketCache, PerpMarketInfo, PriceCache, RootBank, RootBankCache, SpotMarketInfo,
    TokenInfo, UserActiveAssets, FREE_ORDER_SLOT, INFO_LEN, MAX_NODE_BANKS, MAX_PAIRS,
    MAX_PERP_OPEN_ORDERS, ONE_I80F48, QUOTE_INDEX, ZERO_I80F48,
};
use crate::utils::{gen_signer_key, gen_signer_seeds};
use switchboard_program::FastRoundResultAccountData;

declare_check_assert_macros!(SourceFileId::Processor);

pub struct Processor {}

impl Processor {
    #[inline(never)]
    fn init_mango_group(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        signer_nonce: u64,
        valid_interval: u64,
        quote_optimal_util: I80F48,
        quote_optimal_rate: I80F48,
        quote_max_rate: I80F48,
    ) -> MangoResult<()> {
        const NUM_FIXED: usize = 11;
        let accounts = array_ref![accounts, 0, NUM_FIXED];

        let [
            mango_group_ai,     // write
            signer_ai,          // read
            admin_ai,           // read
            quote_mint_ai,      // read
            quote_vault_ai,     // read
            quote_node_bank_ai, // write
            quote_root_bank_ai, // write
            dao_vault_ai,       // read
            msrm_vault_ai,      // read
            mango_cache_ai,     // write
            dex_prog_ai         // read
        ] = accounts;
        check_eq!(mango_group_ai.owner, program_id, MangoErrorCode::InvalidGroupOwner)?;
        let rent = Rent::get()?;
        check!(
            rent.is_exempt(mango_group_ai.lamports(), size_of::<MangoGroup>()),
            MangoErrorCode::GroupNotRentExempt
        )?;
        let mut mango_group = MangoGroup::load_mut(mango_group_ai)?;
        check!(!mango_group.meta_data.is_initialized, MangoErrorCode::Default)?;

        check!(
            gen_signer_key(signer_nonce, mango_group_ai.key, program_id)? == *signer_ai.key,
            MangoErrorCode::InvalidSignerKey
        )?;
        mango_group.signer_nonce = signer_nonce;
        mango_group.signer_key = *signer_ai.key;
        mango_group.valid_interval = valid_interval;
        mango_group.dex_program_id = *dex_prog_ai.key;

        // TODO OPT make PDA
        let dao_vault = Account::unpack(&dao_vault_ai.try_borrow_data()?)?;
        check!(dao_vault.is_initialized(), MangoErrorCode::Default)?;
        check_eq!(dao_vault.owner, mango_group.signer_key, MangoErrorCode::InvalidVault)?;
        check_eq!(&dao_vault.mint, quote_mint_ai.key, MangoErrorCode::InvalidVault)?;
        check_eq!(dao_vault_ai.owner, &spl_token::ID, MangoErrorCode::InvalidVault)?;
        mango_group.dao_vault = *dao_vault_ai.key;

        // TODO OPT make this a PDA
        if msrm_vault_ai.key != &Pubkey::default() {
            let msrm_vault = Account::unpack(&msrm_vault_ai.try_borrow_data()?)?;
            check!(msrm_vault.is_initialized(), MangoErrorCode::InvalidVault)?;
            check_eq!(msrm_vault.owner, mango_group.signer_key, MangoErrorCode::InvalidVault)?;
            check_eq!(&msrm_vault.mint, &msrm_token::ID, MangoErrorCode::InvalidVault)?;
            check_eq!(msrm_vault_ai.owner, &spl_token::ID, MangoErrorCode::InvalidVault)?;
            mango_group.msrm_vault = *msrm_vault_ai.key;
        }

        let _root_bank = init_root_bank(
            program_id,
            &mango_group,
            quote_mint_ai,
            quote_vault_ai,
            quote_root_bank_ai,
            quote_node_bank_ai,
            &rent,
            quote_optimal_util,
            quote_optimal_rate,
            quote_max_rate,
        )?;
        let mint = Mint::unpack(&quote_mint_ai.try_borrow_data()?)?;
        mango_group.tokens[QUOTE_INDEX] = TokenInfo {
            mint: *quote_mint_ai.key,
            root_bank: *quote_root_bank_ai.key,
            decimals: mint.decimals,
            padding: [0u8; 7],
        };

        check!(admin_ai.is_signer, MangoErrorCode::Default)?;
        mango_group.admin = *admin_ai.key;

        mango_group.meta_data = MetaData::new(DataType::MangoGroup, 0, true);

        // init MangoCache
        let mut mango_cache = MangoCache::load_mut(&mango_cache_ai)?;
        check!(!mango_cache.meta_data.is_initialized, MangoErrorCode::Default)?;
        mango_cache.meta_data = MetaData::new(DataType::MangoCache, 0, true);
        mango_group.mango_cache = *mango_cache_ai.key;

        // check size
        Ok(())
    }

    #[allow(unused)]
    fn remove_spot_market(program_id: &Pubkey, accounts: &[AccountInfo]) -> MangoResult<()> {
        todo!()
    }
    #[inline(never)]
    /// TODO figure out how to do docs for functions with link to instruction.rs instruction documentation
    /// TODO make the mango account a derived address
    fn init_mango_account(program_id: &Pubkey, accounts: &[AccountInfo]) -> MangoResult<()> {
        const NUM_FIXED: usize = 3;
        let accounts = array_ref![accounts, 0, NUM_FIXED];

        let [
            mango_group_ai,     // read
            mango_account_ai,   // write
            owner_ai            // read, signer
        ] = accounts;

        let rent = Rent::get()?;
        check!(
            rent.is_exempt(mango_account_ai.lamports(), size_of::<MangoAccount>()),
            MangoErrorCode::Default
        )?;
        check!(owner_ai.is_signer, MangoErrorCode::Default)?;

        let _mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;

        let mut mango_account: RefMut<MangoAccount> = MangoAccount::load_mut(mango_account_ai)?;
        check_eq!(&mango_account_ai.owner, &program_id, MangoErrorCode::InvalidOwner)?;
        check!(!mango_account.meta_data.is_initialized, MangoErrorCode::Default)?;

        mango_account.mango_group = *mango_group_ai.key;
        mango_account.owner = *owner_ai.key;
        mango_account.order_market = [FREE_ORDER_SLOT; MAX_PERP_OPEN_ORDERS];
        mango_account.meta_data = MetaData::new(DataType::MangoAccount, 0, true);

        Ok(())
    }

    #[inline(never)]
    /// Add asset and spot market to mango group
    /// Initialize a root bank and add it to the mango group
    /// Requires a price oracle for this asset priced in quote currency
    /// Only allow admin to add to MangoGroup
    // TODO - implement remove asset
    fn add_spot_market(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        market_index: usize,
        maint_leverage: I80F48,
        init_leverage: I80F48,
        liquidation_fee: I80F48,
        optimal_util: I80F48,
        optimal_rate: I80F48,
        max_rate: I80F48,
    ) -> MangoResult<()> {
        check!(
            init_leverage >= ONE_I80F48 && maint_leverage > init_leverage,
            MangoErrorCode::InvalidParam
        )?;

        const NUM_FIXED: usize = 8;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai, // write
            spot_market_ai, // read
            dex_program_ai, // read
            mint_ai,        // read
            node_bank_ai,   // write
            vault_ai,       // read
            root_bank_ai,   // write
            admin_ai        // read, signer
        ] = accounts;

        let mut mango_group = MangoGroup::load_mut_checked(mango_group_ai, program_id)?;

        check!(admin_ai.is_signer, MangoErrorCode::SignerNecessary)?;
        check_eq!(admin_ai.key, &mango_group.admin, MangoErrorCode::InvalidOwner)?;

        check!(market_index < mango_group.num_oracles, MangoErrorCode::InvalidParam)?;

        // Make sure there is an oracle at this index -- probably unnecessary because add_oracle is only place that modifies num_oracles
        check!(mango_group.oracles[market_index] != Pubkey::default(), MangoErrorCode::Default)?;

        // Make sure spot market at this index not already initialized
        check!(mango_group.spot_markets[market_index].is_empty(), MangoErrorCode::Default)?;

        // Make sure token at this index not already initialized
        check!(mango_group.tokens[market_index].is_empty(), MangoErrorCode::Default)?;
        let _root_bank = init_root_bank(
            program_id,
            &mango_group,
            mint_ai,
            vault_ai,
            root_bank_ai,
            node_bank_ai,
            &Rent::get()?,
            optimal_util,
            optimal_rate,
            max_rate,
        )?;

        let mint = Mint::unpack(&mint_ai.try_borrow_data()?)?;
        mango_group.tokens[market_index] = TokenInfo {
            mint: *mint_ai.key,
            root_bank: *root_bank_ai.key,
            decimals: mint.decimals,
            padding: [0u8; 7],
        };

        // check leverage is reasonable

        mango_group.spot_markets[market_index] = SpotMarketInfo {
            spot_market: *spot_market_ai.key,
            maint_asset_weight: (maint_leverage - ONE_I80F48).checked_div(maint_leverage).unwrap(),
            init_asset_weight: (init_leverage - ONE_I80F48).checked_div(init_leverage).unwrap(),
            maint_liab_weight: (maint_leverage + ONE_I80F48).checked_div(maint_leverage).unwrap(),
            init_liab_weight: (init_leverage + ONE_I80F48).checked_div(init_leverage).unwrap(),
            liquidation_fee,
        };

        let spot_market = load_market_state(spot_market_ai, dex_program_ai.key)?;

        check_eq!(
            identity(spot_market.coin_mint),
            mint_ai.key.to_aligned_bytes(),
            MangoErrorCode::Default
        )?;
        check_eq!(
            identity(spot_market.pc_mint),
            mango_group.tokens[QUOTE_INDEX].mint.to_aligned_bytes(),
            MangoErrorCode::Default
        )?;

        // TODO - what if quote currency is mngo, srm or msrm
        // if mint is SRM set srm_vault

        if mint_ai.key == &srm_token::ID {
            check!(mango_group.srm_vault == Pubkey::default(), MangoErrorCode::Default)?;
            mango_group.srm_vault = *vault_ai.key;
        }
        Ok(())
    }

    #[inline(never)]
    /// Add an oracle to the MangoGroup
    /// This must be called first before `add_spot_market` or `add_perp_market`
    /// There will never be a gap in the mango_group.oracles array
    fn add_oracle(program_id: &Pubkey, accounts: &[AccountInfo]) -> MangoResult<()> {
        const NUM_FIXED: usize = 3;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai, // write
            oracle_ai,      // write
            admin_ai        // read
        ] = accounts;

        let mut mango_group = MangoGroup::load_mut_checked(mango_group_ai, program_id)?;
        check!(admin_ai.is_signer, MangoErrorCode::Default)?;
        check_eq!(admin_ai.key, &mango_group.admin, MangoErrorCode::Default)?;

        let oracle_type = determine_oracle_type(oracle_ai);
        match oracle_type {
            OracleType::Pyth => {
                msg!("OracleType:Pyth"); // Do nothing really cause all that's needed is storing the pkey
            }
            OracleType::Switchboard => {
                msg!("OracleType::Switchboard");
            }
            OracleType::Stub | OracleType::Unknown => {
                msg!("OracleType: got unknown or stub");
                let rent = Rent::get()?;
                let mut oracle = StubOracle::load_and_init(oracle_ai, program_id, &rent)?;
                oracle.magic = 0x6F676E4D;
            }
        }

        let oracle_index = mango_group.num_oracles;
        mango_group.oracles[oracle_index] = *oracle_ai.key;
        mango_group.num_oracles += 1;

        Ok(())
    }

    #[inline(never)]
    fn set_oracle(program_id: &Pubkey, accounts: &[AccountInfo], price: I80F48) -> MangoResult<()> {
        const NUM_FIXED: usize = 3;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai, // read
            oracle_ai,      // write
            admin_ai        // read
        ] = accounts;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        check!(admin_ai.is_signer, MangoErrorCode::Default)?;
        check_eq!(admin_ai.key, &mango_group.admin, MangoErrorCode::Default)?;
        check!(mango_group.find_oracle_index(oracle_ai.key).is_some(), MangoErrorCode::Default)?;

        let oracle_type = determine_oracle_type(oracle_ai);
        check_eq!(oracle_type, OracleType::Stub, MangoErrorCode::Default)?;

        let mut oracle = StubOracle::load_mut_checked(oracle_ai, program_id)?;
        oracle.price = price;
        let clock = Clock::get()?;
        oracle.last_update = clock.unix_timestamp as u64;
        Ok(())
    }

    #[inline(never)]
    /// Initialize perp market including orderbooks and queues
    fn add_perp_market(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        market_index: usize,
        maint_leverage: I80F48,
        init_leverage: I80F48,
        liquidation_fee: I80F48,
        maker_fee: I80F48,
        taker_fee: I80F48,
        base_lot_size: i64,
        quote_lot_size: i64,
        rate: I80F48, // starting rate for liquidity mining
        max_depth_bps: I80F48,
        target_period_length: u64,
        mngo_per_period: u64,
    ) -> MangoResult<()> {
        // params check
        check!(init_leverage >= ONE_I80F48, MangoErrorCode::InvalidParam)?;
        check!(maint_leverage > init_leverage, MangoErrorCode::InvalidParam)?;
        check!(maker_fee + taker_fee >= ZERO_I80F48, MangoErrorCode::InvalidParam)?;
        check!(base_lot_size.is_positive(), MangoErrorCode::InvalidParam)?;
        check!(quote_lot_size.is_positive(), MangoErrorCode::InvalidParam)?;
        check!(!max_depth_bps.is_negative(), MangoErrorCode::InvalidParam)?;
        check!(!rate.is_negative(), MangoErrorCode::InvalidParam)?;
        check!(target_period_length > 0, MangoErrorCode::InvalidParam)?;

        const NUM_FIXED: usize = 7;
        let accounts = array_ref![accounts, 0, NUM_FIXED];

        let [
            mango_group_ai, // write
            perp_market_ai, // write
            event_queue_ai, // write
            bids_ai,        // write
            asks_ai,        // write
            mngo_vault_ai,  // read
            admin_ai        // read, signer
        ] = accounts;

        let rent = Rent::get()?; // dynamically load rent sysvar

        let mut mango_group = MangoGroup::load_mut_checked(mango_group_ai, program_id)?;

        check!(admin_ai.is_signer, MangoErrorCode::Default)?;
        check_eq!(admin_ai.key, &mango_group.admin, MangoErrorCode::Default)?;

        check!(market_index < mango_group.num_oracles, MangoErrorCode::InvalidParam)?;

        // Make sure there is an oracle at this index -- probably unnecessary because add_oracle is only place that modifies num_oracles
        check!(mango_group.oracles[market_index] != Pubkey::default(), MangoErrorCode::Default)?;

        // Make sure perp market at this index not already initialized
        check!(mango_group.perp_markets[market_index].is_empty(), MangoErrorCode::InvalidParam)?;

        mango_group.perp_markets[market_index] = PerpMarketInfo {
            perp_market: *perp_market_ai.key,
            maint_asset_weight: (maint_leverage - ONE_I80F48).checked_div(maint_leverage).unwrap(),
            init_asset_weight: (init_leverage - ONE_I80F48).checked_div(init_leverage).unwrap(),
            maint_liab_weight: (maint_leverage + ONE_I80F48).checked_div(maint_leverage).unwrap(),
            init_liab_weight: (init_leverage + ONE_I80F48).checked_div(init_leverage).unwrap(),
            liquidation_fee,
            maker_fee,
            taker_fee,
            base_lot_size,
            quote_lot_size,
        };

        // Initialize the Bids
        let _bids = BookSide::load_and_init(bids_ai, program_id, DataType::Bids, &rent)?;

        // Initialize the Asks
        let _asks = BookSide::load_and_init(asks_ai, program_id, DataType::Asks, &rent)?;

        // Initialize the EventQueue
        // TODO: check that the event queue is reasonably large
        let _event_queue = EventQueue::load_and_init(event_queue_ai, program_id, &rent)?;

        // Now initialize the PerpMarket itself
        let _perp_market = PerpMarket::load_and_init(
            perp_market_ai,
            program_id,
            mango_group_ai,
            bids_ai,
            asks_ai,
            event_queue_ai,
            mngo_vault_ai,
            &mango_group,
            &rent,
            base_lot_size,
            quote_lot_size,
            rate,
            max_depth_bps,
            target_period_length,
            mngo_per_period,
        )?;

        Ok(())
    }

    #[inline(never)]
    /// Deposit instruction
    fn deposit(program_id: &Pubkey, accounts: &[AccountInfo], quantity: u64) -> MangoResult<()> {
        const NUM_FIXED: usize = 9;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai,         // read
            mango_account_ai,       // write
            owner_ai,               // read
            mango_cache_ai,         // read
            root_bank_ai,           // read
            node_bank_ai,           // write
            vault_ai,               // write
            token_prog_ai,          // read
            owner_token_account_ai, // write
        ] = accounts;
        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        let mut mango_account =
            MangoAccount::load_mut_checked(mango_account_ai, program_id, mango_group_ai.key)?;

        // TODO - Probably not necessary for deposit to be from owner
        check_eq!(&mango_account.owner, owner_ai.key, MangoErrorCode::InvalidOwner)?;
        check!(!mango_account.is_bankrupt, MangoErrorCode::Bankrupt)?;
        let mango_cache = MangoCache::load_checked(mango_cache_ai, program_id, &mango_group)?;

        let token_index = mango_group
            .find_root_bank_index(root_bank_ai.key)
            .ok_or(throw_err!(MangoErrorCode::InvalidToken))?;

        let mut node_bank = NodeBank::load_mut_checked(node_bank_ai, program_id)?;

        // Find the node_bank pubkey in root_bank, if not found error
        let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
        check!(root_bank.node_banks.contains(node_bank_ai.key), MangoErrorCode::Default)?;
        check_eq!(&node_bank.vault, vault_ai.key, MangoErrorCode::InvalidVault)?;

        // deposit into node bank token vault using invoke_transfer
        check_eq!(token_prog_ai.key, &spl_token::ID, MangoErrorCode::Default)?;

        invoke_transfer(token_prog_ai, owner_token_account_ai, vault_ai, owner_ai, &[], quantity)?;

        // Check validity of root bank cache
        let now_ts = Clock::get()?.unix_timestamp as u64;
        let root_bank_cache = &mango_cache.root_bank_cache[token_index];
        check!(
            now_ts <= root_bank_cache.last_update + mango_group.valid_interval,
            MangoErrorCode::InvalidCache
        )?;

        checked_add_net(
            root_bank_cache,
            &mut node_bank,
            &mut mango_account,
            token_index,
            I80F48::from_num(quantity),
        )
    }

    // TODO create client functions and instruction.rs
    #[inline(never)]
    #[allow(unused)]
    /// Change the shape of the interest rate function
    fn set_rate_params(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        optimal_util: I80F48,
        optimal_rate: I80F48,
        max_rate: I80F48,
    ) -> MangoResult<()> {
        const NUM_FIXED: usize = 3;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai, // read
            root_bank_ai,   // read
            admin_ai        // read, signer
        ] = accounts;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        check!(admin_ai.is_signer, MangoErrorCode::Default)?;
        check_eq!(admin_ai.key, &mango_group.admin, MangoErrorCode::Default)?;
        check!(
            mango_group.find_root_bank_index(root_bank_ai.key).is_some(),
            MangoErrorCode::InvalidRootBank
        )?;
        let mut root_bank = RootBank::load_mut_checked(root_bank_ai, program_id)?;
        root_bank.set_rate_params(optimal_util, optimal_rate, max_rate)?;

        Ok(())
    }

    #[inline(never)]
    /// Write oracle prices onto MangoAccount before calling a value-dep instruction (e.g. Withdraw)
    fn cache_prices(program_id: &Pubkey, accounts: &[AccountInfo]) -> MangoResult<()> {
        const NUM_FIXED: usize = 2;
        let (fixed_ais, oracle_ais) = array_refs![accounts, NUM_FIXED; ..;];
        let [
            mango_group_ai,     // read
            mango_cache_ai,     // write
        ] = fixed_ais;
        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        let mut mango_cache =
            MangoCache::load_mut_checked(mango_cache_ai, program_id, &mango_group)?;
        let clock = Clock::get()?;
        let now_ts = clock.unix_timestamp as u64;

        let mut oracle_indexes = Vec::new();
        let mut oracle_prices = Vec::new();

        for oracle_ai in oracle_ais.iter() {
            let oracle_index = mango_group.find_oracle_index(oracle_ai.key).ok_or(throw!())?;
            let oracle_price = read_oracle(&mango_group, oracle_index, oracle_ai)?;

            mango_cache.price_cache[oracle_index] =
                PriceCache { price: oracle_price, last_update: now_ts };

            oracle_indexes.push(oracle_index);
            oracle_prices.push(oracle_price.to_num::<f64>());
        }

        msg!(
            "cache_prices details: {{ \
            \"oracle_indexes\": {:?}, \
            \"oracle_prices\": {:?}
        }}",
            oracle_indexes,
            oracle_prices
        );

        Ok(())
    }

    #[inline(never)]
    fn cache_root_banks(program_id: &Pubkey, accounts: &[AccountInfo]) -> MangoResult<()> {
        const NUM_FIXED: usize = 2;
        let (fixed_ais, root_bank_ais) = array_refs![accounts, NUM_FIXED; ..;];
        let [
            mango_group_ai,     // read
            mango_cache_ai,     // write
        ] = fixed_ais;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        let mut mango_cache =
            MangoCache::load_mut_checked(mango_cache_ai, program_id, &mango_group)?;
        let clock = Clock::get()?;
        let now_ts = clock.unix_timestamp as u64;

        for root_bank_ai in root_bank_ais.iter() {
            let index = mango_group.find_root_bank_index(root_bank_ai.key).unwrap();
            let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
            mango_cache.root_bank_cache[index] = RootBankCache {
                deposit_index: root_bank.deposit_index,
                borrow_index: root_bank.borrow_index,
                last_update: now_ts,
            };
        }
        Ok(())
    }

    #[inline(never)]
    fn cache_perp_markets(program_id: &Pubkey, accounts: &[AccountInfo]) -> MangoResult<()> {
        const NUM_FIXED: usize = 2;
        let (fixed_ais, perp_market_ais) = array_refs![accounts, NUM_FIXED; ..;];
        let [
            mango_group_ai,     // read
            mango_cache_ai,     // write
        ] = fixed_ais;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        let mut mango_cache =
            MangoCache::load_mut_checked(mango_cache_ai, program_id, &mango_group)?;
        let clock = Clock::get()?;
        let now_ts = clock.unix_timestamp as u64;
        for perp_market_ai in perp_market_ais.iter() {
            let index = mango_group.find_perp_market_index(perp_market_ai.key).unwrap();
            let perp_market =
                PerpMarket::load_checked(perp_market_ai, program_id, mango_group_ai.key)?;
            mango_cache.perp_market_cache[index] = PerpMarketCache {
                long_funding: perp_market.long_funding,
                short_funding: perp_market.short_funding,
                last_update: now_ts,
            };
        }
        Ok(())
    }

    #[inline(never)]
    /// Withdraw a token from the bank if collateral ratio permits
    fn withdraw(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        quantity: u64,
        allow_borrow: bool, // TODO only borrow if true
    ) -> MangoResult<()> {
        const NUM_FIXED: usize = 10;
        let accounts = array_ref![accounts, 0, NUM_FIXED + MAX_PAIRS];
        let (fixed_ais, open_orders_ais) = array_refs![accounts, NUM_FIXED, MAX_PAIRS];
        let [
            mango_group_ai,     // read
            mango_account_ai,   // write
            owner_ai,           // read
            mango_cache_ai,     // read
            root_bank_ai,       // read
            node_bank_ai,       // write
            vault_ai,           // write
            token_account_ai,   // write
            signer_ai,          // read
            token_prog_ai,      // read
        ] = fixed_ais;
        check_eq!(&spl_token::ID, token_prog_ai.key, MangoErrorCode::InvalidProgramId)?;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;

        let mut mango_account =
            MangoAccount::load_mut_checked(mango_account_ai, program_id, mango_group_ai.key)?;
        check!(&mango_account.owner == owner_ai.key, MangoErrorCode::InvalidOwner)?;
        check!(!mango_account.is_bankrupt, MangoErrorCode::Bankrupt)?;
        check!(owner_ai.is_signer, MangoErrorCode::InvalidSignerKey)?;
        mango_account.check_open_orders(&mango_group, open_orders_ais)?;

        let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
        let token_index = mango_group
            .find_root_bank_index(root_bank_ai.key)
            .ok_or(throw_err!(MangoErrorCode::InvalidToken))?;

        let mut node_bank = NodeBank::load_mut_checked(node_bank_ai, program_id)?;
        check!(root_bank.node_banks.contains(node_bank_ai.key), MangoErrorCode::InvalidNodeBank)?;
        let clock = Clock::get()?;
        let now_ts = clock.unix_timestamp as u64;

        // Safety checks
        check_eq!(&node_bank.vault, vault_ai.key, MangoErrorCode::InvalidVault)?;

        let active_assets = UserActiveAssets::new(
            &mango_group,
            &mango_account,
            vec![(AssetType::Token, token_index)],
        );
        let mango_cache = MangoCache::load_checked(mango_cache_ai, program_id, &mango_group)?;
        mango_cache.check_valid(&mango_group, &active_assets, now_ts)?;

        let root_bank_cache = &mango_cache.root_bank_cache[token_index];

        // Borrow if withdrawing more than deposits
        let native_deposit = mango_account.get_native_deposit(root_bank_cache, token_index)?;
        let withdraw = I80F48::from_num(quantity);
        check!(native_deposit >= withdraw || allow_borrow, MangoErrorCode::InsufficientFunds)?;
        checked_sub_net(
            root_bank_cache,
            &mut node_bank,
            &mut mango_account,
            token_index,
            withdraw,
        )?;

        let signers_seeds = gen_signer_seeds(&mango_group.signer_nonce, mango_group_ai.key);
        invoke_transfer(
            token_prog_ai,
            vault_ai,
            token_account_ai,
            signer_ai,
            &[&signers_seeds],
            quantity,
        )?;

        let mut health_cache = HealthCache::new(active_assets);
        health_cache.init_vals(&mango_group, &mango_cache, &mango_account, open_orders_ais)?;
        let health = health_cache.get_health(&mango_group, HealthType::Init);

        check!(health >= ZERO_I80F48, MangoErrorCode::InsufficientFunds)?;

        Ok(())
    }

    #[inline(never)]
    /// Call the init_open_orders instruction in serum dex and add this OpenOrders account to margin account
    fn init_spot_open_orders(program_id: &Pubkey, accounts: &[AccountInfo]) -> MangoResult<()> {
        const NUM_FIXED: usize = 8;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai,         // read
            mango_account_ai,       // write
            owner_ai,               // read
            dex_prog_ai,            // read
            open_orders_ai,         // write
            spot_market_ai,         // read
            signer_ai,              // read
            rent_ai,                // read
        ] = accounts;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        check_eq!(dex_prog_ai.key, &mango_group.dex_program_id, MangoErrorCode::InvalidProgramId)?;

        let market_index = mango_group
            .find_spot_market_index(spot_market_ai.key)
            .ok_or(throw_err!(MangoErrorCode::InvalidMarket))?;

        let mut mango_account =
            MangoAccount::load_mut_checked(mango_account_ai, program_id, mango_group_ai.key)?;
        check!(&mango_account.owner == owner_ai.key, MangoErrorCode::InvalidOwner)?;
        check!(owner_ai.is_signer, MangoErrorCode::InvalidSignerKey)?;
        check!(!mango_account.is_bankrupt, MangoErrorCode::Bankrupt)?;

        {
            // TODO OPT - Unnecessary check because serum dex also checks account flags 0
            let open_orders = load_open_orders(open_orders_ai)?;

            // Make sure this open orders account has not been initialized already
            check_eq!(open_orders.account_flags, 0, MangoErrorCode::Default)?;
        }

        // Make sure there isn't already an open orders account for this market
        check!(
            mango_account.spot_open_orders[market_index] == Pubkey::default(),
            MangoErrorCode::Default
        )?;

        let signers_seeds = gen_signer_seeds(&mango_group.signer_nonce, mango_group_ai.key);
        invoke_init_open_orders(
            dex_prog_ai,
            open_orders_ai,
            signer_ai,
            spot_market_ai,
            rent_ai,
            &[&signers_seeds],
        )?;

        mango_account.spot_open_orders[market_index] = *open_orders_ai.key;

        Ok(())
    }

    // TODO - add serum dex fee discount functionality
    #[inline(never)]
    fn place_spot_order(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        order: serum_dex::instruction::NewOrderInstructionV3,
    ) -> MangoResult<()> {
        const NUM_FIXED: usize = 23;
        let accounts = array_ref![accounts, 0, NUM_FIXED + MAX_PAIRS];
        let (fixed_ais, open_orders_ais) = array_refs![accounts, NUM_FIXED, MAX_PAIRS];

        let [
            mango_group_ai,         // read
            mango_account_ai,       // write
            owner_ai,               // read & signer
            mango_cache_ai,         // read
            dex_prog_ai,            // read
            spot_market_ai,         // write
            bids_ai,                // write
            asks_ai,                // write
            dex_request_queue_ai,   // write
            dex_event_queue_ai,     // write
            dex_base_ai,            // write
            dex_quote_ai,           // write
            base_root_bank_ai,      // read
            base_node_bank_ai,      // write
            base_vault_ai,          // write
            quote_root_bank_ai,      // read
            quote_node_bank_ai,      // write
            quote_vault_ai,          // write
            token_prog_ai,          // read
            signer_ai,              // read
            rent_ai,                // read
            dex_signer_ai,          // read
            msrm_or_srm_vault_ai,   // read
        ] = fixed_ais;

        // TODO OPT - reduce size of this transaction
        // put bank info into group +64 bytes
        // remove settle_funds +64 bytes
        // ask serum dex to use dynamic sysvars +32 bytes
        // only send in open orders pubkeys we need +54 bytes
        // shrink size of order instruction +10 bytes

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        check_eq!(token_prog_ai.key, &spl_token::ID, MangoErrorCode::InvalidProgramId)?;
        check_eq!(dex_prog_ai.key, &mango_group.dex_program_id, MangoErrorCode::InvalidProgramId)?;

        let mut mango_account =
            MangoAccount::load_mut_checked(mango_account_ai, program_id, mango_group_ai.key)?;
        check!(&mango_account.owner == owner_ai.key, MangoErrorCode::InvalidOwner)?;
        check!(owner_ai.is_signer, MangoErrorCode::InvalidSignerKey)?;
        check!(!mango_account.is_bankrupt, MangoErrorCode::Bankrupt)?;

        let clock = Clock::get()?;
        let now_ts = clock.unix_timestamp as u64;

        let market_index = mango_group
            .find_spot_market_index(spot_market_ai.key)
            .ok_or(throw_err!(MangoErrorCode::InvalidMarket))?;

        check!(
            &mango_group.tokens[market_index].root_bank == base_root_bank_ai.key,
            MangoErrorCode::InvalidRootBank
        )?;
        let base_root_bank = RootBank::load_checked(base_root_bank_ai, program_id)?;

        check!(
            base_root_bank.node_banks.contains(base_node_bank_ai.key),
            MangoErrorCode::InvalidNodeBank
        )?;
        let mut base_node_bank = NodeBank::load_mut_checked(base_node_bank_ai, program_id)?;

        check_eq!(&base_node_bank.vault, base_vault_ai.key, MangoErrorCode::InvalidVault)?;

        let quote_root_bank = RootBank::load_checked(quote_root_bank_ai, program_id)?;

        check!(
            &mango_group.tokens[QUOTE_INDEX].root_bank == quote_root_bank_ai.key,
            MangoErrorCode::InvalidRootBank
        )?;

        check!(
            quote_root_bank.node_banks.contains(quote_node_bank_ai.key),
            MangoErrorCode::InvalidNodeBank
        )?;
        let mut quote_node_bank = NodeBank::load_mut_checked(quote_node_bank_ai, program_id)?;

        check_eq!(&quote_node_bank.vault, quote_vault_ai.key, MangoErrorCode::InvalidVault)?;

        // Adjust margin basket; this also makes this market an active asset
        mango_account.add_to_basket(market_index)?;
        mango_account.check_open_orders(&mango_group, open_orders_ais)?;

        let active_assets = UserActiveAssets::new(&mango_group, &mango_account, vec![]);
        let mango_cache = MangoCache::load_checked(mango_cache_ai, program_id, &mango_group)?;
        mango_cache.check_valid(&mango_group, &active_assets, now_ts)?;

        let mut health_cache = HealthCache::new(active_assets);
        health_cache.init_vals(&mango_group, &mango_cache, &mango_account, open_orders_ais)?;
        let pre_health = health_cache.get_health(&mango_group, HealthType::Init);

        // update the being_liquidated flag
        if mango_account.being_liquidated {
            if pre_health >= ZERO_I80F48 {
                mango_account.being_liquidated = false;
            } else {
                return Err(throw_err!(MangoErrorCode::BeingLiquidated));
            }
        }

        // This means health must only go up
        let reduce_only = pre_health < ZERO_I80F48;

        // TODO maybe check that root bank was updated recently
        // TODO maybe check oracle was updated recently

        // TODO OPT - write a zero copy way to deserialize Account to reduce compute
        // this is to keep track of the amount of funds transferred
        let (pre_base, pre_quote) = {
            (
                Account::unpack(&base_vault_ai.try_borrow_data()?)?.amount,
                Account::unpack(&quote_vault_ai.try_borrow_data()?)?.amount,
            )
        };
        let vault_ai = match order.side {
            serum_dex::matching::Side::Bid => quote_vault_ai,
            serum_dex::matching::Side::Ask => base_vault_ai,
        };

        // Send order to serum dex
        let signers_seeds = gen_signer_seeds(&mango_group.signer_nonce, mango_group_ai.key);
        invoke_new_order(
            dex_prog_ai,
            spot_market_ai,
            &open_orders_ais[market_index],
            dex_request_queue_ai,
            dex_event_queue_ai,
            bids_ai,
            asks_ai,
            vault_ai,
            signer_ai,
            dex_base_ai,
            dex_quote_ai,
            token_prog_ai,
            rent_ai,
            msrm_or_srm_vault_ai,
            &[&signers_seeds],
            order,
        )?;

        // Settle funds for this market
        invoke_settle_funds(
            dex_prog_ai,
            spot_market_ai,
            &open_orders_ais[market_index],
            signer_ai,
            dex_base_ai,
            dex_quote_ai,
            base_vault_ai,
            quote_vault_ai,
            dex_signer_ai,
            token_prog_ai,
            &[&signers_seeds],
        )?;

        // See if we can remove this market from margin
        let open_orders = load_open_orders(&open_orders_ais[market_index])?;
        mango_account.update_basket(market_index, &open_orders)?;

        let (post_base, post_quote) = {
            (
                Account::unpack(&base_vault_ai.try_borrow_data()?)?.amount,
                Account::unpack(&quote_vault_ai.try_borrow_data()?)?.amount,
            )
        };

        let quote_change = I80F48::from_num(post_quote) - I80F48::from_num(pre_quote);
        let base_change = I80F48::from_num(post_base) - I80F48::from_num(pre_base);

        checked_change_net(
            &mango_cache.root_bank_cache[QUOTE_INDEX],
            &mut quote_node_bank,
            &mut mango_account,
            QUOTE_INDEX,
            quote_change,
        )?;

        checked_change_net(
            &mango_cache.root_bank_cache[market_index],
            &mut base_node_bank,
            &mut mango_account,
            market_index,
            base_change,
        )?;

        // Update health for tokens that may have changed
        health_cache.update_quote(&mango_cache, &mango_account);
        health_cache.update_spot_val(
            &mango_group,
            &mango_cache,
            &mango_account,
            &open_orders_ais[market_index],
            market_index,
        )?;
        let post_health = health_cache.get_health(&mango_group, HealthType::Init);

        // If an account is in reduce_only mode, health must only go up
        check!(
            post_health >= ZERO_I80F48 || (reduce_only && post_health >= pre_health),
            MangoErrorCode::InsufficientFunds
        )
    }

    #[inline(never)]
    fn cancel_spot_order(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        data: Vec<u8>,
    ) -> MangoResult<()> {
        // TODO add param `ok_invalid_id` to return Ok() instead of Err if order id or client id invalid

        const NUM_FIXED: usize = 10;
        let accounts = array_ref![accounts, 0, NUM_FIXED];

        let [
            mango_group_ai,     // read
            owner_ai,           // signer
            mango_account_ai,   // read
            dex_prog_ai,        // read
            spot_market_ai,     // write
            bids_ai,            // write
            asks_ai,            // write
            open_orders_ai,     // write
            signer_ai,          // read
            dex_event_queue_ai, // write
        ] = accounts;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        check_eq!(dex_prog_ai.key, &mango_group.dex_program_id, MangoErrorCode::Default)?;

        let mango_account =
            MangoAccount::load_checked(mango_account_ai, program_id, mango_group_ai.key)?;
        check!(!mango_account.is_bankrupt, MangoErrorCode::Bankrupt)?;
        check!(owner_ai.is_signer, MangoErrorCode::Default)?;
        check_eq!(&mango_account.owner, owner_ai.key, MangoErrorCode::Default)?;

        let market_i = mango_group.find_spot_market_index(spot_market_ai.key).unwrap();
        check_eq!(
            &mango_account.spot_open_orders[market_i],
            open_orders_ai.key,
            MangoErrorCode::Default
        )?;

        let signer_seeds = gen_signer_seeds(&mango_group.signer_nonce, mango_group_ai.key);
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

    #[inline(never)]
    fn settle_funds(program_id: &Pubkey, accounts: &[AccountInfo]) -> MangoResult<()> {
        const NUM_FIXED: usize = 18;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai,         // read
            mango_cache_ai,         // read
            owner_ai,               // signer
            mango_account_ai,       // write
            dex_prog_ai,            // read
            spot_market_ai,         // write
            open_orders_ai,         // write
            signer_ai,              // read
            dex_base_ai,            // write
            dex_quote_ai,           // write
            base_root_bank_ai,      // read
            base_node_bank_ai,      // write
            quote_root_bank_ai,     // read
            quote_node_bank_ai,     // write
            base_vault_ai,          // write
            quote_vault_ai,         // write
            dex_signer_ai,          // read
            token_prog_ai,          // read
        ] = accounts;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        check_eq!(token_prog_ai.key, &spl_token::id(), MangoErrorCode::Default)?;
        check_eq!(dex_prog_ai.key, &mango_group.dex_program_id, MangoErrorCode::Default)?;

        let mut mango_account =
            MangoAccount::load_mut_checked(mango_account_ai, program_id, mango_group_ai.key)?;
        check!(owner_ai.key == &mango_account.owner, MangoErrorCode::InvalidOwner)?;
        check!(owner_ai.is_signer, MangoErrorCode::SignerNecessary)?;
        check!(!mango_account.is_bankrupt, MangoErrorCode::Bankrupt)?;

        // Make sure the spot market is valid
        let spot_market_index = mango_group
            .find_spot_market_index(spot_market_ai.key)
            .ok_or(throw_err!(MangoErrorCode::InvalidMarket))?;

        let base_root_bank = RootBank::load_checked(base_root_bank_ai, program_id)?;
        check!(
            base_root_bank_ai.key == &mango_group.tokens[spot_market_index].root_bank,
            MangoErrorCode::InvalidRootBank
        )?;

        let mut base_node_bank = NodeBank::load_mut_checked(base_node_bank_ai, program_id)?;
        check!(
            base_root_bank.node_banks.contains(base_node_bank_ai.key),
            MangoErrorCode::InvalidNodeBank
        )?;
        check_eq!(&base_node_bank.vault, base_vault_ai.key, MangoErrorCode::InvalidVault)?;

        let quote_root_bank = RootBank::load_checked(quote_root_bank_ai, program_id)?;
        check!(
            quote_root_bank_ai.key == &mango_group.tokens[QUOTE_INDEX].root_bank,
            MangoErrorCode::InvalidRootBank
        )?;
        let mut quote_node_bank = NodeBank::load_mut_checked(quote_node_bank_ai, program_id)?;
        check!(
            quote_root_bank.node_banks.contains(quote_node_bank_ai.key),
            MangoErrorCode::InvalidNodeBank
        )?;
        check_eq!(&quote_node_bank.vault, quote_vault_ai.key, MangoErrorCode::InvalidVault)?;

        check_eq!(
            &mango_account.spot_open_orders[spot_market_index],
            open_orders_ai.key,
            MangoErrorCode::Default
        )?;

        if *open_orders_ai.key == Pubkey::default() {
            return Ok(());
        }

        let (pre_base, pre_quote) = {
            let open_orders = load_open_orders(open_orders_ai)?;
            (
                open_orders.native_coin_free,
                open_orders.native_pc_free + open_orders.referrer_rebates_accrued,
            )
        };

        let signer_seeds = gen_signer_seeds(&mango_group.signer_nonce, mango_group_ai.key);
        invoke_settle_funds(
            dex_prog_ai,
            spot_market_ai,
            open_orders_ai,
            signer_ai,
            dex_base_ai,
            dex_quote_ai,
            base_vault_ai,
            quote_vault_ai,
            dex_signer_ai,
            token_prog_ai,
            &[&signer_seeds],
        )?;

        let (post_base, post_quote) = {
            let open_orders = load_open_orders(open_orders_ai)?;
            // remove from margin basket if it's empty
            mango_account.update_basket(spot_market_index, &open_orders)?;

            (
                open_orders.native_coin_free,
                open_orders.native_pc_free + open_orders.referrer_rebates_accrued,
            )
        };

        // TODO OPT - remove sanity check if confident
        check!(post_base <= pre_base, MangoErrorCode::Default)?;
        check!(post_quote <= pre_quote, MangoErrorCode::Default)?;

        let mango_cache = MangoCache::load_checked(mango_cache_ai, program_id, &mango_group)?;
        let valid_last_update = Clock::get()?.unix_timestamp as u64 - mango_group.valid_interval;
        check!(
            mango_cache.root_bank_cache[spot_market_index].last_update >= valid_last_update,
            MangoErrorCode::InvalidCache
        )?;
        check!(
            mango_cache.root_bank_cache[QUOTE_INDEX].last_update >= valid_last_update,
            MangoErrorCode::InvalidCache
        )?;

        checked_add_net(
            &mango_cache.root_bank_cache[spot_market_index],
            &mut base_node_bank,
            &mut mango_account,
            spot_market_index,
            I80F48::from_num(pre_base - post_base),
        )?;
        checked_add_net(
            &mango_cache.root_bank_cache[QUOTE_INDEX],
            &mut quote_node_bank,
            &mut mango_account,
            QUOTE_INDEX,
            I80F48::from_num(pre_quote - post_quote),
        )?;

        Ok(())
    }

    #[inline(never)]
    fn place_perp_order(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        side: Side,
        price: i64,
        quantity: i64,
        client_order_id: u64,
        order_type: OrderType,
    ) -> MangoResult<()> {
        check!(price > 0, MangoErrorCode::InvalidParam)?;
        check!(quantity > 0, MangoErrorCode::InvalidParam)?;

        const NUM_FIXED: usize = 8;
        let accounts = array_ref![accounts, 0, NUM_FIXED + MAX_PAIRS];
        let (fixed_ais, open_orders_ais) = array_refs![accounts, NUM_FIXED, MAX_PAIRS];
        let [
            mango_group_ai,     // read
            mango_account_ai,   // write
            owner_ai,           // read, signer
            mango_cache_ai,     // read
            perp_market_ai,     // write
            bids_ai,            // write
            asks_ai,            // write
            event_queue_ai,     // write
        ] = fixed_ais;
        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;

        let mut mango_account =
            MangoAccount::load_mut_checked(mango_account_ai, program_id, mango_group_ai.key)?;
        check!(!mango_account.is_bankrupt, MangoErrorCode::Bankrupt)?;
        check!(owner_ai.is_signer, MangoErrorCode::SignerNecessary)?;
        check_eq!(&mango_account.owner, owner_ai.key, MangoErrorCode::InvalidOwner)?;
        mango_account.check_open_orders(&mango_group, open_orders_ais)?;

        let clock = Clock::get()?;
        let now_ts = clock.unix_timestamp as u64;

        // TODO could also make class PosI64 but it gets ugly when doing computations. Maybe have to do this with a large enough dev team

        let mut perp_market =
            PerpMarket::load_mut_checked(perp_market_ai, program_id, mango_group_ai.key)?;
        let market_index = mango_group
            .find_perp_market_index(perp_market_ai.key)
            .ok_or(throw_err!(MangoErrorCode::InvalidMarket))?;

        let active_assets = UserActiveAssets::new(
            &mango_group,
            &mango_account,
            vec![(AssetType::Perp, market_index)],
        );

        let mango_cache = MangoCache::load_checked(mango_cache_ai, program_id, &mango_group)?;
        mango_cache.check_valid(&mango_group, &active_assets, now_ts)?;

        let mut health_cache = HealthCache::new(active_assets);
        health_cache.init_vals(&mango_group, &mango_cache, &mango_account, open_orders_ais)?;
        let pre_health = health_cache.get_health(&mango_group, HealthType::Init);

        // update the being_liquidated flag
        if mango_account.being_liquidated {
            if pre_health >= ZERO_I80F48 {
                mango_account.being_liquidated = false;
            } else {
                return Err(throw_err!(MangoErrorCode::BeingLiquidated));
            }
        }

        // This means health must only go up
        let reduce_only = pre_health < ZERO_I80F48;

        let mut book = Book::load_checked(program_id, bids_ai, asks_ai, &perp_market)?;
        let mut event_queue =
            EventQueue::load_mut_checked(event_queue_ai, program_id, &perp_market)?;

        book.new_order(
            &mut event_queue,
            &mut perp_market,
            &mango_group.perp_markets[market_index],
            &mut mango_account,
            mango_account_ai.key,
            market_index,
            side,
            price,
            quantity,
            order_type,
            client_order_id,
            now_ts,
        )?;

        health_cache.update_perp_val(&mango_group, &mango_cache, &mango_account, market_index)?;
        let post_health = health_cache.get_health(&mango_group, HealthType::Init);
        // If an account is in reduce_only mode, health must only go up
        check!(
            post_health >= ZERO_I80F48 || (reduce_only && post_health >= pre_health),
            MangoErrorCode::InsufficientFunds
        )
    }

    #[inline(never)]
    fn cancel_perp_order_by_client_id(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        client_order_id: u64,
    ) -> MangoResult<()> {
        const NUM_FIXED: usize = 6;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai,     // read
            mango_account_ai,   // write
            owner_ai,           // read, signer
            perp_market_ai,     // write
            bids_ai,            // write
            asks_ai,            // write
        ] = accounts;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;

        let mut mango_account =
            MangoAccount::load_mut_checked(mango_account_ai, program_id, mango_group_ai.key)?;
        check!(!mango_account.is_bankrupt, MangoErrorCode::Bankrupt)?;
        check!(owner_ai.is_signer, MangoErrorCode::Default)?;
        check_eq!(&mango_account.owner, owner_ai.key, MangoErrorCode::InvalidOwner)?;

        let mut perp_market =
            PerpMarket::load_mut_checked(perp_market_ai, program_id, mango_group_ai.key)?;

        let market_index = mango_group.find_perp_market_index(perp_market_ai.key).unwrap();

        let (order_id, side) = mango_account
            .find_order_with_client_id(market_index, client_order_id)
            .ok_or(throw_err!(MangoErrorCode::ClientIdNotFound))?;

        let mut book = Book::load_checked(program_id, bids_ai, asks_ai, &perp_market)?;

        let best_final = match side {
            Side::Bid => book.get_best_bid_price().unwrap(),
            Side::Ask => book.get_best_ask_price().unwrap(),
        };

        let order = book.cancel_order(order_id, side)?;
        check_eq!(&order.owner, mango_account_ai.key, MangoErrorCode::InvalidOrderId)?;
        mango_account.remove_order(order.owner_slot as usize, order.quantity)?;

        let perp_account = &mut mango_account.perp_accounts[market_index];
        perp_account.apply_incentives(
            &mut perp_market,
            side,
            order.price(),
            order.best_initial,
            best_final,
            order.timestamp,
            Clock::get()?.unix_timestamp as u64,
            order.quantity,
        )?;

        Ok(())
    }

    #[inline(never)]
    fn cancel_perp_order(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        order_id: i128,
    ) -> MangoResult<()> {
        const NUM_FIXED: usize = 6;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai,     // read
            mango_account_ai,   // write
            owner_ai,           // read, signer
            perp_market_ai,     // write
            bids_ai,            // write
            asks_ai,            // write
        ] = accounts;

        // TODO OPT put the liquidity incentive stuff in the bids and asks accounts so perp market
        //  doesn't have to be passed in as write

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;

        let mut mango_account =
            MangoAccount::load_mut_checked(mango_account_ai, program_id, mango_group_ai.key)?;
        check!(!mango_account.is_bankrupt, MangoErrorCode::Bankrupt)?;
        check!(owner_ai.is_signer, MangoErrorCode::Default)?;
        check_eq!(&mango_account.owner, owner_ai.key, MangoErrorCode::InvalidOwner)?;

        let mut perp_market =
            PerpMarket::load_mut_checked(perp_market_ai, program_id, mango_group_ai.key)?;

        let market_index = mango_group.find_perp_market_index(perp_market_ai.key).unwrap();
        let side = mango_account
            .find_order_side(market_index, order_id)
            .ok_or(throw_err!(MangoErrorCode::InvalidOrderId))?;
        let mut book = Book::load_checked(program_id, bids_ai, asks_ai, &perp_market)?;

        let best_final = match side {
            Side::Bid => book.get_best_bid_price().unwrap(),
            Side::Ask => book.get_best_ask_price().unwrap(),
        };

        let order = book.cancel_order(order_id, side)?;
        check_eq!(&order.owner, mango_account_ai.key, MangoErrorCode::InvalidOrderId)?;
        mango_account.remove_order(order.owner_slot as usize, order.quantity)?;
        mango_account.perp_accounts[market_index].apply_incentives(
            &mut perp_market,
            side,
            order.price(),
            order.best_initial,
            best_final,
            order.timestamp,
            Clock::get()?.unix_timestamp as u64,
            order.quantity,
        )?;

        Ok(())
    }

    #[inline(never)]
    /// Take two MangoAccount and settle quote currency pnl between them
    fn settle_pnl(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        market_index: usize,
    ) -> MangoResult<()> {
        // TODO - what if someone has no collateral except other perps contracts
        //  maybe you don't allow people to withdraw if they don't have enough
        //  when liquidating, make sure you settle their pnl first?
        // TODO consider doing this in batches of 32 accounts that are close to zero sum
        // TODO write unit tests for this function

        const NUM_FIXED: usize = 6;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai,     // read
            mango_account_a_ai, // write
            mango_account_b_ai, // write
            mango_cache_ai,     // read
            root_bank_ai,       // read
            node_bank_ai,       // write
        ] = accounts;
        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;

        let mut mango_account_a =
            MangoAccount::load_mut_checked(mango_account_a_ai, program_id, mango_group_ai.key)?;
        check!(!mango_account_a.is_bankrupt, MangoErrorCode::Bankrupt)?;

        let mut mango_account_b =
            MangoAccount::load_mut_checked(mango_account_b_ai, program_id, mango_group_ai.key)?;
        check!(!mango_account_b.is_bankrupt, MangoErrorCode::Bankrupt)?;

        match mango_group.find_root_bank_index(root_bank_ai.key) {
            None => return Err(throw_err!(MangoErrorCode::Default)),
            Some(i) => check!(i == QUOTE_INDEX, MangoErrorCode::Default)?,
        }
        let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
        let mut node_bank = NodeBank::load_mut_checked(node_bank_ai, program_id)?;
        check!(root_bank.node_banks.contains(node_bank_ai.key), MangoErrorCode::Default)?;

        let mango_cache = MangoCache::load_checked(mango_cache_ai, program_id, &mango_group)?;
        let now_ts = Clock::get()?.unix_timestamp as u64;

        let valid_last_update = now_ts - mango_group.valid_interval;
        let perp_market_cache = &mango_cache.perp_market_cache[market_index];

        check!(
            valid_last_update <= mango_cache.price_cache[market_index].last_update,
            MangoErrorCode::InvalidCache
        )?;
        check!(
            valid_last_update <= mango_cache.root_bank_cache[QUOTE_INDEX].last_update,
            MangoErrorCode::InvalidCache
        )?;
        check!(valid_last_update <= perp_market_cache.last_update, MangoErrorCode::InvalidCache)?;

        let price = mango_cache.price_cache[market_index].price;

        // No need to check if market_index is in basket because if it's not, it will be zero and not possible to settle

        let a = &mut mango_account_a.perp_accounts[market_index];
        let b = &mut mango_account_b.perp_accounts[market_index];

        // Account for unrealized funding payments before settling
        a.settle_funding(perp_market_cache);
        b.settle_funding(perp_market_cache);

        let contract_size = mango_group.perp_markets[market_index].base_lot_size;
        let new_quote_pos_a = I80F48::from_num(-a.base_position * contract_size) * price;
        let new_quote_pos_b = I80F48::from_num(-b.base_position * contract_size) * price;
        let a_pnl = a.quote_position - new_quote_pos_a;
        let b_pnl = b.quote_position - new_quote_pos_b;

        // pnl must be opposite signs for there to be a settlement
        if a_pnl * b_pnl > 0 {
            return Ok(());
        }

        let settlement = a_pnl.abs().min(b_pnl.abs());
        if a_pnl > 0 {
            a.quote_position -= settlement;
            b.quote_position += settlement;
        } else {
            a.quote_position += settlement;
            b.quote_position -= settlement;
        }

        checked_add_net(
            &mango_cache.root_bank_cache[QUOTE_INDEX],
            &mut node_bank,
            if a_pnl > 0 { &mut mango_account_a } else { &mut mango_account_b },
            QUOTE_INDEX,
            settlement,
        )?;
        checked_sub_net(
            &mango_cache.root_bank_cache[QUOTE_INDEX],
            &mut node_bank,
            if a_pnl > 0 { &mut mango_account_b } else { &mut mango_account_a },
            QUOTE_INDEX,
            settlement,
        )
    }

    #[inline(never)]
    /// Take an account that has losses in the selected perp market to account for fees_accrued
    fn settle_fees(program_id: &Pubkey, accounts: &[AccountInfo]) -> MangoResult<()> {
        // TODO
        const NUM_FIXED: usize = 11;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai,     // read
            mango_cache_ai,     // read
            perp_market_ai,     // write
            mango_account_ai,   // write
            root_bank_ai,       // read
            node_bank_ai,       // write
            bank_vault_ai,      // write
            dao_vault_ai,       // write
            signer_ai,          // read
            admin_ai,           // read, signer
            token_prog_ai,      // read
        ] = accounts;
        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        let mut perp_market =
            PerpMarket::load_mut_checked(perp_market_ai, program_id, mango_group_ai.key)?;
        let market_index = mango_group.find_perp_market_index(perp_market_ai.key).unwrap();

        let mut mango_account =
            MangoAccount::load_mut_checked(mango_account_ai, program_id, mango_group_ai.key)?;
        check!(!mango_account.is_bankrupt, MangoErrorCode::Bankrupt)?;

        check!(admin_ai.key == &mango_group.admin, MangoErrorCode::InvalidSignerKey)?;
        check!(admin_ai.is_signer, MangoErrorCode::InvalidSignerKey)?;
        check!(signer_ai.key == &mango_group.signer_key, MangoErrorCode::InvalidSignerKey)?;

        match mango_group.find_root_bank_index(root_bank_ai.key) {
            None => return Err(throw_err!(MangoErrorCode::InvalidRootBank)),
            Some(i) => check!(i == QUOTE_INDEX, MangoErrorCode::InvalidRootBank)?,
        }
        let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
        let mut node_bank = NodeBank::load_mut_checked(node_bank_ai, program_id)?;
        check!(root_bank.node_banks.contains(node_bank_ai.key), MangoErrorCode::Default)?;
        check!(bank_vault_ai.key == &node_bank.vault, MangoErrorCode::InvalidVault)?;

        let mango_cache = MangoCache::load_checked(mango_cache_ai, program_id, &mango_group)?;
        let now_ts = Clock::get()?.unix_timestamp as u64;

        let valid_last_update = now_ts - mango_group.valid_interval;
        let perp_market_cache = &mango_cache.perp_market_cache[market_index];
        let root_bank_cache = &mango_cache.root_bank_cache[QUOTE_INDEX];

        check!(
            valid_last_update <= mango_cache.price_cache[market_index].last_update,
            MangoErrorCode::InvalidCache
        )?;
        check!(valid_last_update <= root_bank_cache.last_update, MangoErrorCode::InvalidCache)?;
        check!(valid_last_update <= perp_market_cache.last_update, MangoErrorCode::InvalidCache)?;

        let price = mango_cache.price_cache[market_index].price;

        let pa = &mut mango_account.perp_accounts[market_index];

        let contract_size = mango_group.perp_markets[market_index].base_lot_size;
        let new_quote_pos = I80F48::from_num(-pa.base_position * contract_size) * price;
        let pnl: I80F48 = pa.quote_position - new_quote_pos;
        check!(pnl.is_negative(), MangoErrorCode::Default)?;
        check!(perp_market.fees_accrued.is_positive(), MangoErrorCode::Default)?;

        let settlement = pnl.abs().min(perp_market.fees_accrued).checked_floor().unwrap();

        perp_market.fees_accrued -= settlement;
        pa.quote_position += settlement;

        // Transfer quote token from bank vault to dao vault
        check_eq!(token_prog_ai.key, &spl_token::ID, MangoErrorCode::Default)?;
        let signers_seeds = gen_signer_seeds(&mango_group.signer_nonce, mango_group_ai.key);
        invoke_transfer(
            token_prog_ai,
            bank_vault_ai,
            dao_vault_ai,
            signer_ai,
            &[&signers_seeds],
            settlement.to_num(),
        )?;

        // Decrement deposits on mango account
        checked_sub_net(
            root_bank_cache,
            &mut node_bank,
            &mut mango_account,
            QUOTE_INDEX,
            settlement,
        )
    }

    #[inline(never)]
    fn force_cancel_spot_orders(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        limit: u8,
    ) -> MangoResult<()> {
        const NUM_FIXED: usize = 19;
        let accounts = array_ref![accounts, 0, NUM_FIXED + MAX_PAIRS];
        let (fixed_ais, liqee_open_orders_ais) = array_refs![accounts, NUM_FIXED, MAX_PAIRS];

        let [
            mango_group_ai,         // read
            mango_cache_ai,         // read
            liqee_mango_account_ai, // write
            base_root_bank_ai,      // read
            base_node_bank_ai,      // write
            base_vault_ai,          // write
            quote_root_bank_ai,     // read
            quote_node_bank_ai,     // write
            quote_vault_ai,         // write

            spot_market_ai,         // write
            bids_ai,                // write
            asks_ai,                // write
            signer_ai,              // read
            dex_event_queue_ai,     // write
            dex_base_ai,            // write
            dex_quote_ai,           // write
            dex_signer_ai,          // read
            dex_prog_ai,            // read
            token_prog_ai,          // read
        ] = fixed_ais;

        // Check token program id
        check_eq!(token_prog_ai.key, &spl_token::ID, MangoErrorCode::InvalidProgramId)?;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        check_eq!(dex_prog_ai.key, &mango_group.dex_program_id, MangoErrorCode::InvalidProgramId)?;

        let mango_cache = MangoCache::load_checked(mango_cache_ai, program_id, &mango_group)?;
        let mut liqee_ma =
            MangoAccount::load_mut_checked(liqee_mango_account_ai, program_id, mango_group_ai.key)?;
        check!(!liqee_ma.is_bankrupt, MangoErrorCode::Bankrupt)?;
        liqee_ma.check_open_orders(&mango_group, liqee_open_orders_ais)?;

        let market_index = mango_group.find_spot_market_index(spot_market_ai.key).unwrap();
        check!(liqee_ma.in_margin_basket[market_index], MangoErrorCode::Default)?;

        check_eq!(
            &mango_group.tokens[market_index].root_bank,
            base_root_bank_ai.key,
            MangoErrorCode::InvalidRootBank
        )?;
        let base_root_bank = RootBank::load_checked(base_root_bank_ai, program_id)?;

        check!(
            base_root_bank.node_banks.contains(base_node_bank_ai.key),
            MangoErrorCode::InvalidNodeBank
        )?;
        let mut base_node_bank = NodeBank::load_mut_checked(base_node_bank_ai, program_id)?;
        check_eq!(&base_node_bank.vault, base_vault_ai.key, MangoErrorCode::InvalidVault)?;

        check_eq!(
            &mango_group.tokens[QUOTE_INDEX].root_bank,
            quote_root_bank_ai.key,
            MangoErrorCode::InvalidRootBank
        )?;
        let quote_root_bank = RootBank::load_checked(quote_root_bank_ai, program_id)?;

        check!(
            quote_root_bank.node_banks.contains(quote_node_bank_ai.key),
            MangoErrorCode::InvalidNodeBank
        )?;
        let mut quote_node_bank = NodeBank::load_mut_checked(quote_node_bank_ai, program_id)?;
        check_eq!(&quote_node_bank.vault, quote_vault_ai.key, MangoErrorCode::InvalidVault)?;

        let now_ts = Clock::get()?.unix_timestamp as u64;

        let liqee_active_assets = UserActiveAssets::new(&mango_group, &liqee_ma, vec![]);

        mango_cache.check_valid(&mango_group, &liqee_active_assets, now_ts)?;

        let mut health_cache = HealthCache::new(liqee_active_assets);
        health_cache.init_vals(&mango_group, &mango_cache, &liqee_ma, liqee_open_orders_ais)?;
        let init_health = health_cache.get_health(&mango_group, HealthType::Init);
        let maint_health = health_cache.get_health(&mango_group, HealthType::Maint);

        // Can only force cancel on an account already being liquidated
        if liqee_ma.being_liquidated {
            if init_health > ZERO_I80F48 {
                liqee_ma.being_liquidated = false;
                msg!("Account init_health above zero.");
                return Ok(());
            }
        } else if maint_health >= ZERO_I80F48 {
            return Err(throw_err!(MangoErrorCode::NotLiquidatable));
        } else {
            liqee_ma.being_liquidated = true;
        }

        // Cancel orders up to the limit
        let open_orders_ai = &liqee_open_orders_ais[market_index];
        let signers_seeds = gen_signer_seeds(&mango_group.signer_nonce, mango_group_ai.key);
        invoke_cancel_orders(
            open_orders_ai,
            dex_prog_ai,
            spot_market_ai,
            bids_ai,
            asks_ai,
            signer_ai,
            dex_event_queue_ai,
            &[&signers_seeds],
            limit,
        )?;

        let (pre_base, pre_quote) = {
            let open_orders = load_open_orders(open_orders_ai)?;
            (
                open_orders.native_coin_free,
                open_orders.native_pc_free + open_orders.referrer_rebates_accrued,
            )
        };

        if pre_base == 0 && pre_quote == 0 {
            return Ok(());
        }

        // Settle funds released by canceling open orders
        // TODO OPT add a new ForceSettleFunds to save compute in this instruction
        invoke_settle_funds(
            dex_prog_ai,
            spot_market_ai,
            open_orders_ai,
            signer_ai,
            dex_base_ai,
            dex_quote_ai,
            base_vault_ai,
            quote_vault_ai,
            dex_signer_ai,
            token_prog_ai,
            &[&signers_seeds],
        )?;

        let (post_base, post_quote) = {
            let open_orders = load_open_orders(open_orders_ai)?;
            liqee_ma.update_basket(market_index, &open_orders)?;
            (
                open_orders.native_coin_free,
                open_orders.native_pc_free + open_orders.referrer_rebates_accrued,
            )
        };

        check!(post_base <= pre_base, MangoErrorCode::Default)?;
        check!(post_quote <= pre_quote, MangoErrorCode::Default)?;

        // Update balances from settling funds
        let base_change = I80F48::from_num(pre_base - post_base);
        let quote_change = I80F48::from_num(pre_quote - post_quote);

        checked_add_net(
            &mango_cache.root_bank_cache[market_index],
            &mut base_node_bank,
            &mut liqee_ma,
            market_index,
            base_change,
        )?;
        checked_add_net(
            &mango_cache.root_bank_cache[QUOTE_INDEX],
            &mut quote_node_bank,
            &mut liqee_ma,
            QUOTE_INDEX,
            quote_change,
        )?;

        Ok(())
    }

    #[inline(never)]
    fn force_cancel_perp_orders(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        limit: u8,
    ) -> MangoResult<()> {
        const NUM_FIXED: usize = 6;
        let accounts = array_ref![accounts, 0, NUM_FIXED + MAX_PAIRS];
        let (fixed_ais, liqee_open_orders_ais) = array_refs![accounts, NUM_FIXED, MAX_PAIRS];

        let [
            mango_group_ai,         // read
            mango_cache_ai,         // read
            perp_market_ai,         // read
            bids_ai,                // write
            asks_ai,                // write
            liqee_mango_account_ai, // write
        ] = fixed_ais;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        let mango_cache = MangoCache::load_checked(mango_cache_ai, program_id, &mango_group)?;

        let mut liqee_ma =
            MangoAccount::load_mut_checked(liqee_mango_account_ai, program_id, mango_group_ai.key)?;
        check!(!liqee_ma.is_bankrupt, MangoErrorCode::Bankrupt)?;
        liqee_ma.check_open_orders(&mango_group, liqee_open_orders_ais)?;

        let perp_market = PerpMarket::load_checked(perp_market_ai, program_id, mango_group_ai.key)?;
        let market_index = mango_group.find_perp_market_index(perp_market_ai.key).unwrap();
        let perp_market_info = &mango_group.perp_markets[market_index];
        check!(!perp_market_info.is_empty(), MangoErrorCode::InvalidMarket)?;

        let now_ts = Clock::get()?.unix_timestamp as u64;

        let liqee_active_assets = UserActiveAssets::new(&mango_group, &liqee_ma, vec![]);

        mango_cache.check_valid(&mango_group, &liqee_active_assets, now_ts)?;

        let mut health_cache = HealthCache::new(liqee_active_assets);
        health_cache.init_vals(&mango_group, &mango_cache, &liqee_ma, liqee_open_orders_ais)?;
        let init_health = health_cache.get_health(&mango_group, HealthType::Init);
        let maint_health = health_cache.get_health(&mango_group, HealthType::Maint);

        if liqee_ma.being_liquidated {
            if init_health > ZERO_I80F48 {
                liqee_ma.being_liquidated = false;
                msg!("Account init_health above zero.");
                return Ok(());
            }
        } else if maint_health >= ZERO_I80F48 {
            msg!(
                "maint health {} init health {}",
                maint_health.to_num::<f64>(),
                init_health.to_num::<f64>()
            );
            return Err(throw_err!(MangoErrorCode::NotLiquidatable));
        } else {
            liqee_ma.being_liquidated = true;
        }

        let mut book = Book::load_checked(program_id, bids_ai, asks_ai, &perp_market)?;
        book.cancel_all(&mut liqee_ma, market_index, limit)
    }

    #[inline(never)]
    /// Liquidator takes some of borrows at token at `liab_index` and receives some deposits from
    /// the token at `asset_index`
    /// Requires: `liab_index != asset_index`
    fn liquidate_token_and_token(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        max_liab_transfer: I80F48,
    ) -> MangoResult<()> {
        // parameter checks
        check!(max_liab_transfer.is_positive(), MangoErrorCode::InvalidParam)?;

        const NUM_FIXED: usize = 9;
        let accounts = array_ref![accounts, 0, NUM_FIXED + 2 * MAX_PAIRS];
        let (fixed_ais, liqee_open_orders_ais, liqor_open_orders_ais) =
            array_refs![accounts, NUM_FIXED, MAX_PAIRS, MAX_PAIRS];

        let [
            mango_group_ai,         // read
            mango_cache_ai,         // read
            liqee_mango_account_ai, // write
            liqor_mango_account_ai, // write
            liqor_ai,               // read, signer
            asset_root_bank_ai,     // read
            asset_node_bank_ai,     // write
            liab_root_bank_ai,      // read
            liab_node_bank_ai,      // write
        ] = fixed_ais;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        let mango_cache = MangoCache::load_checked(mango_cache_ai, program_id, &mango_group)?;
        let mut liqee_ma =
            MangoAccount::load_mut_checked(liqee_mango_account_ai, program_id, mango_group_ai.key)?;
        check!(!liqee_ma.is_bankrupt, MangoErrorCode::Bankrupt)?;
        liqee_ma.check_open_orders(&mango_group, liqee_open_orders_ais)?;

        let mut liqor_ma =
            MangoAccount::load_mut_checked(liqor_mango_account_ai, program_id, mango_group_ai.key)?;
        check_eq!(liqor_ai.key, &liqor_ma.owner, MangoErrorCode::InvalidOwner)?;
        check!(liqor_ai.is_signer, MangoErrorCode::InvalidSignerKey)?;
        check!(!liqor_ma.is_bankrupt, MangoErrorCode::Bankrupt)?;
        liqor_ma.check_open_orders(&mango_group, liqor_open_orders_ais)?;

        let asset_root_bank = RootBank::load_checked(asset_root_bank_ai, program_id)?;
        let asset_index = mango_group.find_root_bank_index(asset_root_bank_ai.key).unwrap();
        let mut asset_node_bank = NodeBank::load_mut_checked(asset_node_bank_ai, program_id)?;
        check!(
            asset_root_bank.node_banks.contains(asset_node_bank_ai.key),
            MangoErrorCode::Default
        )?;

        let liab_root_bank = RootBank::load_checked(liab_root_bank_ai, program_id)?;
        let liab_index = mango_group.find_root_bank_index(liab_root_bank_ai.key).unwrap();
        let mut liab_node_bank = NodeBank::load_mut_checked(liab_node_bank_ai, program_id)?;
        check!(liab_root_bank.node_banks.contains(liab_node_bank_ai.key), MangoErrorCode::Default)?;
        check!(asset_index != liab_index, MangoErrorCode::InvalidParam)?;

        let now_ts = Clock::get()?.unix_timestamp as u64;
        let liqee_active_assets = UserActiveAssets::new(&mango_group, &liqee_ma, vec![]);
        let liqor_active_assets = UserActiveAssets::new(
            &mango_group,
            &liqor_ma,
            vec![(AssetType::Token, asset_index), (AssetType::Token, liab_index)],
        );

        mango_cache.check_valid(
            &mango_group,
            &UserActiveAssets::merge(&liqee_active_assets, &liqor_active_assets),
            now_ts,
        )?;

        // Make sure orders are cancelled for perps and check orders
        for i in 0..mango_group.num_oracles {
            if liqee_active_assets.perps[i] {
                check!(liqee_ma.perp_accounts[i].is_liquidatable(), MangoErrorCode::Default)?;
            }
        }

        let mut health_cache = HealthCache::new(liqee_active_assets);
        health_cache.init_vals(&mango_group, &mango_cache, &liqee_ma, liqee_open_orders_ais)?;
        let init_health = health_cache.get_health(&mango_group, HealthType::Init);
        let maint_health = health_cache.get_health(&mango_group, HealthType::Maint);

        if liqee_ma.being_liquidated {
            if init_health > ZERO_I80F48 {
                liqee_ma.being_liquidated = false;
                msg!("Account init_health above zero.");
                return Ok(());
            }
        } else if maint_health >= ZERO_I80F48 {
            return Err(throw_err!(MangoErrorCode::NotLiquidatable));
        } else {
            liqee_ma.being_liquidated = true;
        }

        check!(liqee_ma.deposits[asset_index].is_positive(), MangoErrorCode::Default)?;
        check!(liqee_ma.borrows[liab_index].is_positive(), MangoErrorCode::Default)?;

        let asset_bank = &mango_cache.root_bank_cache[asset_index];
        let liab_bank = &mango_cache.root_bank_cache[liab_index];

        let asset_price = mango_cache.get_price(asset_index);
        let liab_price = mango_cache.get_price(liab_index);

        let (asset_fee, init_asset_weight) = if asset_index == QUOTE_INDEX {
            (ONE_I80F48, ONE_I80F48)
        } else {
            let asset_info = &mango_group.spot_markets[asset_index];
            check!(!asset_info.is_empty(), MangoErrorCode::InvalidMarket)?;
            (ONE_I80F48 + asset_info.liquidation_fee, asset_info.init_asset_weight)
        };

        let (liab_fee, init_liab_weight) = if liab_index == QUOTE_INDEX {
            (ONE_I80F48, ONE_I80F48)
        } else {
            let liab_info = &mango_group.spot_markets[liab_index];
            check!(!liab_info.is_empty(), MangoErrorCode::InvalidMarket)?;
            (ONE_I80F48 - liab_info.liquidation_fee, liab_info.init_liab_weight)
        };

        // Max liab transferred to reach init_health == 0
        let deficit_max_liab: I80F48 = -init_health
            / (liab_price * (init_liab_weight - init_asset_weight * asset_fee / liab_fee));

        let native_deposits = liqee_ma.get_native_deposit(asset_bank, asset_index)?;
        let native_borrows = liqee_ma.get_native_borrow(liab_bank, liab_index)?;

        // Max liab transferred to reach asset_i == 0
        let asset_implied_liab_transfer =
            native_deposits * asset_price * liab_fee / (liab_price * asset_fee);
        let actual_liab_transfer = min(
            min(min(deficit_max_liab, native_borrows), max_liab_transfer),
            asset_implied_liab_transfer,
        );

        // Transfer into liqee to reduce liabilities
        checked_add_net(
            &liab_bank,
            &mut liab_node_bank,
            &mut liqee_ma,
            liab_index,
            actual_liab_transfer,
        )?; // TODO make sure deposits for this index is == 0

        // Transfer from liqor
        checked_sub_net(
            &liab_bank,
            &mut liab_node_bank,
            &mut liqor_ma,
            liab_index,
            actual_liab_transfer,
        )?;

        let asset_transfer =
            actual_liab_transfer * liab_price * asset_fee / (liab_fee * asset_price);

        // Transfer collater into liqor
        checked_add_net(
            &asset_bank,
            &mut asset_node_bank,
            &mut liqor_ma,
            asset_index,
            asset_transfer,
        )?;

        // Transfer collateral out of liqee
        checked_sub_net(
            &asset_bank,
            &mut asset_node_bank,
            &mut liqee_ma,
            asset_index,
            asset_transfer,
        )?;

        let mut liqor_health_cache = HealthCache::new(liqor_active_assets);
        liqor_health_cache.init_vals(
            &mango_group,
            &mango_cache,
            &liqor_ma,
            liqor_open_orders_ais,
        )?;
        let liqor_health = liqor_health_cache.get_health(&mango_group, HealthType::Init);
        check!(liqor_health >= ZERO_I80F48, MangoErrorCode::InsufficientFunds)?;

        // Update liqee's health where it may have changed
        for &i in &[asset_index, liab_index] {
            health_cache.update_token_val(
                &mango_group,
                &mango_cache,
                &liqee_ma,
                liqee_open_orders_ais,
                i,
            )?;
        }
        let liqee_maint_health = health_cache.get_health(&mango_group, HealthType::Maint);
        if liqee_maint_health < ZERO_I80F48 {
            liqee_ma.is_bankrupt =
                liqee_ma.check_enter_bankruptcy(&mango_group, liqee_open_orders_ais);
        } else {
            let liqee_init_health = health_cache.get_health(&mango_group, HealthType::Init);
            if liqee_init_health >= ZERO_I80F48 {
                liqee_ma.being_liquidated = false;
            }
        }

        msg!(
            "liquidate_token_and_token details: {{ \
            \"asset_index\": {}, \
            \"liab_index\": {}, \
            \"asset_transfer\": {}, \
            \"liab_transfer\": {}, \
            \"asset_price\": {}, \
            \"liab_price\": {}, \
            \"bankruptcy\": {}
        }}",
            asset_index,
            liab_index,
            asset_transfer.to_num::<f64>(),
            actual_liab_transfer.to_num::<f64>(),
            asset_price.to_num::<f64>(),
            liab_price.to_num::<f64>(),
            liqee_ma.is_bankrupt
        );

        Ok(())
    }

    #[inline(never)]
    /// swap tokens for perp quote position only and only if the base position in that market is 0
    fn liquidate_token_and_perp(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        asset_type: AssetType,
        asset_index: usize,
        liab_type: AssetType,
        liab_index: usize,
        max_liab_transfer: I80F48,
    ) -> MangoResult<()> {
        check!(max_liab_transfer.is_positive(), MangoErrorCode::InvalidParam)?;
        check!(asset_type != liab_type, MangoErrorCode::InvalidParam)?;

        const NUM_FIXED: usize = 7;
        let accounts = array_ref![accounts, 0, NUM_FIXED + 2 * MAX_PAIRS];
        let (fixed_ais, liqee_open_orders_ais, liqor_open_orders_ais) =
            array_refs![accounts, NUM_FIXED, MAX_PAIRS, MAX_PAIRS];

        let [
            mango_group_ai,         // read
            mango_cache_ai,         // read
            liqee_mango_account_ai, // write
            liqor_mango_account_ai, // write
            liqor_ai,               // read, signer
            root_bank_ai,           // read
            node_bank_ai,           // write
        ] = fixed_ais;
        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        let mango_cache = MangoCache::load_checked(mango_cache_ai, program_id, &mango_group)?;
        let mut liqee_ma =
            MangoAccount::load_mut_checked(liqee_mango_account_ai, program_id, mango_group_ai.key)?;
        check!(!liqee_ma.is_bankrupt, MangoErrorCode::Bankrupt)?;
        liqee_ma.check_open_orders(&mango_group, liqee_open_orders_ais)?;

        let mut liqor_ma =
            MangoAccount::load_mut_checked(liqor_mango_account_ai, program_id, mango_group_ai.key)?;
        check_eq!(liqor_ai.key, &liqor_ma.owner, MangoErrorCode::InvalidOwner)?;
        check!(liqor_ai.is_signer, MangoErrorCode::InvalidSignerKey)?;
        check!(!liqor_ma.is_bankrupt, MangoErrorCode::Bankrupt)?;
        liqor_ma.check_open_orders(&mango_group, liqor_open_orders_ais)?;

        let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
        let mut node_bank = NodeBank::load_mut_checked(node_bank_ai, program_id)?;
        check!(root_bank.node_banks.contains(node_bank_ai.key), MangoErrorCode::InvalidNodeBank)?;

        let now_ts = Clock::get()?.unix_timestamp as u64;
        let liqee_active_assets = UserActiveAssets::new(&mango_group, &liqee_ma, vec![]);
        let liqor_active_assets = UserActiveAssets::new(
            &mango_group,
            &liqor_ma,
            vec![(asset_type, asset_index), (liab_type, liab_index)],
        );

        mango_cache.check_valid(
            &mango_group,
            &UserActiveAssets::merge(&liqee_active_assets, &liqor_active_assets),
            now_ts,
        )?;

        // Make sure orders are cancelled for perps and check orders
        for i in 0..mango_group.num_oracles {
            if liqee_active_assets.perps[i] {
                check!(liqee_ma.perp_accounts[i].is_liquidatable(), MangoErrorCode::Default)?;
            }
        }

        let mut health_cache = HealthCache::new(liqee_active_assets);
        health_cache.init_vals(&mango_group, &mango_cache, &liqee_ma, liqee_open_orders_ais)?;
        let init_health = health_cache.get_health(&mango_group, HealthType::Init);
        let maint_health = health_cache.get_health(&mango_group, HealthType::Maint);

        if liqee_ma.being_liquidated {
            if init_health > ZERO_I80F48 {
                liqee_ma.being_liquidated = false;
                msg!("Account init_health above zero.");
                return Ok(());
            }
        } else if maint_health >= ZERO_I80F48 {
            return Err(throw_err!(MangoErrorCode::NotLiquidatable));
        } else {
            liqee_ma.being_liquidated = true;
        }

        let asset_price: I80F48;
        let liab_price: I80F48;
        let asset_transfer: I80F48;
        let actual_liab_transfer: I80F48;
        if asset_type == AssetType::Token {
            // we know asset_type != liab_type
            asset_price = mango_cache.get_price(asset_index);
            liab_price = ONE_I80F48;
            let bank_cache = &mango_cache.root_bank_cache[asset_index];
            check!(liqee_ma.deposits[asset_index].is_positive(), MangoErrorCode::Default)?;
            check!(liab_index != QUOTE_INDEX, MangoErrorCode::Default)?;
            check!(
                mango_group.find_root_bank_index(root_bank_ai.key).unwrap() == asset_index,
                MangoErrorCode::InvalidRootBank
            )?;
            let native_borrows = -liqee_ma.perp_accounts[liab_index].quote_position;
            check!(liqee_ma.perp_accounts[liab_index].base_position == 0, MangoErrorCode::Default)?;
            check!(native_borrows.is_positive(), MangoErrorCode::Default)?;

            let (asset_fee, init_asset_weight) = if asset_index == QUOTE_INDEX {
                (ONE_I80F48, ONE_I80F48)
            } else {
                let asset_info = &mango_group.spot_markets[asset_index];
                check!(!asset_info.is_empty(), MangoErrorCode::InvalidMarket)?;
                (ONE_I80F48 + asset_info.liquidation_fee, asset_info.init_asset_weight)
            };

            let liab_info = &mango_group.perp_markets[liab_index];
            check!(!liab_info.is_empty(), MangoErrorCode::InvalidMarket)?;

            let (liab_fee, init_liab_weight) = (ONE_I80F48, ONE_I80F48);

            let native_deposits = liqee_ma.get_native_deposit(bank_cache, asset_index)?;

            // Max liab transferred to reach init_health == 0
            let deficit_max_liab = if asset_index == QUOTE_INDEX {
                native_deposits
            } else {
                -init_health
                    / (liab_price * (init_liab_weight - init_asset_weight * asset_fee / liab_fee))
            };

            // Max liab transferred to reach asset_i == 0
            let asset_implied_liab_transfer =
                native_deposits * asset_price * liab_fee / (liab_price * asset_fee);
            actual_liab_transfer = min(
                min(min(deficit_max_liab, native_borrows), max_liab_transfer),
                asset_implied_liab_transfer,
            );

            liqee_ma.perp_accounts[liab_index].transfer_quote_position(
                &mut liqor_ma.perp_accounts[liab_index],
                -actual_liab_transfer,
            );

            asset_transfer =
                actual_liab_transfer * liab_price * asset_fee / (liab_fee * asset_price);

            // Transfer collater into liqor
            checked_add_net(
                bank_cache,
                &mut node_bank,
                &mut liqor_ma,
                asset_index,
                asset_transfer,
            )?;

            // Transfer collateral out of liqee
            checked_sub_net(
                bank_cache,
                &mut node_bank,
                &mut liqee_ma,
                asset_index,
                asset_transfer,
            )?;

            health_cache.update_token_val(
                &mango_group,
                &mango_cache,
                &liqee_ma,
                liqee_open_orders_ais,
                asset_index,
            )?;

            health_cache.update_perp_val(&mango_group, &mango_cache, &liqee_ma, liab_index)?;
        } else {
            asset_price = ONE_I80F48;
            liab_price = mango_cache.get_price(liab_index);
            check!(
                mango_group.find_root_bank_index(root_bank_ai.key).unwrap() == liab_index,
                MangoErrorCode::InvalidRootBank
            )?;

            check!(liqee_ma.borrows[liab_index].is_positive(), MangoErrorCode::Default)?;
            check!(asset_index != QUOTE_INDEX, MangoErrorCode::Default)?;

            check!(
                liqee_ma.perp_accounts[asset_index].base_position == 0,
                MangoErrorCode::Default
            )?;
            let native_deposits = liqee_ma.perp_accounts[asset_index].quote_position;
            check!(native_deposits.is_positive(), MangoErrorCode::Default)?;

            let bank_cache = &mango_cache.root_bank_cache[liab_index];
            let (asset_fee, init_asset_weight) = (ONE_I80F48, ONE_I80F48);
            let (liab_fee, init_liab_weight) = if liab_index == QUOTE_INDEX {
                (ONE_I80F48, ONE_I80F48)
            } else {
                let liab_info = &mango_group.spot_markets[liab_index];
                check!(!liab_info.is_empty(), MangoErrorCode::InvalidMarket)?;
                (ONE_I80F48 + liab_info.liquidation_fee, liab_info.init_asset_weight)
            };

            let native_borrows = liqee_ma.get_native_borrow(bank_cache, liab_index)?;

            // Max liab transferred to reach init_health == 0
            let deficit_max_liab: I80F48 = -init_health
                / (liab_price * (init_liab_weight - init_asset_weight * asset_fee / liab_fee));

            // Max liab transferred to reach asset_i == 0
            let asset_implied_liab_transfer =
                native_deposits * asset_price * liab_fee / (liab_price * asset_fee);
            actual_liab_transfer = min(
                min(min(deficit_max_liab, native_borrows), max_liab_transfer),
                asset_implied_liab_transfer,
            );

            asset_transfer =
                actual_liab_transfer * liab_price * asset_fee / (liab_fee * asset_price);

            // Transfer collater into liqor
            checked_add_net(
                bank_cache,
                &mut node_bank,
                &mut liqor_ma,
                liab_index,
                actual_liab_transfer,
            )?;

            // Transfer collateral out of liqee
            checked_sub_net(
                bank_cache,
                &mut node_bank,
                &mut liqee_ma,
                liab_index,
                actual_liab_transfer,
            )?;

            liqee_ma.perp_accounts[asset_index]
                .transfer_quote_position(&mut liqor_ma.perp_accounts[asset_index], asset_transfer);

            health_cache.update_token_val(
                &mango_group,
                &mango_cache,
                &liqee_ma,
                liqee_open_orders_ais,
                liab_index,
            )?;

            health_cache.update_perp_val(&mango_group, &mango_cache, &liqee_ma, asset_index)?;
        }

        let mut liqor_health_cache = HealthCache::new(liqor_active_assets);
        liqor_health_cache.init_vals(
            &mango_group,
            &mango_cache,
            &liqor_ma,
            liqor_open_orders_ais,
        )?;
        let liqor_health = liqor_health_cache.get_health(&mango_group, HealthType::Init);
        check!(liqor_health >= ZERO_I80F48, MangoErrorCode::InsufficientFunds)?;

        let liqee_maint_health = health_cache.get_health(&mango_group, HealthType::Maint);
        if liqee_maint_health < ZERO_I80F48 {
            liqee_ma.is_bankrupt =
                liqee_ma.check_enter_bankruptcy(&mango_group, liqee_open_orders_ais);
        } else {
            let liqee_init_health = health_cache.get_health(&mango_group, HealthType::Init);
            liqee_ma.being_liquidated = liqee_init_health < ZERO_I80F48;
        }

        msg!(
            "liquidate_token_and_perp details: {{ \
            \"asset_index\": {}, \
            \"liab_index\": {}, \
            \"asset_type\": \"{:?}\", \
            \"liab_type\": \"{:?}\", \
            \"asset_price\": {}, \
            \"liab_price\": {}, \
            \"asset_transfer\": {}, \
            \"actual_liab_transfer\": {}
        }}",
            asset_index,
            liab_index,
            asset_type,
            liab_type,
            asset_price.to_num::<f64>(),
            liab_price.to_num::<f64>(),
            asset_transfer.to_num::<f64>(),
            actual_liab_transfer.to_num::<f64>()
        );

        Ok(())
    }

    #[inline(never)]
    /// Reduce some of the base position in exchange for quote position in this market
    /// Transfer will not exceed abs(base_position)
    /// Example:
    ///     BTC/USD price 9.4k
    ///     liquidation_fee = 0.025
    ///     liqee initial
    ///         USDC deposit 10k
    ///         BTC-PERP base_position = 10
    ///         BTC-PERP quote_position = -100k
    ///         maint_health = -700
    ///         init_health = -5400
    ///     liqee after liquidate_perp_market
    ///         USDC deposit 10k
    ///         BTC-PERP base_position = 2.3404
    ///         BTC-PERP quote_position = -29799.766
    ///         init_health = 0.018
    ///     liqor after liquidate_perp_market
    ///         BTC-PERP base_position = 7.6596
    ///         BTC-PERP quote_position = -70200.234
    fn liquidate_perp_market(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        base_transfer_request: i64,
    ) -> MangoResult<()> {
        // TODO - make sure sum of all quote positions + funding in system == 0
        // TODO - find a way to send in open orders accounts
        // liqor passes in his own account and the liqee mango account
        // position is transfered to the liqor at favorable rate
        check!(base_transfer_request != 0, MangoErrorCode::InvalidParam)?;
        const NUM_FIXED: usize = 7;
        let accounts = array_ref![accounts, 0, NUM_FIXED + 2 * MAX_PAIRS];
        let (fixed_ais, liqee_open_orders_ais, liqor_open_orders_ais) =
            array_refs![accounts, NUM_FIXED, MAX_PAIRS, MAX_PAIRS];

        let [
            mango_group_ai,         // read
            mango_cache_ai,         // read
            perp_market_ai,         // write
            event_queue_ai,         // write
            liqee_mango_account_ai, // write
            liqor_mango_account_ai, // write
            liqor_ai,               // read, signer
        ] = fixed_ais;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        let mango_cache = MangoCache::load_checked(mango_cache_ai, program_id, &mango_group)?;

        let mut liqee_ma =
            MangoAccount::load_mut_checked(liqee_mango_account_ai, program_id, mango_group_ai.key)?;
        check!(!liqee_ma.is_bankrupt, MangoErrorCode::Bankrupt)?;
        liqee_ma.check_open_orders(&mango_group, liqee_open_orders_ais)?;

        let mut liqor_ma =
            MangoAccount::load_mut_checked(liqor_mango_account_ai, program_id, mango_group_ai.key)?;
        check!(!liqor_ma.is_bankrupt, MangoErrorCode::Bankrupt)?;
        check_eq!(liqor_ai.key, &liqor_ma.owner, MangoErrorCode::InvalidOwner)?;
        check!(liqor_ai.is_signer, MangoErrorCode::InvalidSignerKey)?;
        liqor_ma.check_open_orders(&mango_group, liqor_open_orders_ais)?;

        let mut perp_market =
            PerpMarket::load_mut_checked(perp_market_ai, program_id, mango_group_ai.key)?;
        let market_index = mango_group.find_perp_market_index(perp_market_ai.key).unwrap();
        let perp_market_info = &mango_group.perp_markets[market_index];
        check!(!perp_market_info.is_empty(), MangoErrorCode::InvalidMarket)?;
        let mut event_queue: EventQueue =
            EventQueue::load_mut_checked(event_queue_ai, program_id, &perp_market)?;

        // Move funding into quote position. Not necessary to adjust funding settled after funding is moved
        let cache = &mango_cache.perp_market_cache[market_index];

        let now_ts = Clock::get()?.unix_timestamp as u64;
        let liqee_active_assets = UserActiveAssets::new(&mango_group, &liqee_ma, vec![]);
        let liqor_active_assets =
            UserActiveAssets::new(&mango_group, &liqor_ma, vec![(AssetType::Perp, market_index)]);

        mango_cache.check_valid(
            &mango_group,
            &UserActiveAssets::merge(&liqee_active_assets, &liqor_active_assets),
            now_ts,
        )?;
        liqee_ma.perp_accounts[market_index].settle_funding(cache);
        liqor_ma.perp_accounts[market_index].settle_funding(cache);

        // Make sure orders are cancelled for perps before liquidation
        for i in 0..mango_group.num_oracles {
            if liqee_active_assets.perps[i] {
                check!(liqee_ma.perp_accounts[i].is_liquidatable(), MangoErrorCode::Default)?;
            }
        }

        let mut health_cache = HealthCache::new(liqee_active_assets);
        health_cache.init_vals(&mango_group, &mango_cache, &liqee_ma, liqee_open_orders_ais)?;
        let init_health = health_cache.get_health(&mango_group, HealthType::Init);
        let maint_health = health_cache.get_health(&mango_group, HealthType::Maint);

        if liqee_ma.being_liquidated {
            if init_health > ZERO_I80F48 {
                liqee_ma.being_liquidated = false;
                msg!("Account init_health above zero.");
                return Ok(());
            }
        } else if maint_health >= ZERO_I80F48 {
            return Err(throw_err!(MangoErrorCode::NotLiquidatable));
        } else {
            liqee_ma.being_liquidated = true;
        }

        // TODO - what happens if base position and quote position have same sign?
        // TODO - what if base position is 0 but quote is negative. Perhaps settle that pnl first?

        let liqee_perp_account = &mut liqee_ma.perp_accounts[market_index];
        let liqor_perp_account = &mut liqor_ma.perp_accounts[market_index];

        let price = mango_cache.price_cache[market_index].price;
        let (base_transfer, quote_transfer) = if liqee_perp_account.base_position > 0 {
            check!(base_transfer_request > 0, MangoErrorCode::InvalidParam)?;

            // TODO - verify this calculation is accurate
            let max_transfer: I80F48 = -init_health
                / (price
                    * (ONE_I80F48
                        - perp_market_info.init_asset_weight
                        - perp_market_info.liquidation_fee));
            let max_transfer: i64 = max_transfer.checked_ceil().unwrap().to_num();

            let base_transfer =
                max_transfer.min(base_transfer_request).min(liqee_perp_account.base_position);

            let quote_transfer = I80F48::from_num(-base_transfer * perp_market_info.base_lot_size)
                * price
                * (ONE_I80F48 - perp_market_info.liquidation_fee);

            (base_transfer, quote_transfer)
        } else {
            // We know it liqee_perp_account.base_position < 0
            check!(base_transfer_request < 0, MangoErrorCode::InvalidParam)?;

            // TODO verify calculations are accurate
            let max_transfer: I80F48 = -init_health
                / (price
                    * (ONE_I80F48 - perp_market_info.init_liab_weight
                        + perp_market_info.liquidation_fee));
            let max_transfer: i64 = max_transfer.checked_floor().unwrap().to_num();

            let base_transfer =
                max_transfer.max(base_transfer_request).max(liqee_perp_account.base_position);
            let quote_transfer = I80F48::from_num(-base_transfer * perp_market_info.base_lot_size)
                * price
                * (ONE_I80F48 + perp_market_info.liquidation_fee);

            (base_transfer, quote_transfer)
        };

        liqee_perp_account.change_base_position(&mut perp_market, -base_transfer);
        liqor_perp_account.change_base_position(&mut perp_market, base_transfer);

        liqee_perp_account.transfer_quote_position(liqor_perp_account, quote_transfer);

        // Log this to EventQueue
        let liquidate_event = LiquidateEvent::new(
            now_ts,
            event_queue.header.seq_num,
            *liqee_mango_account_ai.key,
            *liqor_mango_account_ai.key,
            price,
            base_transfer,
            perp_market_info.liquidation_fee,
        );
        event_queue.push_back(cast(liquidate_event)).unwrap();

        // Calculate the health of liqor and see if liqor is still valid
        let mut liqor_health_cache = HealthCache::new(liqor_active_assets);
        liqor_health_cache.init_vals(
            &mango_group,
            &mango_cache,
            &liqor_ma,
            liqor_open_orders_ais,
        )?;
        let liqor_health = liqor_health_cache.get_health(&mango_group, HealthType::Init);
        check!(liqor_health >= ZERO_I80F48, MangoErrorCode::InsufficientFunds)?;

        health_cache.update_perp_val(&mango_group, &mango_cache, &liqee_ma, market_index)?;
        let liqee_maint_health = health_cache.get_health(&mango_group, HealthType::Maint);
        if liqee_maint_health < ZERO_I80F48 {
            liqee_ma.is_bankrupt =
                liqee_ma.check_enter_bankruptcy(&mango_group, liqee_open_orders_ais);
        } else {
            let liqee_init_health = health_cache.get_health(&mango_group, HealthType::Init);
            liqee_ma.being_liquidated = liqee_init_health < ZERO_I80F48;
        }

        // TODO make this more efficient
        msg!(
            "liquidate_perp_market: {{ market_index: {}, base_transfer: {}, quote_transfer: {}, bankruptcy: {} }}",
            market_index,
            base_transfer,
            quote_transfer.to_num::<f64>(),
            liqee_ma.is_bankrupt,
        );
        Ok(())
    }

    #[inline(never)]
    /// Claim insurance fund and then socialize loss
    fn resolve_perp_bankruptcy(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        liab_index: usize,
        max_liab_transfer: I80F48,
    ) -> MangoResult<()> {
        // First check the account is bankrupt
        // Determine the value of the liab transfer
        // Check if insurance fund has enough (given the fees)
        // If insurance fund does not have enough, start the socialize loss function

        // TODO - since liquidation fee is 0 for USDC, what's the incentive for someone to call this?
        //  just add 1bp fee

        // Do parameter checks
        check!(liab_index < QUOTE_INDEX, MangoErrorCode::InvalidParam)?;
        check!(max_liab_transfer.is_positive(), MangoErrorCode::InvalidParam)?;

        const NUM_FIXED: usize = 12;
        let accounts = array_ref![accounts, 0, NUM_FIXED + MAX_PAIRS];
        let (fixed_ais, liqor_open_orders_ais) = array_refs![accounts, NUM_FIXED, MAX_PAIRS];

        let [
            mango_group_ai,         // read
            mango_cache_ai,         // write
            liqee_mango_account_ai, // write
            liqor_mango_account_ai, // write
            liqor_ai,               // read, signer
            root_bank_ai,           // read
            node_bank_ai,           // write
            vault_ai,               // write
            dao_vault_ai,           // write
            signer_ai,              // read
            perp_market_ai,         // write
            token_prog_ai,          // read
        ] = fixed_ais;
        check_eq!(token_prog_ai.key, &spl_token::ID, MangoErrorCode::InvalidProgramId)?;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        let mut mango_cache =
            MangoCache::load_mut_checked(mango_cache_ai, program_id, &mango_group)?;
        let mut liqee_ma =
            MangoAccount::load_mut_checked(liqee_mango_account_ai, program_id, mango_group_ai.key)?;
        check!(liqee_ma.is_bankrupt, MangoErrorCode::Default)?;

        let mut liqor_ma =
            MangoAccount::load_mut_checked(liqor_mango_account_ai, program_id, mango_group_ai.key)?;
        check_eq!(liqor_ai.key, &liqor_ma.owner, MangoErrorCode::InvalidOwner)?;
        check!(liqor_ai.is_signer, MangoErrorCode::InvalidSignerKey)?;
        check!(!liqor_ma.is_bankrupt, MangoErrorCode::Bankrupt)?;
        liqor_ma.check_open_orders(&mango_group, liqor_open_orders_ais)?;

        let root_bank = RootBank::load_checked(root_bank_ai, program_id)?;
        check!(
            &mango_group.tokens[QUOTE_INDEX].root_bank == root_bank_ai.key,
            MangoErrorCode::InvalidRootBank
        )?;

        check!(root_bank.node_banks.contains(node_bank_ai.key), MangoErrorCode::InvalidNodeBank)?;
        let mut node_bank = NodeBank::load_mut_checked(node_bank_ai, program_id)?;
        check!(vault_ai.key == &node_bank.vault, MangoErrorCode::InvalidVault)?;

        let now_ts = Clock::get()?.unix_timestamp as u64;
        let liqor_active_assets =
            UserActiveAssets::new(&mango_group, &liqor_ma, vec![(AssetType::Perp, liab_index)]);

        mango_cache.check_valid(&mango_group, &liqor_active_assets, now_ts)?;

        check!(dao_vault_ai.key == &mango_group.dao_vault, MangoErrorCode::InvalidVault)?;
        let dao_vault = Account::unpack(&dao_vault_ai.try_borrow_data()?)?;

        let bank_cache = &mango_cache.root_bank_cache[QUOTE_INDEX];
        let quote_pos = liqee_ma.perp_accounts[liab_index].quote_position;
        check!(quote_pos.is_negative(), MangoErrorCode::Default)?;

        let liab_transfer_u64 = max_liab_transfer
            .min(-quote_pos) // minimum of what liqor wants and what liqee has
            .checked_ceil() // round up and convert to native quote token
            .unwrap()
            .to_num::<u64>()
            .min(dao_vault.amount); // take min of what ins. fund has

        if liab_transfer_u64 != 0 {
            let signers_seeds = gen_signer_seeds(&mango_group.signer_nonce, mango_group_ai.key);
            invoke_transfer(
                token_prog_ai,
                dao_vault_ai,
                vault_ai,
                signer_ai,
                &[&signers_seeds],
                liab_transfer_u64,
            )?;
            let liab_transfer = I80F48::from_num(liab_transfer_u64);
            liqee_ma.perp_accounts[liab_index]
                .transfer_quote_position(&mut liqor_ma.perp_accounts[liab_index], -liab_transfer);
            checked_add_net(bank_cache, &mut node_bank, &mut liqor_ma, QUOTE_INDEX, liab_transfer)?;

            // Make sure liqor is above init cond.
            let mut liqor_health_cache = HealthCache::new(liqor_active_assets);
            liqor_health_cache.init_vals(
                &mango_group,
                &mango_cache,
                &liqor_ma,
                liqor_open_orders_ais,
            )?;

            let liqor_health = liqor_health_cache.get_health(&mango_group, HealthType::Init);
            check!(liqor_health >= ZERO_I80F48, MangoErrorCode::InsufficientFunds)?;
            msg!(
                "perp_bankruptcy: {{ liab_index: {}, dao_transfer:{} }}",
                liab_index,
                liab_transfer_u64
            );
        }

        let quote_position = liqee_ma.perp_accounts[liab_index].quote_position;
        // If we transferred everything out of dao_vault, dao vault is empty
        // and if quote position is still negative
        if liab_transfer_u64 == dao_vault.amount && quote_position.is_negative() {
            // insurance fund empty so socialize loss
            check!(
                &mango_group.perp_markets[liab_index].perp_market == perp_market_ai.key,
                MangoErrorCode::InvalidMarket
            )?;
            let mut perp_market =
                PerpMarket::load_mut_checked(perp_market_ai, program_id, mango_group_ai.key)?;

            // TODO - log this
            perp_market.socialize_loss(
                &mut liqee_ma.perp_accounts[liab_index],
                &mut mango_cache.perp_market_cache[liab_index],
            )?;
            msg!(
                "perp_socialized_loss: {{ liab_index: {}, socialized_loss:{} }}",
                liab_index,
                (quote_position / (I80F48::from_num(perp_market.open_interest))).to_num::<f64>()
            );
        }

        liqee_ma.is_bankrupt = !liqee_ma.check_exit_bankruptcy(&mango_group);

        Ok(())
    }

    #[inline(never)]
    /// Claim insurance fund and then socialize loss
    fn resolve_token_bankruptcy(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        max_liab_transfer: I80F48, // in native token terms
    ) -> MangoResult<()> {
        // First check the account is bankrupt
        // Determine the value of the liab transfer
        // Check if insurance fund has enough (given the fees)
        // If insurance fund does not have enough, start the socialize loss function
        check!(max_liab_transfer.is_positive(), MangoErrorCode::Default)?;

        const NUM_FIXED: usize = 13;
        let accounts = array_ref![accounts, 0, NUM_FIXED + MAX_PAIRS + MAX_NODE_BANKS];
        let (
            fixed_ais,
            liqor_open_orders_ais, // read
            liab_node_bank_ais,    // write
        ) = array_refs![accounts, NUM_FIXED, MAX_PAIRS, MAX_NODE_BANKS];

        let [
            mango_group_ai,         // read
            mango_cache_ai,         // write
            liqee_mango_account_ai, // write
            liqor_mango_account_ai, // write
            liqor_ai,               // read, signer
            quote_root_bank_ai,     // read
            quote_node_bank_ai,     // write
            quote_vault_ai,         // write
            dao_vault_ai,           // write
            signer_ai,              // read
            liab_root_bank_ai,      // write
            liab_node_bank_ai,      // write
            token_prog_ai,          // read
        ] = fixed_ais;
        check_eq!(token_prog_ai.key, &spl_token::ID, MangoErrorCode::InvalidProgramId)?;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        let mut mango_cache =
            MangoCache::load_mut_checked(mango_cache_ai, program_id, &mango_group)?;

        // Load the liqee's mango account
        let mut liqee_ma =
            MangoAccount::load_mut_checked(liqee_mango_account_ai, program_id, mango_group_ai.key)?;
        check!(liqee_ma.is_bankrupt, MangoErrorCode::Default)?;

        // Load the liqor's mango account
        let mut liqor_ma =
            MangoAccount::load_mut_checked(liqor_mango_account_ai, program_id, mango_group_ai.key)?;
        check_eq!(liqor_ai.key, &liqor_ma.owner, MangoErrorCode::InvalidOwner)?;
        check!(liqor_ai.is_signer, MangoErrorCode::InvalidSignerKey)?;
        check!(!liqor_ma.is_bankrupt, MangoErrorCode::Bankrupt)?;
        liqor_ma.check_open_orders(&mango_group, liqor_open_orders_ais)?;

        // Load the bank for liab token
        let mut liab_root_bank = RootBank::load_mut_checked(liab_root_bank_ai, program_id)?;
        let liab_index = mango_group.find_root_bank_index(liab_root_bank_ai.key).unwrap();
        let mut liab_node_bank = NodeBank::load_mut_checked(liab_node_bank_ai, program_id)?;
        check!(liab_root_bank.node_banks.contains(liab_node_bank_ai.key), MangoErrorCode::Default)?;

        let now_ts = Clock::get()?.unix_timestamp as u64;
        let liqor_active_assets =
            UserActiveAssets::new(&mango_group, &liqor_ma, vec![(AssetType::Token, liab_index)]);

        mango_cache.check_valid(&mango_group, &liqor_active_assets, now_ts)?;

        // Load the dao vault (insurance fund)
        check!(dao_vault_ai.key == &mango_group.dao_vault, MangoErrorCode::InvalidVault)?;
        let dao_vault = Account::unpack(&dao_vault_ai.try_borrow_data()?)?;

        // Make sure there actually exist liabs here
        check!(liqee_ma.borrows[liab_index].is_positive(), MangoErrorCode::Default)?;
        let liab_price = mango_cache.get_price(liab_index);
        let liab_fee = if liab_index == QUOTE_INDEX {
            ONE_I80F48
        } else {
            let liab_info = &mango_group.spot_markets[liab_index];
            ONE_I80F48 - liab_info.liquidation_fee
        };

        let liab_bank_cache = &mango_cache.root_bank_cache[liab_index];
        let native_borrows = liqee_ma.get_native_borrow(liab_bank_cache, liab_index)?;

        let insured_liabs = I80F48::from_num(dao_vault.amount) * liab_fee / liab_price;
        let liab_transfer = max_liab_transfer.min(native_borrows).min(insured_liabs);

        let dao_transfer = (liab_transfer * liab_price / liab_fee)
            .checked_ceil()
            .unwrap()
            .to_num::<u64>()
            .min(dao_vault.amount);

        if dao_transfer != 0 {
            check!(signer_ai.key == &mango_group.signer_key, MangoErrorCode::InvalidSignerKey)?;
            let signers_seeds = gen_signer_seeds(&mango_group.signer_nonce, mango_group_ai.key);
            invoke_transfer(
                token_prog_ai,
                dao_vault_ai,
                quote_vault_ai,
                signer_ai,
                &[&signers_seeds],
                dao_transfer,
            )?;
            let liab_transfer = I80F48::from_num(dao_transfer) * liab_fee / liab_price;

            if liab_index == QUOTE_INDEX {
                checked_add_net(
                    &mango_cache.root_bank_cache[QUOTE_INDEX],
                    &mut liab_node_bank,
                    &mut liqor_ma,
                    QUOTE_INDEX,
                    I80F48::from_num(dao_transfer),
                )?;
            } else {
                // Load the bank for quote token
                let quote_root_bank = RootBank::load_checked(quote_root_bank_ai, program_id)?;
                check!(
                    &mango_group.tokens[QUOTE_INDEX].root_bank == quote_root_bank_ai.key,
                    MangoErrorCode::InvalidRootBank
                )?;
                let mut quote_node_bank =
                    NodeBank::load_mut_checked(quote_node_bank_ai, program_id)?;
                check!(
                    quote_root_bank.node_banks.contains(quote_node_bank_ai.key),
                    MangoErrorCode::InvalidNodeBank
                )?;

                checked_add_net(
                    &mango_cache.root_bank_cache[QUOTE_INDEX],
                    &mut quote_node_bank,
                    &mut liqor_ma,
                    QUOTE_INDEX,
                    I80F48::from_num(dao_transfer),
                )?;
            }

            checked_add_net(
                liab_bank_cache,
                &mut liab_node_bank,
                &mut liqee_ma,
                liab_index,
                liab_transfer,
            )?;
            checked_sub_net(
                liab_bank_cache,
                &mut liab_node_bank,
                &mut liqor_ma,
                liab_index,
                liab_transfer,
            )?;

            // Make sure liqor is above init health
            let mut liqor_health_cache = HealthCache::new(liqor_active_assets);
            liqor_health_cache.init_vals(
                &mango_group,
                &mango_cache,
                &liqor_ma,
                liqor_open_orders_ais,
            )?;
            let liqor_health = liqor_health_cache.get_health(&mango_group, HealthType::Init);
            check!(liqor_health >= ZERO_I80F48, MangoErrorCode::InsufficientFunds)?;
            msg!(
                "token_bankruptcy details: {{ \"liab_index\": {}, \"dao_transfer\": {} }}",
                liab_index,
                dao_transfer
            );
        }

        if dao_transfer == dao_vault.amount && liqee_ma.borrows[liab_index].is_positive() {
            // insurance fund empty so socialize loss
            let native_borrows = liqee_ma.get_native_borrow(liab_bank_cache, liab_index)?;
            liab_root_bank.socialize_loss(
                program_id,
                liab_index,
                &mut mango_cache,
                &mut liqee_ma,
                liab_node_bank_ais,
                &mut liab_node_bank,
                liab_node_bank_ai.key,
            )?;
            msg!(
                "token_socialized_loss details: {{ \"liab_index\": {}, \"native_borrows\":{} }}",
                liab_index,
                native_borrows.to_num::<f64>()
            );
        }

        liqee_ma.is_bankrupt = !liqee_ma.check_exit_bankruptcy(&mango_group);

        Ok(())
    }

    #[inline(never)]
    /// *** Keeper Related Instructions ***
    /// Update the deposit and borrow index on a passed in RootBank
    fn update_root_bank(program_id: &Pubkey, accounts: &[AccountInfo]) -> MangoResult<()> {
        const NUM_FIXED: usize = 2;
        let (fixed_accounts, node_bank_ais) = array_refs![accounts, NUM_FIXED; ..;];
        let [
            mango_group_ai, // read
            root_bank_ai,   // write
        ] = fixed_accounts;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        check!(
            mango_group.find_root_bank_index(root_bank_ai.key).is_some(),
            MangoErrorCode::InvalidRootBank
        )?;
        // TODO check root bank belongs to group in load functions
        let mut root_bank = RootBank::load_mut_checked(&root_bank_ai, program_id)?;
        check_eq!(root_bank.num_node_banks, node_bank_ais.len(), MangoErrorCode::Default)?;
        for i in 0..root_bank.num_node_banks - 1 {
            check!(
                node_bank_ais.iter().any(|ai| ai.key == &root_bank.node_banks[i]),
                MangoErrorCode::InvalidNodeBank
            )?;
        }

        root_bank.update_index(node_bank_ais, program_id)?;

        Ok(())
    }

    #[inline(never)]
    /// similar to serum dex, but also need to do some extra magic with funding
    fn consume_events(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        limit: usize,
    ) -> MangoResult<()> {
        const NUM_FIXED: usize = 4;
        let (fixed_ais, mango_account_ais) = array_refs![accounts, NUM_FIXED; ..;];
        let [
            mango_group_ai,     // read
            mango_cache_ai,     // read
            perp_market_ai,     // write
            event_queue_ai,     // write
        ] = fixed_ais;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        let mango_cache = MangoCache::load_checked(mango_cache_ai, program_id, &mango_group)?;
        let mut perp_market =
            PerpMarket::load_mut_checked(perp_market_ai, program_id, mango_group_ai.key)?;
        let mut event_queue: EventQueue =
            EventQueue::load_mut_checked(event_queue_ai, program_id, &perp_market)?;

        let market_index = mango_group.find_perp_market_index(perp_market_ai.key).unwrap();
        let cache = &mango_cache.perp_market_cache[market_index];
        let info = &mango_group.perp_markets[market_index];

        for _ in 0..limit {
            let event = match event_queue.peek_front() {
                None => break,
                Some(e) => e,
            };

            match EventType::try_from(event.event_type).map_err(|_| throw!())? {
                EventType::Fill => {
                    let fill: &FillEvent = cast_ref(event);

                    // TODO add msg! for FillEvent

                    // handle self trade separately
                    if fill.maker == fill.taker {
                        let mut ma = match mango_account_ais
                            .binary_search_by_key(&fill.maker, |ai| *(ai.key))
                        {
                            Ok(i) => MangoAccount::load_mut_checked(
                                &mango_account_ais[i],
                                program_id,
                                mango_group_ai.key,
                            )?,
                            Err(_) => {
                                msg!("Unable to find account {}", fill.taker.to_string());
                                return Ok(());
                            } // If it's not found, stop consuming events
                        };

                        ma.execute_maker(market_index, &mut perp_market, info, cache, fill)?;
                        ma.execute_taker(market_index, &mut perp_market, info, cache, fill)?;
                    } else {
                        let mut maker = match mango_account_ais
                            .binary_search_by_key(&fill.maker, |ai| *(ai.key))
                        {
                            Ok(i) => MangoAccount::load_mut_checked(
                                &mango_account_ais[i],
                                program_id,
                                mango_group_ai.key,
                            )?,
                            Err(_) => {
                                msg!("Unable to find maker account {}", fill.maker.to_string());
                                return Ok(());
                            } // If it's not found, stop consuming events
                        };

                        let mut taker = match mango_account_ais
                            .binary_search_by_key(&fill.taker, |ai| *(ai.key))
                        {
                            Ok(i) => MangoAccount::load_mut_checked(
                                &mango_account_ais[i],
                                program_id,
                                mango_group_ai.key,
                            )?,
                            Err(_) => {
                                msg!("Unable to find taker account {}", fill.taker.to_string());
                                return Ok(());
                            } // If it's not found, stop consuming events
                        };

                        maker.execute_maker(market_index, &mut perp_market, info, cache, fill)?;
                        taker.execute_taker(market_index, &mut perp_market, info, cache, fill)?;
                    }
                }
                EventType::Out => {
                    let out_event: &OutEvent = cast_ref(event);
                    let mut mango_account = match mango_account_ais
                        .binary_search_by_key(&out_event.owner, |ai| *ai.key)
                    {
                        Ok(i) => MangoAccount::load_mut_checked(
                            &mango_account_ais[i],
                            program_id,
                            mango_group_ai.key,
                        )?,
                        Err(_) => return Ok(()), // If it's not found, stop consuming events
                    };
                    mango_account.remove_order(out_event.slot as usize, out_event.quantity)?;
                }
                EventType::Liquidate => {}
            }

            // consume this event
            event_queue.pop_front().map_err(|_| throw!())?;
        }
        Ok(())
    }

    #[inline(never)]
    /// Update the `funding_earned` of a `PerpMarket` using the current book price, spot index price
    /// and time since last update
    fn update_funding(program_id: &Pubkey, accounts: &[AccountInfo]) -> MangoResult<()> {
        const NUM_FIXED: usize = 5;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai,     // read
            mango_cache_ai,     // read
            perp_market_ai,     // write
            bids_ai,            // read
            asks_ai,            // read
        ] = accounts;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;

        let mango_cache = MangoCache::load_checked(mango_cache_ai, program_id, &mango_group)?;

        let mut perp_market =
            PerpMarket::load_mut_checked(perp_market_ai, program_id, mango_group_ai.key)?;

        let book = Book::load_checked(program_id, bids_ai, asks_ai, &perp_market)?;

        let market_index = mango_group.find_perp_market_index(perp_market_ai.key).unwrap();

        let clock = Clock::get()?;
        let now_ts = clock.unix_timestamp as u64;

        perp_market.update_funding(&mango_group, &book, &mango_cache, market_index, now_ts)?;

        msg!(
            "{{\"long_funding\":{}, \"short_funding\":{}}}",
            perp_market.long_funding.to_num::<f64>(),
            perp_market.short_funding.to_num::<f64>()
        );

        Ok(())
    }

    #[inline(never)]
    /// Settle the mngo_accrued in a PerpAccount for MNGO tokens
    fn redeem_mngo(program_id: &Pubkey, accounts: &[AccountInfo]) -> MangoResult<()> {
        const NUM_FIXED: usize = 11;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai,     // read
            mango_cache_ai,     // read
            mango_account_ai,   // write
            owner_ai,           // read, signer
            perp_market_ai,     // read
            mngo_perp_vault_ai, // write
            mngo_root_bank_ai,  // read
            mngo_node_bank_ai,  // write
            mngo_bank_vault_ai, // write
            signer_ai,          // read
            token_prog_ai,      // read
        ] = accounts;
        check!(token_prog_ai.key == &spl_token::ID, MangoErrorCode::InvalidProgramId)?;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        let market_index = mango_group
            .find_perp_market_index(perp_market_ai.key)
            .ok_or(throw_err!(MangoErrorCode::InvalidMarket))?;
        let mngo_index = mango_group
            .find_root_bank_index(mngo_root_bank_ai.key)
            .ok_or(throw_err!(MangoErrorCode::InvalidRootBank))?;

        let mango_cache = MangoCache::load_checked(mango_cache_ai, program_id, &mango_group)?;
        let mngo_bank_cache = &mango_cache.root_bank_cache[mngo_index];

        let mut mango_account =
            MangoAccount::load_mut_checked(mango_account_ai, program_id, mango_group_ai.key)?;
        check!(&mango_account.owner == owner_ai.key, MangoErrorCode::InvalidOwner)?;
        check!(!mango_account.is_bankrupt, MangoErrorCode::Bankrupt)?;
        check!(owner_ai.is_signer, MangoErrorCode::SignerNecessary)?;

        let perp_account = &mut mango_account.perp_accounts[market_index];

        // Load the mngo banks
        let root_bank = RootBank::load_checked(mngo_root_bank_ai, program_id)?;
        check!(
            root_bank.node_banks.contains(mngo_node_bank_ai.key),
            MangoErrorCode::InvalidNodeBank
        )?;
        let mut mngo_node_bank = NodeBank::load_mut_checked(mngo_node_bank_ai, program_id)?;
        check_eq!(&mngo_node_bank.vault, mngo_bank_vault_ai.key, MangoErrorCode::InvalidVault)?;

        let perp_market = PerpMarket::load_checked(perp_market_ai, program_id, mango_group_ai.key)?;
        check!(mngo_perp_vault_ai.key == &perp_market.mngo_vault, MangoErrorCode::InvalidVault)?;

        let mngo_perp_vault = Account::unpack(&mngo_perp_vault_ai.try_borrow_data()?)?;

        let mngo = min(perp_account.mngo_accrued, mngo_perp_vault.amount);
        perp_account.mngo_accrued -= mngo;

        let signers_seeds = gen_signer_seeds(&mango_group.signer_nonce, mango_group_ai.key);
        invoke_transfer(
            token_prog_ai,
            mngo_perp_vault_ai,
            mngo_bank_vault_ai,
            signer_ai,
            &[&signers_seeds],
            mngo,
        )?;

        let now_ts = Clock::get()?.unix_timestamp as u64;
        check!(
            now_ts <= mngo_bank_cache.last_update + mango_group.valid_interval,
            MangoErrorCode::InvalidCache
        )?;

        checked_add_net(
            mngo_bank_cache,
            &mut mngo_node_bank,
            &mut mango_account,
            mngo_index,
            I80F48::from_num(mngo),
        )
    }

    #[inline(never)]
    fn add_mango_account_info(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        info: [u8; INFO_LEN],
    ) -> MangoResult<()> {
        const NUM_FIXED: usize = 3;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai,     // read 
            mango_account_ai,   // write
            owner_ai            // signer
        ] = accounts;

        let mut mango_account =
            MangoAccount::load_mut_checked(mango_account_ai, program_id, mango_group_ai.key)?;
        check!(&mango_account.owner == owner_ai.key, MangoErrorCode::InvalidOwner)?;
        check!(owner_ai.is_signer, MangoErrorCode::InvalidSignerKey)?;

        mango_account.info = info;
        Ok(())
    }

    #[inline(never)]
    fn deposit_msrm(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        quantity: u64,
    ) -> MangoResult<()> {
        const NUM_FIXED: usize = 6;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai,     // read
            mango_account_ai,   // write
            owner_ai,           // read, signer
            msrm_account_ai,    // write
            msrm_vault_ai,      // write
            token_prog_ai,      // read
        ] = accounts;
        check!(token_prog_ai.key == &spl_token::ID, MangoErrorCode::InvalidProgramId)?;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        check!(msrm_vault_ai.key == &mango_group.msrm_vault, MangoErrorCode::InvalidVault)?;

        let mut mango_account =
            MangoAccount::load_mut_checked(mango_account_ai, program_id, mango_group_ai.key)?;
        check!(&mango_account.owner == owner_ai.key, MangoErrorCode::InvalidOwner)?;
        check!(owner_ai.is_signer, MangoErrorCode::SignerNecessary)?;

        invoke_transfer(token_prog_ai, msrm_account_ai, msrm_vault_ai, owner_ai, &[], quantity)?;

        mango_account.msrm_amount += quantity;

        Ok(())
    }

    #[inline(never)]
    fn withdraw_msrm(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        quantity: u64,
    ) -> MangoResult<()> {
        const NUM_FIXED: usize = 7;
        let accounts = array_ref![accounts, 0, NUM_FIXED];
        let [
            mango_group_ai,     // read
            mango_account_ai,   // write
            owner_ai,           // read, signer
            msrm_account_ai,    // write
            msrm_vault_ai,      // write
            signer_ai,          // read
            token_prog_ai,      // read
        ] = accounts;
        check!(token_prog_ai.key == &spl_token::ID, MangoErrorCode::InvalidProgramId)?;

        let mango_group = MangoGroup::load_checked(mango_group_ai, program_id)?;
        check!(msrm_vault_ai.key == &mango_group.msrm_vault, MangoErrorCode::InvalidVault)?;

        let mut mango_account =
            MangoAccount::load_mut_checked(mango_account_ai, program_id, mango_group_ai.key)?;
        check!(&mango_account.owner == owner_ai.key, MangoErrorCode::InvalidOwner)?;
        check!(owner_ai.is_signer, MangoErrorCode::SignerNecessary)?;

        check!(mango_account.msrm_amount >= quantity, MangoErrorCode::InsufficientFunds)?;

        let signer_seeds = gen_signer_seeds(&mango_group.signer_nonce, mango_group_ai.key);
        invoke_transfer(
            token_prog_ai,
            msrm_vault_ai,
            msrm_account_ai,
            signer_ai,
            &[&signer_seeds],
            quantity,
        )?;

        mango_account.msrm_amount -= quantity;

        Ok(())
    }

    pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> MangoResult<()> {
        let instruction =
            MangoInstruction::unpack(data).ok_or(ProgramError::InvalidInstructionData)?;
        match instruction {
            MangoInstruction::InitMangoGroup {
                signer_nonce,
                valid_interval,
                quote_optimal_util,
                quote_optimal_rate,
                quote_max_rate,
            } => {
                msg!("Mango: InitMangoGroup");
                Self::init_mango_group(
                    program_id,
                    accounts,
                    signer_nonce,
                    valid_interval,
                    quote_optimal_util,
                    quote_optimal_rate,
                    quote_max_rate,
                )
            }
            MangoInstruction::InitMangoAccount => {
                msg!("Mango: InitMangoAccount");
                Self::init_mango_account(program_id, accounts)
            }
            MangoInstruction::Deposit { quantity } => {
                msg!("Mango: Deposit");
                Self::deposit(program_id, accounts, quantity)
            }
            MangoInstruction::Withdraw { quantity, allow_borrow } => {
                msg!("Mango: Withdraw");
                Self::withdraw(program_id, accounts, quantity, allow_borrow)
            }
            MangoInstruction::AddSpotMarket {
                market_index,
                maint_leverage,
                init_leverage,
                liquidation_fee,
                optimal_util,
                optimal_rate,
                max_rate,
            } => {
                msg!("Mango: AddSpotMarket");
                Self::add_spot_market(
                    program_id,
                    accounts,
                    market_index,
                    maint_leverage,
                    init_leverage,
                    liquidation_fee,
                    optimal_util,
                    optimal_rate,
                    max_rate,
                )
            }
            MangoInstruction::AddToBasket { .. } => {
                msg!("Mango: AddToBasket Deprecated");
                Ok(())
            }
            MangoInstruction::Borrow { .. } => {
                msg!("Mango: Borrow DEPRECATED");
                Ok(())
            }
            MangoInstruction::CachePrices => {
                msg!("Mango: CachePrices");
                Self::cache_prices(program_id, accounts)
            }
            MangoInstruction::CacheRootBanks => {
                msg!("Mango: CacheRootBanks");
                Self::cache_root_banks(program_id, accounts)
            }
            MangoInstruction::PlaceSpotOrder { order } => {
                msg!("Mango: PlaceSpotOrder");
                Self::place_spot_order(program_id, accounts, order)
            }
            MangoInstruction::CancelSpotOrder { order, .. } => {
                msg!("Mango: CancelSpotOrder");
                let data = serum_dex::instruction::MarketInstruction::CancelOrderV2(order).pack();
                Self::cancel_spot_order(program_id, accounts, data)
            }
            MangoInstruction::AddOracle => {
                msg!("Mango: AddOracle");
                Self::add_oracle(program_id, accounts)
            }
            MangoInstruction::SettleFunds => {
                msg!("Mango: SettleFunds");
                Self::settle_funds(program_id, accounts)
            }
            MangoInstruction::UpdateRootBank => {
                msg!("Mango: UpdateRootBank");
                Self::update_root_bank(program_id, accounts)
            }

            MangoInstruction::AddPerpMarket {
                market_index,
                maint_leverage,
                init_leverage,
                liquidation_fee,
                maker_fee,
                taker_fee,
                base_lot_size,
                quote_lot_size,
                rate,
                max_depth_bps,
                target_period_length,
                mngo_per_period,
            } => {
                msg!("Mango: AddPerpMarket");
                Self::add_perp_market(
                    program_id,
                    accounts,
                    market_index,
                    maint_leverage,
                    init_leverage,
                    liquidation_fee,
                    maker_fee,
                    taker_fee,
                    base_lot_size,
                    quote_lot_size,
                    rate,
                    max_depth_bps,
                    target_period_length,
                    mngo_per_period,
                )
            }
            MangoInstruction::PlacePerpOrder {
                side,
                price,
                quantity,
                client_order_id,
                order_type,
            } => {
                msg!("Mango: PlacePerpOrder client_order_id={}", client_order_id);
                Self::place_perp_order(
                    program_id,
                    accounts,
                    side,
                    price,
                    quantity,
                    client_order_id,
                    order_type,
                )
            }
            MangoInstruction::CancelPerpOrderByClientId { client_order_id, invalid_id_ok } => {
                msg!("Mango: CancelPerpOrderByClientId client_order_id={}", client_order_id);
                let result =
                    Self::cancel_perp_order_by_client_id(program_id, accounts, client_order_id);
                if invalid_id_ok {
                    if let Err(MangoError::MangoErrorCode { mango_error_code, .. }) = result {
                        if mango_error_code == MangoErrorCode::InvalidOrderId
                            || mango_error_code == MangoErrorCode::ClientIdNotFound
                        {
                            return Ok(());
                        }
                    }
                }
                result
            }
            MangoInstruction::CancelPerpOrder { order_id, invalid_id_ok } => {
                // TODO OPT this log may cost too much compute
                msg!("Mango: CancelPerpOrder order_id={}", order_id);
                let result = Self::cancel_perp_order(program_id, accounts, order_id);
                if invalid_id_ok {
                    if let Err(MangoError::MangoErrorCode { mango_error_code, .. }) = result {
                        if mango_error_code == MangoErrorCode::InvalidOrderId {
                            return Ok(());
                        }
                    }
                }
                result
            }
            MangoInstruction::ConsumeEvents { limit } => {
                msg!("Mango: ConsumeEvents limit={}", limit);
                Self::consume_events(program_id, accounts, limit)
            }
            MangoInstruction::CachePerpMarkets => {
                msg!("Mango: CachePerpMarkets");
                Self::cache_perp_markets(program_id, accounts)
            }
            MangoInstruction::UpdateFunding => {
                msg!("Mango: UpdateFunding");
                Self::update_funding(program_id, accounts)
            }
            MangoInstruction::SetOracle { price } => {
                // msg!("Mango: SetOracle {:?}", price);
                msg!("Mango: SetOracle");
                Self::set_oracle(program_id, accounts, price)
            }
            MangoInstruction::SettlePnl { market_index } => {
                msg!("Mango: SettlePnl");
                Self::settle_pnl(program_id, accounts, market_index)
            }
            MangoInstruction::SettleBorrow { .. } => {
                msg!("Mango: SettleBorrow DEPRECATED");
                Ok(())
            }
            MangoInstruction::ForceCancelSpotOrders { limit } => {
                msg!("Mango: ForceCancelSpotOrders");
                Self::force_cancel_spot_orders(program_id, accounts, limit)
            }
            MangoInstruction::ForceCancelPerpOrders { limit } => {
                msg!("Mango: ForceCancelPerpOrders");
                Self::force_cancel_perp_orders(program_id, accounts, limit)
            }
            MangoInstruction::LiquidateTokenAndToken { max_liab_transfer } => {
                msg!("Mango: LiquidateTokenAndToken");
                Self::liquidate_token_and_token(program_id, accounts, max_liab_transfer)
            }
            MangoInstruction::LiquidateTokenAndPerp {
                asset_type,
                asset_index,
                liab_type,
                liab_index,
                max_liab_transfer,
            } => {
                msg!("Mango: LiquidateTokenAndPerp");
                Self::liquidate_token_and_perp(
                    program_id,
                    accounts,
                    asset_type,
                    asset_index,
                    liab_type,
                    liab_index,
                    max_liab_transfer,
                )
            }
            MangoInstruction::LiquidatePerpMarket { base_transfer_request } => {
                msg!("Mango: LiquidatePerpMarket");
                Self::liquidate_perp_market(program_id, accounts, base_transfer_request)
            }
            MangoInstruction::SettleFees => {
                msg!("Mango: SettleFees");
                Self::settle_fees(program_id, accounts)
            }
            MangoInstruction::ResolvePerpBankruptcy { liab_index, max_liab_transfer } => {
                msg!("Mango: ResolvePerpBankruptcy");
                Self::resolve_perp_bankruptcy(program_id, accounts, liab_index, max_liab_transfer)
            }
            MangoInstruction::ResolveTokenBankruptcy { max_liab_transfer } => {
                msg!("Mango: ResolveTokenBankruptcy");
                Self::resolve_token_bankruptcy(program_id, accounts, max_liab_transfer)
            }
            MangoInstruction::InitSpotOpenOrders => {
                msg!("Mango: InitSpotOpenOrders");
                Self::init_spot_open_orders(program_id, accounts)
            }
            MangoInstruction::RedeemMngo => {
                msg!("Mango: RedeemMngo");
                Self::redeem_mngo(program_id, accounts)
            }
            MangoInstruction::AddMangoAccountInfo { info } => {
                msg!("Mango: AddMangoAccountInfo");
                Self::add_mango_account_info(program_id, accounts, info)
            }
            MangoInstruction::DepositMsrm { quantity } => {
                msg!("Mango: DepositMsrm");
                Self::deposit_msrm(program_id, accounts, quantity)
            }
            MangoInstruction::WithdrawMsrm { quantity } => {
                msg!("Mango: WithdrawMsrm");
                Self::withdraw_msrm(program_id, accounts, quantity)
            }
        }
    }
}

fn init_root_bank(
    program_id: &Pubkey,
    mango_group: &MangoGroup,
    mint_ai: &AccountInfo,
    vault_ai: &AccountInfo,
    root_bank_ai: &AccountInfo,
    node_bank_ai: &AccountInfo,
    rent: &Rent,

    optimal_util: I80F48,
    optimal_rate: I80F48,
    max_rate: I80F48,
) -> MangoResult<RootBank> {
    let vault = Account::unpack(&vault_ai.try_borrow_data()?)?;
    check!(vault.is_initialized(), MangoErrorCode::Default)?;
    check_eq!(vault.owner, mango_group.signer_key, MangoErrorCode::Default)?;
    check_eq!(&vault.mint, mint_ai.key, MangoErrorCode::Default)?;
    check_eq!(vault_ai.owner, &spl_token::id(), MangoErrorCode::Default)?;

    let mut _node_bank = NodeBank::load_and_init(&node_bank_ai, &program_id, &vault_ai, rent)?;
    let root_bank = RootBank::load_and_init(
        &root_bank_ai,
        &program_id,
        node_bank_ai,
        rent,
        optimal_util,
        optimal_rate,
        max_rate,
    )?;

    Ok(*root_bank)
}

fn invoke_settle_funds<'a>(
    dex_prog_ai: &AccountInfo<'a>,
    spot_market_ai: &AccountInfo<'a>,
    open_orders_ai: &AccountInfo<'a>,
    signer_ai: &AccountInfo<'a>,
    dex_base_ai: &AccountInfo<'a>,
    dex_quote_ai: &AccountInfo<'a>,
    base_vault_ai: &AccountInfo<'a>,
    quote_vault_ai: &AccountInfo<'a>,
    dex_signer_ai: &AccountInfo<'a>,
    token_prog_ai: &AccountInfo<'a>,
    signers_seeds: &[&[&[u8]]],
) -> ProgramResult {
    let data = serum_dex::instruction::MarketInstruction::SettleFunds.pack();
    let instruction = Instruction {
        program_id: *dex_prog_ai.key,
        data,
        accounts: vec![
            AccountMeta::new(*spot_market_ai.key, false),
            AccountMeta::new(*open_orders_ai.key, false),
            AccountMeta::new_readonly(*signer_ai.key, true),
            AccountMeta::new(*dex_base_ai.key, false),
            AccountMeta::new(*dex_quote_ai.key, false),
            AccountMeta::new(*base_vault_ai.key, false),
            AccountMeta::new(*quote_vault_ai.key, false),
            AccountMeta::new_readonly(*dex_signer_ai.key, false),
            AccountMeta::new_readonly(*token_prog_ai.key, false),
            AccountMeta::new(*quote_vault_ai.key, false),
        ],
    };

    let account_infos = [
        dex_prog_ai.clone(),
        spot_market_ai.clone(),
        open_orders_ai.clone(),
        signer_ai.clone(),
        dex_base_ai.clone(),
        dex_quote_ai.clone(),
        base_vault_ai.clone(),
        quote_vault_ai.clone(),
        dex_signer_ai.clone(),
        token_prog_ai.clone(),
        quote_vault_ai.clone(),
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
    token_prog_ai: &AccountInfo<'a>,
    source_ai: &AccountInfo<'a>,
    dest_ai: &AccountInfo<'a>,
    authority_ai: &AccountInfo<'a>,
    signers_seeds: &[&[&[u8]]],
    quantity: u64,
) -> ProgramResult {
    let transfer_instruction = spl_token::instruction::transfer(
        &spl_token::ID,
        source_ai.key,
        dest_ai.key,
        authority_ai.key,
        &[],
        quantity,
    )?;
    let accs = [
        token_prog_ai.clone(), // TODO check if passing in program_id is necessary
        source_ai.clone(),
        dest_ai.clone(),
        authority_ai.clone(),
    ];

    solana_program::program::invoke_signed(&transfer_instruction, &accs, signers_seeds)
}

#[inline(never)]
fn read_oracle(
    mango_group: &MangoGroup,
    token_index: usize,
    oracle_ai: &AccountInfo,
) -> MangoResult<I80F48> {
    let quote_decimals: u8 = mango_group.tokens[QUOTE_INDEX].decimals;
    let oracle_type = determine_oracle_type(oracle_ai);
    let price = match oracle_type {
        OracleType::Pyth => {
            let price_account = Price::get_price(oracle_ai).unwrap();
            let value = I80F48::from_num(price_account.agg.price);

            let decimals = (quote_decimals as i32)
                .checked_add(price_account.expo)
                .unwrap()
                .checked_sub(mango_group.tokens[token_index].decimals as i32)
                .unwrap();

            let decimal_adj = I80F48::from_num(10u64.pow(decimals.abs() as u32));
            if decimals < 0 {
                value.checked_div(decimal_adj).unwrap()
            } else {
                value.checked_mul(decimal_adj).unwrap()
            }
        }
        OracleType::Stub => {
            let oracle = StubOracle::load(oracle_ai)?;
            I80F48::from_num(oracle.price)
        }
        OracleType::Switchboard => {
            // TODO do decimal fixes for cases where base decimals != quote decimals
            let result =
                FastRoundResultAccountData::deserialize(&oracle_ai.try_borrow_data()?).unwrap();
            let value = I80F48::from_num(result.result.result);
            let decimals = (quote_decimals as i32)
                .checked_sub(mango_group.tokens[token_index].decimals as i32)
                .unwrap();
            if decimals < 0 {
                let decimal_adj = I80F48::from_num(10u64.pow(decimals.abs() as u32));
                value.checked_div(decimal_adj).unwrap()
            } else if decimals > 0 {
                let decimal_adj = I80F48::from_num(10u64.pow(decimals.abs() as u32));
                value.checked_mul(decimal_adj).unwrap()
            } else {
                value
            }
        }
        OracleType::Unknown => {
            panic!("Unknown oracle");
        }
    };
    Ok(price)
}

fn checked_change_net(
    root_bank_cache: &RootBankCache,
    node_bank: &mut NodeBank,
    mango_account: &mut MangoAccount,
    token_index: usize,
    native_quantity: I80F48,
) -> MangoResult<()> {
    if native_quantity.is_negative() {
        checked_sub_net(root_bank_cache, node_bank, mango_account, token_index, -native_quantity)
    } else if native_quantity.is_positive() {
        checked_add_net(root_bank_cache, node_bank, mango_account, token_index, native_quantity)
    } else {
        Ok(()) // This is an optimization to prevent unnecessary I80F48 calculations
    }
}

/// If there are borrows, pay down borrows first then increase deposits
/// WARNING: won't work if native_quantity is less than zero
fn checked_add_net(
    root_bank_cache: &RootBankCache,
    node_bank: &mut NodeBank,
    mango_account: &mut MangoAccount,
    token_index: usize,
    mut native_quantity: I80F48,
) -> MangoResult<()> {
    if mango_account.borrows[token_index].is_positive() {
        let native_borrows = mango_account.get_native_borrow(root_bank_cache, token_index)?;

        if native_quantity < native_borrows {
            return checked_sub_borrow(
                node_bank,
                mango_account,
                token_index,
                native_quantity / root_bank_cache.borrow_index,
            );
        } else {
            let borrows = mango_account.borrows[token_index];
            checked_sub_borrow(node_bank, mango_account, token_index, borrows)?;
            native_quantity -= native_borrows;
        }
    }

    checked_add_deposit(
        node_bank,
        mango_account,
        token_index,
        native_quantity / root_bank_cache.deposit_index,
    )
}

/// If there are deposits, draw down deposits first then increase borrows
/// WARNING: won't work if native_quantity is less than zero
fn checked_sub_net(
    root_bank_cache: &RootBankCache,
    node_bank: &mut NodeBank,
    mango_account: &mut MangoAccount,
    token_index: usize,
    mut native_quantity: I80F48,
) -> MangoResult<()> {
    if mango_account.deposits[token_index].is_positive() {
        let native_deposits = mango_account.get_native_deposit(root_bank_cache, token_index)?;

        if native_quantity < native_deposits {
            return checked_sub_deposit(
                node_bank,
                mango_account,
                token_index,
                native_quantity / root_bank_cache.deposit_index,
            );
        } else {
            let deposits = mango_account.deposits[token_index];
            checked_sub_deposit(node_bank, mango_account, token_index, deposits)?;
            native_quantity -= native_deposits;
        }
    }

    checked_add_borrow(
        node_bank,
        mango_account,
        token_index,
        native_quantity / root_bank_cache.borrow_index,
    )?;

    check!(
        node_bank.has_valid_deposits_borrows(root_bank_cache),
        MangoErrorCode::InsufficientLiquidity
    )
}

/// TODO - although these values are I8048, they must never be less than zero
fn checked_add_deposit(
    node_bank: &mut NodeBank,
    mango_account: &mut MangoAccount,
    token_index: usize,
    quantity: I80F48,
) -> MangoResult<()> {
    mango_account.checked_add_deposit(token_index, quantity)?;
    node_bank.checked_add_deposit(quantity)
}

fn checked_sub_deposit(
    node_bank: &mut NodeBank,
    mango_account: &mut MangoAccount,
    token_index: usize,
    quantity: I80F48,
) -> MangoResult<()> {
    mango_account.checked_sub_deposit(token_index, quantity)?;
    node_bank.checked_sub_deposit(quantity)
}

fn checked_add_borrow(
    node_bank: &mut NodeBank,
    mango_account: &mut MangoAccount,
    token_index: usize,
    quantity: I80F48,
) -> MangoResult<()> {
    mango_account.checked_add_borrow(token_index, quantity)?;
    node_bank.checked_add_borrow(quantity)
}

fn checked_sub_borrow(
    node_bank: &mut NodeBank,
    mango_account: &mut MangoAccount,
    token_index: usize,
    quantity: I80F48,
) -> MangoResult<()> {
    mango_account.checked_sub_borrow(token_index, quantity)?;
    node_bank.checked_sub_borrow(quantity)
}

fn invoke_cancel_orders<'a>(
    open_orders_ai: &AccountInfo<'a>,
    dex_prog_ai: &AccountInfo<'a>,
    spot_market_ai: &AccountInfo<'a>,
    bids_ai: &AccountInfo<'a>,
    asks_ai: &AccountInfo<'a>,
    signer_ai: &AccountInfo<'a>,
    dex_event_queue_ai: &AccountInfo<'a>,
    signers_seeds: &[&[&[u8]]],

    mut limit: u8,
) -> MangoResult<()> {
    let mut cancels = vec![];
    {
        let open_orders = load_open_orders(open_orders_ai)?;

        let market = load_market_state(spot_market_ai, dex_prog_ai.key)?;
        let bids = load_bids_mut(&market, bids_ai)?;
        let asks = load_asks_mut(&market, asks_ai)?;

        limit = min(limit, open_orders.free_slot_bits.count_zeros() as u8);
        if limit == 0 {
            return Ok(());
        }
        for j in 0..128 {
            let slot_mask = 1u128 << j;
            if open_orders.free_slot_bits & slot_mask != 0 {
                // means slot is free
                continue;
            }
            let order_id = open_orders.orders[j];

            let side = if open_orders.is_bid_bits & slot_mask != 0 {
                match bids.find_by_key(order_id) {
                    None => continue,
                    Some(_) => serum_dex::matching::Side::Bid,
                }
            } else {
                match asks.find_by_key(order_id) {
                    None => continue,
                    Some(_) => serum_dex::matching::Side::Ask,
                }
            };

            let cancel_instruction =
                serum_dex::instruction::CancelOrderInstructionV2 { side, order_id };

            cancels.push(cancel_instruction);

            limit -= 1;
            if limit == 0 {
                break;
            }
        }
    }

    let mut instruction = Instruction {
        program_id: *dex_prog_ai.key,
        data: vec![],
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

    for cancel in cancels.iter() {
        let cancel_instruction =
            serum_dex::instruction::MarketInstruction::CancelOrderV2(cancel.clone());
        instruction.data = cancel_instruction.pack();
        solana_program::program::invoke_signed(&instruction, &account_infos, signers_seeds)?;
    }

    Ok(())
}

fn invoke_new_order<'a>(
    dex_prog_ai: &AccountInfo<'a>, // Have to add account of the program id
    spot_market_ai: &AccountInfo<'a>,
    open_orders_ai: &AccountInfo<'a>,
    dex_request_queue_ai: &AccountInfo<'a>,
    dex_event_queue_ai: &AccountInfo<'a>,
    bids_ai: &AccountInfo<'a>,
    asks_ai: &AccountInfo<'a>,
    vault_ai: &AccountInfo<'a>,
    signer_ai: &AccountInfo<'a>,
    dex_base_ai: &AccountInfo<'a>,
    dex_quote_ai: &AccountInfo<'a>,
    token_prog_ai: &AccountInfo<'a>,
    rent_ai: &AccountInfo<'a>,
    msrm_or_srm_vault_ai: &AccountInfo<'a>,
    signers_seeds: &[&[&[u8]]],

    order: NewOrderInstructionV3,
) -> ProgramResult {
    let data = serum_dex::instruction::MarketInstruction::NewOrderV3(order).pack();
    let mut instruction = Instruction {
        program_id: *dex_prog_ai.key,
        data,
        accounts: vec![
            AccountMeta::new(*spot_market_ai.key, false),
            AccountMeta::new(*open_orders_ai.key, false),
            AccountMeta::new(*dex_request_queue_ai.key, false),
            AccountMeta::new(*dex_event_queue_ai.key, false),
            AccountMeta::new(*bids_ai.key, false),
            AccountMeta::new(*asks_ai.key, false),
            AccountMeta::new(*vault_ai.key, false),
            AccountMeta::new_readonly(*signer_ai.key, true),
            AccountMeta::new(*dex_base_ai.key, false),
            AccountMeta::new(*dex_quote_ai.key, false),
            AccountMeta::new_readonly(*token_prog_ai.key, false),
            AccountMeta::new_readonly(*rent_ai.key, false),
        ],
    };

    if msrm_or_srm_vault_ai.key != &Pubkey::default() {
        instruction.accounts.push(AccountMeta::new_readonly(*msrm_or_srm_vault_ai.key, false));
        let account_infos = [
            dex_prog_ai.clone(), // Have to add account of the program id
            spot_market_ai.clone(),
            open_orders_ai.clone(),
            dex_request_queue_ai.clone(),
            dex_event_queue_ai.clone(),
            bids_ai.clone(),
            asks_ai.clone(),
            vault_ai.clone(),
            signer_ai.clone(),
            dex_base_ai.clone(),
            dex_quote_ai.clone(),
            token_prog_ai.clone(),
            rent_ai.clone(),
            msrm_or_srm_vault_ai.clone(),
        ];
        solana_program::program::invoke_signed(&instruction, &account_infos, signers_seeds)
    } else {
        let account_infos = [
            dex_prog_ai.clone(), // Have to add account of the program id
            spot_market_ai.clone(),
            open_orders_ai.clone(),
            dex_request_queue_ai.clone(),
            dex_event_queue_ai.clone(),
            bids_ai.clone(),
            asks_ai.clone(),
            vault_ai.clone(),
            signer_ai.clone(),
            dex_base_ai.clone(),
            dex_quote_ai.clone(),
            token_prog_ai.clone(),
            rent_ai.clone(),
        ];
        solana_program::program::invoke_signed(&instruction, &account_infos, signers_seeds)
    }
}

fn invoke_init_open_orders<'a>(
    dex_prog_ai: &AccountInfo<'a>, // Have to add account of the program id
    open_orders_ai: &AccountInfo<'a>,
    signer_ai: &AccountInfo<'a>,
    spot_market_ai: &AccountInfo<'a>,
    rent_ai: &AccountInfo<'a>,
    signers_seeds: &[&[&[u8]]],
) -> ProgramResult {
    let data = serum_dex::instruction::MarketInstruction::InitOpenOrders.pack();
    let instruction = Instruction {
        program_id: *dex_prog_ai.key,
        data,
        accounts: vec![
            AccountMeta::new(*open_orders_ai.key, false),
            AccountMeta::new_readonly(*signer_ai.key, true),
            AccountMeta::new_readonly(*spot_market_ai.key, false),
            AccountMeta::new_readonly(*rent_ai.key, false),
        ],
    };

    let account_infos = [
        dex_prog_ai.clone(),
        open_orders_ai.clone(),
        signer_ai.clone(),
        spot_market_ai.clone(),
        rent_ai.clone(),
    ];
    solana_program::program::invoke_signed(&instruction, &account_infos, signers_seeds)
}

/*
TODO
check bankruptcy everywhere

TODO test order types
 */
