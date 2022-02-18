use anchor_lang::Key;
use std::borrow::Borrow;
use std::mem::size_of;
use std::str::FromStr;

use bincode::deserialize;
use fixed::types::I80F48;
use mango_common::Loadable;
use solana_program::{
    account_info::AccountInfo,
    clock::{Clock, UnixTimestamp},
    program_option::COption,
    program_pack::Pack,
    pubkey::*,
    rent::*,
    system_instruction, sysvar,
};
use solana_program_test::*;
use solana_sdk::{
    account::ReadableAccount,
    instruction::Instruction,
    signature::{Keypair, Signer},
    transaction::Transaction,
    transport::TransportError,
};
use spl_token::{state::*, *};

use mango::{entrypoint::*, ids::*, instruction::*, matching::*, oracle::*, state::*, utils::*};

use serum_dex::instruction::NewOrderInstructionV3;
use solana_program::entrypoint::ProgramResult;

pub mod assertions;
pub mod cookies;
pub mod scenarios;
use self::cookies::*;

const RUST_LOG_DEFAULT: &str = "solana_rbpf::vm=info,\
             solana_program_runtime::stable_log=debug,\
             solana_runtime::message_processor=debug,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info,\
             solana_bpf_loader_program=debug"; // for - Program ... consumed 5857 of 200000 compute units

trait AddPacked {
    fn add_packable_account<T: Pack>(
        &mut self,
        pubkey: Pubkey,
        amount: u64,
        data: &T,
        owner: &Pubkey,
    );
}

impl AddPacked for ProgramTest {
    fn add_packable_account<T: Pack>(
        &mut self,
        pubkey: Pubkey,
        amount: u64,
        data: &T,
        owner: &Pubkey,
    ) {
        let mut account = solana_sdk::account::Account::new(amount, T::get_packed_len(), owner);
        data.pack_into_slice(&mut account.data);
        self.add_account(pubkey, account);
    }
}

pub struct ListingKeys {
    market_key: Keypair,
    req_q_key: Keypair,
    event_q_key: Keypair,
    bids_key: Keypair,
    asks_key: Keypair,
    vault_signer_pk: Pubkey,
    vault_signer_nonce: u64,
}

#[derive(Copy, Clone)]
pub struct MarketPubkeys {
    pub market: Pubkey,
    pub req_q: Pubkey,
    pub event_q: Pubkey,
    pub bids: Pubkey,
    pub asks: Pubkey,
    pub coin_vault: Pubkey,
    pub pc_vault: Pubkey,
    pub vault_signer_key: Pubkey,
}

pub struct MangoProgramTestConfig {
    pub compute_limit: u64,
    pub num_users: usize,
    pub num_mints: usize,
    pub consume_perp_events_count: usize,
}

impl MangoProgramTestConfig {
    #[allow(dead_code)]
    pub fn default() -> Self {
        MangoProgramTestConfig {
            compute_limit: 200_000,
            num_users: 2,
            num_mints: 16,
            consume_perp_events_count: 3,
        }
    }
    #[allow(dead_code)]
    pub fn default_two_mints() -> Self {
        MangoProgramTestConfig { num_mints: 2, ..Self::default() }
    }
}

pub struct MangoProgramTest {
    pub context: ProgramTestContext,
    pub rent: Rent,
    pub mango_program_id: Pubkey,
    pub serum_program_id: Pubkey,
    pub num_mints: usize,
    pub quote_index: usize,
    pub quote_mint: MintCookie,
    pub mints: Vec<MintCookie>,
    pub num_users: usize,
    pub users: Vec<Keypair>,
    pub token_accounts: Vec<Pubkey>, // user x mint
    pub consume_perp_events_count: usize,
}

impl MangoProgramTest {
    #[allow(dead_code)]
    pub async fn start_new(config: &MangoProgramTestConfig) -> Self {
        let mango_program_id = Pubkey::new_unique();
        let serum_program_id = Pubkey::new_unique();

        // Predefined mints, maybe can even add symbols to them
        // TODO: Figure out where to put MNGO and MSRM mint
        let mut mints: Vec<MintCookie> = vec![
            MintCookie {
                index: 0,
                decimals: 6,
                unit: 10u64.pow(6) as f64,
                base_lot: 100 as f64,
                quote_lot: 10 as f64,
                pubkey: None, //Some(mngo_token::ID),
            }, // symbol: "MNGO".to_string()
            MintCookie {
                index: 1,
                decimals: 6,
                unit: 10u64.pow(6) as f64,
                base_lot: 100 as f64,
                quote_lot: 10 as f64,
                pubkey: None, //Some(msrm_token::ID),
            }, // symbol: "MSRM".to_string()
            MintCookie {
                index: 2,
                decimals: 6,
                unit: 10u64.pow(6) as f64,
                base_lot: 100 as f64,
                quote_lot: 10 as f64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintCookie {
                index: 3,
                decimals: 6,
                unit: 10u64.pow(6) as f64,
                base_lot: 1000 as f64,
                quote_lot: 10 as f64,
                pubkey: None,
            }, // symbol: "ETH".to_string()
            MintCookie {
                index: 4,
                decimals: 9,
                unit: 10u64.pow(9) as f64,
                base_lot: 100000000 as f64,
                quote_lot: 100 as f64,
                pubkey: None,
            }, // symbol: "SOL".to_string()
            MintCookie {
                index: 5,
                decimals: 6,
                unit: 10u64.pow(6) as f64,
                base_lot: 100000 as f64,
                quote_lot: 100 as f64,
                pubkey: None,
            }, // symbol: "SRM".to_string()
            MintCookie {
                index: 6,
                decimals: 6,
                unit: 10u64.pow(6) as f64,
                base_lot: 100 as f64,
                quote_lot: 10 as f64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintCookie {
                index: 7,
                decimals: 6,
                unit: 10u64.pow(6) as f64,
                base_lot: 100 as f64,
                quote_lot: 10 as f64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintCookie {
                index: 8,
                decimals: 6,
                unit: 10u64.pow(6) as f64,
                base_lot: 100 as f64,
                quote_lot: 10 as f64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintCookie {
                index: 9,
                decimals: 6,
                unit: 10u64.pow(6) as f64,
                base_lot: 100 as f64,
                quote_lot: 10 as f64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintCookie {
                index: 10,
                decimals: 6,
                unit: 10u64.pow(6) as f64,
                base_lot: 100 as f64,
                quote_lot: 10 as f64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintCookie {
                index: 11,
                decimals: 6,
                unit: 10u64.pow(6) as f64,
                base_lot: 100 as f64,
                quote_lot: 10 as f64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintCookie {
                index: 12,
                decimals: 6,
                unit: 10u64.pow(6) as f64,
                base_lot: 100 as f64,
                quote_lot: 10 as f64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintCookie {
                index: 13,
                decimals: 6,
                unit: 10u64.pow(6) as f64,
                base_lot: 100 as f64,
                quote_lot: 10 as f64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintCookie {
                index: 14,
                decimals: 6,
                unit: 10u64.pow(6) as f64,
                base_lot: 100 as f64,
                quote_lot: 10 as f64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintCookie {
                index: 15,
                decimals: 6,
                unit: 10u64.pow(6) as f64,
                base_lot: 0 as f64,
                quote_lot: 0 as f64,
                pubkey: None,
            }, // symbol: "USDC".to_string()
        ];

        let num_mints = config.num_mints as usize;
        let quote_index = num_mints - 1;
        let mut quote_mint = mints[(mints.len() - 1) as usize];
        let num_users = config.num_users as usize;
        // Make sure that the user defined length of mint list always have the quote_mint as last
        quote_mint.index = quote_index;
        mints[quote_index] = quote_mint;

        let mut test = ProgramTest::new("mango", mango_program_id, processor!(process_instruction));
        test.add_program("serum_dex", serum_program_id, processor!(process_serum_instruction));
        // TODO: add more programs (oracles)
        // limit to track compute unit increase
        test.set_bpf_compute_max_units(config.compute_limit);

        // Supress some of the logs
        solana_logger::setup_with_default(RUST_LOG_DEFAULT);

        // Add MNGO mint
        test.add_packable_account(
            mngo_token::ID,
            u32::MAX as u64,
            &Mint {
                is_initialized: true,
                mint_authority: COption::Some(Pubkey::new_unique()),
                decimals: 6,
                ..Mint::default()
            },
            &spl_token::id(),
        );
        // Add MSRM mint
        test.add_packable_account(
            msrm_token::ID,
            u32::MAX as u64,
            &Mint {
                is_initialized: true,
                mint_authority: COption::Some(Pubkey::new_unique()),
                decimals: 6,
                ..Mint::default()
            },
            &spl_token::id(),
        );

        // Add mints in loop
        for mint_index in 0..num_mints {
            let mint_pk: Pubkey;
            if mints[mint_index].pubkey.is_none() {
                mint_pk = Pubkey::new_unique();
            } else {
                mint_pk = mints[mint_index].pubkey.unwrap();
            }

            test.add_packable_account(
                mint_pk,
                u32::MAX as u64,
                &Mint {
                    is_initialized: true,
                    mint_authority: COption::Some(Pubkey::new_unique()),
                    decimals: mints[mint_index].decimals,
                    ..Mint::default()
                },
                &spl_token::id(),
            );
            mints[mint_index].pubkey = Some(mint_pk);
        }

        // add users in loop
        let mut users = Vec::new();
        let mut token_accounts = Vec::new();
        for _ in 0..num_users {
            let user_key = Keypair::new();
            test.add_account(
                user_key.pubkey(),
                solana_sdk::account::Account::new(
                    u32::MAX as u64,
                    0,
                    &solana_sdk::system_program::id(),
                ),
            );

            // give every user 10^18 (< 2^60) of every token
            // ~~ 1 trillion in case of 6 decimals
            for mint_index in 0..num_mints {
                let token_key = Pubkey::new_unique();
                test.add_packable_account(
                    token_key,
                    u32::MAX as u64,
                    &spl_token::state::Account {
                        mint: mints[mint_index].pubkey.unwrap(),
                        owner: user_key.pubkey(),
                        amount: 1_000_000_000_000_000_000,
                        state: spl_token::state::AccountState::Initialized,
                        ..spl_token::state::Account::default()
                    },
                    &spl_token::id(),
                );

                token_accounts.push(token_key);
            }
            users.push(user_key);
        }

        let mut context = test.start_with_context().await;
        let rent = context.banks_client.get_rent().await.unwrap();
        mints = mints[..num_mints].to_vec();

        Self {
            context,
            rent,
            mango_program_id,
            serum_program_id,
            num_mints,
            quote_index,
            quote_mint,
            mints,
            num_users,
            users,
            token_accounts,
            consume_perp_events_count: config.consume_perp_events_count,
        }
    }

    #[allow(dead_code)]
    pub async fn process_transaction(
        &mut self,
        instructions: &[Instruction],
        signers: Option<&[&Keypair]>,
    ) -> Result<(), TransportError> {
        let mut transaction =
            Transaction::new_with_payer(&instructions, Some(&self.context.payer.pubkey()));

        let mut all_signers = vec![&self.context.payer];

        if let Some(signers) = signers {
            all_signers.extend_from_slice(signers);
        }

        // This fails when warping is involved - https://gitmemory.com/issue/solana-labs/solana/18201/868325078
        // let recent_blockhash = self.context.banks_client.get_recent_blockhash().await.unwrap();

        transaction.sign(&all_signers, self.context.last_blockhash);

        self.context.banks_client.process_transaction(transaction).await
    }

    #[allow(dead_code)]
    pub async fn get_account(&mut self, address: Pubkey) -> solana_sdk::account::Account {
        return self.context.banks_client.get_account(address).await.unwrap().unwrap();
    }

    #[allow(dead_code)]
    pub fn get_payer_pk(&mut self) -> Pubkey {
        return self.context.payer.pubkey();
    }

    #[allow(dead_code)]
    pub async fn get_lamport_balance(&mut self, address: Pubkey) -> u64 {
        self.context.banks_client.get_account(address).await.unwrap().unwrap().lamports()
    }

    #[allow(dead_code)]
    pub async fn get_token_balance(&mut self, address: Pubkey) -> u64 {
        let token = self.context.banks_client.get_account(address).await.unwrap().unwrap();
        return spl_token::state::Account::unpack(&token.data[..]).unwrap().amount;
    }

    #[allow(dead_code)]
    pub async fn get_oo_info(
        &mut self,
        mango_group_cookie: &MangoGroupCookie,
        user_index: usize,
        mint_index: usize,
    ) -> (I80F48, I80F48, I80F48, I80F48) {
        let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
        let mut oo = self.get_account(mango_account.spot_open_orders[0]).await;
        let clock = self.get_clock().await;
        let acc = solana_program::account_info::AccountInfo::new(
            &mango_account.spot_open_orders[mint_index],
            false,
            false,
            &mut oo.lamports,
            &mut oo.data,
            &mango_group_cookie.mango_accounts[user_index].address,
            false,
            clock.epoch,
        );
        let open_orders = load_open_orders(&acc).unwrap();
        let (quote_free, quote_locked, base_free, base_locked) = split_open_orders(&open_orders);
        return (quote_free, quote_locked, base_free, base_locked);
    }

    #[allow(dead_code)]
    pub async fn create_account(&mut self, size: usize, owner: &Pubkey) -> Pubkey {
        let keypair = Keypair::new();
        let rent = self.rent.minimum_balance(size);

        let instructions = [system_instruction::create_account(
            &self.context.payer.pubkey(),
            &keypair.pubkey(),
            rent as u64,
            size as u64,
            owner,
        )];

        self.process_transaction(&instructions, Some(&[&keypair])).await.unwrap();

        return keypair.pubkey();
    }

    #[allow(dead_code)]
    pub async fn create_mint(&mut self, mint_authority: &Pubkey) -> Pubkey {
        let keypair = Keypair::new();
        let rent = self.rent.minimum_balance(Mint::LEN);

        let instructions = [
            system_instruction::create_account(
                &self.context.payer.pubkey(),
                &keypair.pubkey(),
                rent,
                Mint::LEN as u64,
                &spl_token::id(),
            ),
            spl_token::instruction::initialize_mint(
                &spl_token::id(),
                &keypair.pubkey(),
                &mint_authority,
                None,
                0,
            )
            .unwrap(),
        ];

        self.process_transaction(&instructions, Some(&[&keypair])).await.unwrap();

        return keypair.pubkey();
    }

    #[allow(dead_code)]
    pub async fn create_token_account(&mut self, owner: &Pubkey, mint: &Pubkey) -> Pubkey {
        let keypair = Keypair::new();
        let rent = self.rent.minimum_balance(spl_token::state::Account::LEN);

        let instructions = [
            system_instruction::create_account(
                &self.context.payer.pubkey(),
                &keypair.pubkey(),
                rent,
                spl_token::state::Account::LEN as u64,
                &spl_token::id(),
            ),
            spl_token::instruction::initialize_account(
                &spl_token::id(),
                &keypair.pubkey(),
                mint,
                owner,
            )
            .unwrap(),
        ];

        self.process_transaction(&instructions, Some(&[&keypair])).await.unwrap();
        return keypair.pubkey();
    }

    pub async fn load_account<T: Loadable>(&mut self, acc_pk: Pubkey) -> T {
        let mut acc = self.context.banks_client.get_account(acc_pk).await.unwrap().unwrap();
        let acc_info: AccountInfo = (&acc_pk, &mut acc).into();
        return *T::load(&acc_info).unwrap();
    }

    #[allow(dead_code)]
    pub async fn get_bincode_account<T: serde::de::DeserializeOwned>(
        &mut self,
        address: &Pubkey,
    ) -> T {
        self.context
            .banks_client
            .get_account(*address)
            .await
            .unwrap()
            .map(|a| deserialize::<T>(&a.data.borrow()).unwrap())
            .expect(format!("GET-TEST-ACCOUNT-ERROR: Account {}", address).as_str())
    }

    #[allow(dead_code)]
    pub async fn get_clock(&mut self) -> Clock {
        self.get_bincode_account::<Clock>(&sysvar::clock::id()).await
    }

    #[allow(dead_code)]
    pub async fn advance_clock_by_slots(&mut self, slots: u64) {
        let mut clock: Clock = self.get_clock().await;
        println!("clock slot before: {}", clock.slot);
        self.context.warp_to_slot(clock.slot + slots).unwrap();
        clock = self.get_clock().await;
        println!("clock slot after: {}", clock.slot);
    }

    #[allow(dead_code)]
    pub async fn advance_clock_past_timestamp(&mut self, unix_timestamp: UnixTimestamp) {
        let mut clock: Clock = self.get_clock().await;
        let mut n = 1;

        while clock.unix_timestamp <= unix_timestamp {
            // Since the exact time is not deterministic keep wrapping by arbitrary 400 slots until we pass the requested timestamp
            self.context.warp_to_slot(clock.slot + 400).unwrap();

            n = n + 1;
            clock = self.get_clock().await;
        }
    }

    #[allow(dead_code)]
    pub async fn advance_clock_by_min_timespan(&mut self, time_span: u64) {
        let clock: Clock = self.get_clock().await;
        self.advance_clock_past_timestamp(clock.unix_timestamp + (time_span as i64)).await;
    }

    #[allow(dead_code)]
    pub async fn advance_clock(&mut self) {
        let clock: Clock = self.get_clock().await;
        self.context.warp_to_slot(clock.slot + 2).unwrap();
    }

    #[allow(dead_code)]
    pub async fn with_mango_cache(&mut self, mango_group: &MangoGroup) -> (Pubkey, MangoCache) {
        let mango_cache = self.load_account::<MangoCache>(mango_group.mango_cache).await;
        return (mango_group.mango_cache, mango_cache);
    }

    #[allow(dead_code)]
    pub fn with_mint(&mut self, mint_index: usize) -> MintCookie {
        return self.mints[mint_index];
    }

    #[allow(dead_code)]
    pub fn with_user_token_account(&mut self, user_index: usize, mint_index: usize) -> Pubkey {
        return self.token_accounts[(user_index * self.num_mints) + mint_index];
    }

    #[allow(dead_code)]
    pub async fn with_mango_account_deposit(
        &mut self,
        mango_account_pk: &Pubkey,
        mint_index: usize,
    ) -> u64 {
        // self.mints last token index will not always be QUOTE_INDEX hence the check
        let actual_mint_index =
            if mint_index == self.quote_index { QUOTE_INDEX } else { mint_index };
        let mango_account = self.load_account::<MangoAccount>(*mango_account_pk).await;
        // TODO - make this use cached root bank deposit index instead
        return (mango_account.deposits[actual_mint_index] * INDEX_START).to_num();
    }

    #[allow(dead_code)]
    pub fn with_oracle_price(&mut self, base_mint: &MintCookie, price: f64) -> I80F48 {
        return I80F48::from_num(price) * I80F48::from_num(self.quote_mint.unit)
            / I80F48::from_num(base_mint.unit);
    }

    #[allow(dead_code)]
    pub async fn set_oracle(
        &mut self,
        mango_group: &MangoGroup,
        mango_group_pk: &Pubkey,
        oracle_pk: &Pubkey,
        oracle_price: I80F48,
    ) {
        let mango_program_id = self.mango_program_id;
        let instructions = [
            mango::instruction::set_oracle(
                &mango_program_id,
                &mango_group_pk,
                &oracle_pk,
                &self.context.payer.pubkey(),
                oracle_price,
            )
            .unwrap(),
            cache_prices(
                &mango_program_id,
                &mango_group_pk,
                &mango_group.mango_cache,
                &[*oracle_pk],
            )
            .unwrap(),
        ];
        self.process_transaction(&instructions, None).await.unwrap();
    }

    #[allow(dead_code)]
    pub fn base_size_number_to_lots(&mut self, mint: &MintCookie, quantity: f64) -> u64 {
        return ((quantity * mint.unit) / mint.base_lot) as u64;
    }

    #[allow(dead_code)]
    pub fn quote_size_number_to_lots(&mut self, mint: &MintCookie, price: f64, size: f64) -> u64 {
        let limit_price = self.price_number_to_lots(&mint, price);
        let max_coin_qty = self.base_size_number_to_lots(&mint, size);
        return mint.quote_lot as u64 * limit_price * max_coin_qty;
    }

    #[allow(dead_code)]
    pub fn price_number_to_lots(&mut self, mint: &MintCookie, price: f64) -> u64 {
        return ((price * self.quote_mint.unit * mint.base_lot) / (mint.unit * mint.quote_lot))
            as u64;
    }

    #[allow(dead_code)]
    pub fn to_native(&mut self, mint: &MintCookie, size: f64) -> I80F48 {
        return I80F48::from_num(mint.unit * size);
    }

    #[allow(dead_code)]
    pub fn to_native_fixedint(&mut self, mint: &MintCookie, size: I80F48) -> I80F48 {
        return I80F48::from_num(mint.unit) * size;
    }

    #[allow(dead_code)]
    pub async fn with_root_bank(
        &mut self,
        mango_group: &MangoGroup,
        mint_index: usize,
    ) -> (Pubkey, RootBank) {
        // self.mints last token index will not always be QUOTE_INDEX hence the check
        let actual_mint_index =
            if mint_index == self.quote_index { QUOTE_INDEX } else { mint_index };

        let root_bank_pk = mango_group.tokens[actual_mint_index].root_bank;
        let root_bank = self.load_account::<RootBank>(root_bank_pk).await;
        return (root_bank_pk, root_bank);
    }

    #[allow(dead_code)]
    pub async fn with_node_bank(
        &mut self,
        root_bank: &RootBank,
        bank_index: usize,
    ) -> (Pubkey, NodeBank) {
        let node_bank_pk = root_bank.node_banks[bank_index];
        let node_bank = self.load_account::<NodeBank>(node_bank_pk).await;
        return (node_bank_pk, node_bank);
    }

    #[allow(dead_code)]
    pub async fn place_perp_order(
        &mut self,
        mango_group_cookie: &MangoGroupCookie,
        perp_market_cookie: &PerpMarketCookie,
        user_index: usize,
        order_side: Side,
        order_size: u64,
        order_price: u64,
        order_id: u64,
        order_type: OrderType,
        reduce_only: bool,
    ) {
        let mango_program_id = self.mango_program_id;
        let mango_group = mango_group_cookie.mango_group;
        let mango_group_pk = mango_group_cookie.address;
        let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
        let mango_account_pk = mango_group_cookie.mango_accounts[user_index].address;
        let perp_market = perp_market_cookie.perp_market;
        let perp_market_pk = perp_market_cookie.address;

        let user = Keypair::from_base58_string(&self.users[user_index].to_base58_string());
        let instructions = [place_perp_order(
            &mango_program_id,
            &mango_group_pk,
            &mango_account_pk,
            &user.pubkey(),
            &mango_group.mango_cache,
            &perp_market_pk,
            &perp_market.bids,
            &perp_market.asks,
            &perp_market.event_queue,
            None,
            &mango_account.spot_open_orders,
            order_side,
            order_price as i64,
            order_size as i64,
            order_id,
            order_type,
            reduce_only,
        )
        .unwrap()];
        self.process_transaction(&instructions, Some(&[&user])).await.unwrap();
    }

    #[allow(dead_code)]
    pub async fn place_perp_order2(
        &mut self,
        mango_group_cookie: &MangoGroupCookie,
        perp_market_cookie: &PerpMarketCookie,
        user_index: usize,
        order_side: Side,
        order_size: u64,
        order_price: u64,
        order_id: u64,
        order_type: OrderType,
        reduce_only: bool,
        expiry_timestamp: Option<u64>,
    ) {
        let mango_program_id = self.mango_program_id;
        let mango_group = mango_group_cookie.mango_group;
        let mango_group_pk = mango_group_cookie.address;
        let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
        let mango_account_pk = mango_group_cookie.mango_accounts[user_index].address;
        let perp_market = perp_market_cookie.perp_market;
        let perp_market_pk = perp_market_cookie.address;

        let user = Keypair::from_base58_string(&self.users[user_index].to_base58_string());
        let open_orders_pks: Vec<Pubkey> = mango_account
            .spot_open_orders
            .iter()
            .enumerate()
            .filter_map(|(i, &pk)| if mango_account.in_margin_basket[i] { Some(pk) } else { None })
            .collect();

        let instructions = [place_perp_order2(
            &mango_program_id,
            &mango_group_pk,
            &mango_account_pk,
            &user.pubkey(),
            &mango_group.mango_cache,
            &perp_market_pk,
            &perp_market.bids,
            &perp_market.asks,
            &perp_market.event_queue,
            None,
            &open_orders_pks,
            order_side,
            order_price as i64,
            order_size as i64,
            order_id,
            order_type,
            reduce_only,
            expiry_timestamp,
        )
        .unwrap()];
        self.process_transaction(&instructions, Some(&[&user])).await.unwrap();
    }

    #[allow(dead_code)]
    pub async fn consume_perp_events(
        &mut self,
        mango_group_cookie: &MangoGroupCookie,
        perp_market_cookie: &PerpMarketCookie,
        mango_account_pks: &mut Vec<Pubkey>,
    ) {
        let mango_program_id = self.mango_program_id;
        let mango_group = mango_group_cookie.mango_group;
        let mango_group_pk = mango_group_cookie.address;
        let perp_market = perp_market_cookie.perp_market;
        let perp_market_pk = perp_market_cookie.address;

        let instructions = [consume_events(
            &mango_program_id,
            &mango_group_pk,
            &mango_group.mango_cache,
            &perp_market_pk,
            &perp_market.event_queue,
            &mut mango_account_pks[..],
            self.consume_perp_events_count,
        )
        .unwrap()];
        self.process_transaction(&instructions, None).await.unwrap();
    }

    #[allow(dead_code)]
    pub async fn force_cancel_perp_orders(
        &mut self,
        mango_group_cookie: &MangoGroupCookie,
        perp_market_cookie: &PerpMarketCookie,
        user_index: usize,
    ) {
        let mango_program_id = self.mango_program_id;
        let mango_group = mango_group_cookie.mango_group;
        let mango_group_pk = mango_group_cookie.address;
        let perp_market = perp_market_cookie.perp_market;
        let perp_market_pk = perp_market_cookie.address;
        let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
        let mango_account_pk = mango_group_cookie.mango_accounts[user_index].address;

        let instructions = [force_cancel_perp_orders(
            &mango_program_id,
            &mango_group_pk,
            &mango_group.mango_cache,
            &perp_market_pk,
            &perp_market.bids,
            &perp_market.asks,
            &mango_account_pk,
            &mango_account.spot_open_orders,
            20,
        )
        .unwrap()];
        self.process_transaction(&instructions, None).await.unwrap();
    }

    // pub fn get_pnl(
    //     &mut self,
    //     mango_group_cookie: &MangoGroupCookie,
    //     perp_market_cookie: &PerpMarketCookie,
    //     user_index: usize,
    // ) {
    //     let mango_cache = mango_group_cookie.mango_cache;
    //     let mint = perp_market_cookie.mint;
    //     let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
    //     let perp_account = mango_account.perp_accounts[mint.index];
    //     let price = mango_cache.price_cache[mint.index].price;
    //     return I80F48::from_num(perp_account.base_position) * I80F48::from_num(mint.base_lot) *
    //         price +
    //         I80F48::from_num(perp_account.quote_position);
    // }

    #[allow(dead_code)]
    pub async fn settle_perp_funds(
        &mut self,
        mango_group_cookie: &MangoGroupCookie,
        perp_market_cookie: &PerpMarketCookie,
        user_a_index: usize,
        user_b_index: usize,
    ) {
        let mango_program_id = self.mango_program_id;
        let mango_group = mango_group_cookie.mango_group;
        let mango_group_pk = mango_group_cookie.address;
        let mango_account_a_pk = mango_group_cookie.mango_accounts[user_a_index].address;
        let mango_account_b_pk = mango_group_cookie.mango_accounts[user_b_index].address;
        let mango_cache_pk = mango_group.mango_cache;
        let market_index = perp_market_cookie.mint.index;
        let (root_bank_pk, root_bank) = self.with_root_bank(&mango_group, self.quote_index).await;
        let (node_bank_pk, _node_bank) = self.with_node_bank(&root_bank, 0).await;

        let instructions = [settle_pnl(
            &mango_program_id,
            &mango_group_pk,
            &mango_account_a_pk,
            &mango_account_b_pk,
            &mango_cache_pk,
            &root_bank_pk,
            &node_bank_pk,
            market_index,
        )
        .unwrap()];

        self.process_transaction(&instructions, None).await.unwrap();
    }

    #[allow(dead_code)]
    pub fn create_dex_account(&mut self, unpadded_len: usize) -> (Keypair, Instruction) {
        let serum_program_id = self.serum_program_id;
        let key = Keypair::new();
        let len = unpadded_len + 12;
        let rent = self.rent.minimum_balance(len);
        let create_account_instr = solana_sdk::system_instruction::create_account(
            &self.context.payer.pubkey(),
            &key.pubkey(),
            rent,
            len as u64,
            &serum_program_id,
        );
        return (key, create_account_instr);
    }

    fn gen_listing_params(
        &mut self,
        _coin_mint: &Pubkey,
        _pc_mint: &Pubkey,
    ) -> (ListingKeys, Vec<Instruction>) {
        let serum_program_id = self.serum_program_id;
        // let payer_pk = &self.context.payer.pubkey();

        let (market_key, create_market) = self.create_dex_account(376);
        let (req_q_key, create_req_q) = self.create_dex_account(640);
        let (event_q_key, create_event_q) = self.create_dex_account(1 << 20);
        let (bids_key, create_bids) = self.create_dex_account(1 << 16);
        let (asks_key, create_asks) = self.create_dex_account(1 << 16);

        let (vault_signer_pk, vault_signer_nonce) =
            create_signer_key_and_nonce(&serum_program_id, &market_key.pubkey());

        let info = ListingKeys {
            market_key,
            req_q_key,
            event_q_key,
            bids_key,
            asks_key,
            vault_signer_pk,
            vault_signer_nonce,
        };
        let instructions =
            vec![create_market, create_req_q, create_event_q, create_bids, create_asks];
        return (info, instructions);
    }

    #[allow(dead_code)]
    pub async fn list_spot_market(&mut self, base_index: usize) -> SpotMarketCookie {
        let serum_program_id = self.serum_program_id;
        let coin_mint = self.mints[base_index].pubkey.unwrap();
        let pc_mint = self.mints[self.quote_index].pubkey.unwrap();
        let (listing_keys, mut instructions) = self.gen_listing_params(&coin_mint, &pc_mint);
        let ListingKeys {
            market_key,
            req_q_key,
            event_q_key,
            bids_key,
            asks_key,
            vault_signer_pk,
            vault_signer_nonce,
        } = listing_keys;

        let coin_vault = self.create_token_account(&vault_signer_pk, &coin_mint).await;
        let pc_vault = self.create_token_account(&listing_keys.vault_signer_pk, &pc_mint).await;

        let init_market_instruction = serum_dex::instruction::initialize_market(
            &market_key.pubkey(),
            &serum_program_id,
            &coin_mint,
            &pc_mint,
            &coin_vault,
            &pc_vault,
            None,
            None,
            &bids_key.pubkey(),
            &asks_key.pubkey(),
            &req_q_key.pubkey(),
            &event_q_key.pubkey(),
            self.mints[base_index].base_lot as u64,
            self.mints[base_index].quote_lot as u64,
            vault_signer_nonce,
            100,
        )
        .unwrap();

        instructions.push(init_market_instruction);

        let signers = vec![
            &market_key,
            &req_q_key,
            &event_q_key,
            &bids_key,
            &asks_key,
            &req_q_key,
            &event_q_key,
        ];

        self.process_transaction(&instructions, Some(&signers)).await.unwrap();

        SpotMarketCookie {
            market: market_key.pubkey(),
            req_q: req_q_key.pubkey(),
            event_q: event_q_key.pubkey(),
            bids: bids_key.pubkey(),
            asks: asks_key.pubkey(),
            coin_vault: coin_vault,
            pc_vault: pc_vault,
            vault_signer_key: vault_signer_pk,
            mint: self.with_mint(base_index),
        }
    }

    #[allow(dead_code)]
    pub async fn consume_spot_events(
        &mut self,
        spot_market_cookie: &SpotMarketCookie,
        open_orders: Vec<&Pubkey>,
        user_index: usize,
    ) {
        let serum_program_id = self.serum_program_id;
        let coin_fee_receivable_account =
            self.with_user_token_account(user_index, spot_market_cookie.mint.index);
        let pc_fee_receivable_account = self.with_user_token_account(user_index, self.quote_index);

        for open_order in open_orders {
            let instructions = [serum_dex::instruction::consume_events(
                &serum_program_id,
                vec![open_order],
                &spot_market_cookie.market,
                &spot_market_cookie.event_q,
                &coin_fee_receivable_account,
                &pc_fee_receivable_account,
                5,
            )
            .unwrap()];
            self.process_transaction(&instructions, None).await.unwrap();
        }
    }

    #[allow(dead_code)]
    pub async fn init_spot_open_orders(
        &mut self,
        mango_group_pk: &Pubkey,
        mango_group: &MangoGroup,
        mango_account_pk: &Pubkey,
        mango_account: &MangoAccount,
        user_index: usize,
        market_index: usize,
    ) -> Pubkey {
        let (orders_key, create_account_instr) =
            self.create_dex_account(size_of::<serum_dex::state::OpenOrders>());
        let open_orders_pk = orders_key.pubkey();
        let init_spot_open_orders_instruction = init_spot_open_orders(
            &self.mango_program_id,
            mango_group_pk,
            mango_account_pk,
            &mango_account.owner,
            &self.serum_program_id,
            &open_orders_pk,
            &mango_group.spot_markets[market_index].spot_market,
            &mango_group.signer_key,
        )
        .unwrap();

        let instructions = vec![create_account_instr, init_spot_open_orders_instruction];
        let user = Keypair::from_base58_string(&self.users[user_index].to_base58_string());
        let signers = vec![&user, &orders_key];
        self.process_transaction(&instructions, Some(&signers)).await.unwrap();
        open_orders_pk
    }

    #[allow(dead_code)]
    pub async fn create_mango_account(
        &mut self,
        mango_group_pk: &Pubkey,
        user_index: usize,
        account_num: u64,
        payer: Option<&Keypair>,
    ) -> Pubkey {
        let owner_key = &self.users[user_index];
        let owner_pk = owner_key.pubkey();
        let seeds: &[&[u8]] =
            &[&mango_group_pk.as_ref(), &owner_pk.as_ref(), &account_num.to_le_bytes()];
        let (mango_account_pk, _) = Pubkey::find_program_address(seeds, &self.mango_program_id);

        let mut instruction = create_mango_account(
            &self.mango_program_id,
            mango_group_pk,
            &mango_account_pk,
            &owner_pk,
            &solana_sdk::system_program::id(),
            &payer.map(|k| k.pubkey()).unwrap_or(owner_pk),
            account_num,
        )
        .unwrap();

        // Allow testing the compatibility case with no payer
        if payer.is_none() {
            instruction.accounts.pop();
            instruction.accounts[2].is_writable = true; // owner pays lamports
        }

        let instructions = vec![instruction];
        let owner_key_c = Keypair::from_base58_string(&owner_key.to_base58_string());
        let mut signers = vec![&owner_key_c];
        if let Some(payer_key) = payer {
            signers.push(payer_key);
        }
        self.process_transaction(&instructions, Some(&signers)).await.unwrap();
        mango_account_pk
    }

    #[allow(dead_code)]
    pub async fn create_spot_open_orders(
        &mut self,
        mango_group_pk: &Pubkey,
        mango_group: &MangoGroup,
        mango_account_pk: &Pubkey,
        user_index: usize,
        market_index: usize,
        payer: Option<&Keypair>,
    ) -> Pubkey {
        let open_orders_seeds: &[&[u8]] =
            &[&mango_account_pk.as_ref(), &market_index.to_le_bytes(), b"OpenOrders"];
        let (open_orders_pk, _) =
            Pubkey::find_program_address(open_orders_seeds, &self.mango_program_id);

        let owner_key = &self.users[user_index];
        let owner_pk = owner_key.pubkey();
        let mut instruction = create_spot_open_orders(
            &self.mango_program_id,
            mango_group_pk,
            mango_account_pk,
            &owner_pk,
            &self.serum_program_id,
            &open_orders_pk,
            &mango_group.spot_markets[market_index].spot_market,
            &mango_group.signer_key,
            &payer.map(|k| k.pubkey()).unwrap_or(owner_pk),
        )
        .unwrap();

        // Allow testing the compatibility case with no payer
        if payer.is_none() {
            instruction.accounts.pop();
            instruction.accounts[2].is_writable = true; // owner pays lamports
        }

        let instructions = vec![instruction];
        let owner_key_c = Keypair::from_bytes(&owner_key.to_bytes()).unwrap();
        let mut signers = vec![&owner_key_c];
        if let Some(payer_key) = payer {
            signers.push(payer_key);
        }
        self.process_transaction(&instructions, Some(&signers)).await.unwrap();
        open_orders_pk
    }

    #[allow(dead_code)]
    pub async fn init_open_orders(&mut self) -> Pubkey {
        let (orders_key, instruction) =
            self.create_dex_account(size_of::<serum_dex::state::OpenOrders>());

        let mut instructions = Vec::new();
        let orders_keypair = orders_key;
        instructions.push(instruction);

        let orders_pk = orders_keypair.pubkey();
        self.process_transaction(&instructions, Some(&[&orders_keypair])).await.unwrap();

        return orders_pk;
    }

    #[allow(dead_code)]
    pub async fn add_oracles_to_mango_group(&mut self, mango_group_pk: &Pubkey) -> Vec<Pubkey> {
        let mango_program_id = self.mango_program_id;
        let admin_pk = self.context.payer.pubkey();
        let mut oracle_pks = Vec::new();
        let mut instructions = Vec::new();
        for _ in 0..self.quote_index {
            let oracle_pk = self.create_account(size_of::<StubOracle>(), &mango_program_id).await;
            instructions.push(
                add_oracle(&mango_program_id, &mango_group_pk, &oracle_pk, &admin_pk).unwrap(),
            );
            oracle_pks.push(oracle_pk);
        }
        self.process_transaction(&instructions, None).await.unwrap();
        return oracle_pks;
    }

    #[allow(dead_code)]
    pub async fn cache_all_perp_markets(
        &mut self,
        mango_group: &MangoGroup,
        mango_group_pk: &Pubkey,
        perp_market_pks: &[Pubkey],
    ) {
        let mango_program_id = self.mango_program_id;
        let instructions = [cache_perp_markets(
            &mango_program_id,
            &mango_group_pk,
            &mango_group.mango_cache,
            &perp_market_pks,
        )
        .unwrap()];
        self.process_transaction(&instructions, None).await.unwrap();
    }

    pub async fn cache_all_root_banks(
        &mut self,
        mango_group: &MangoGroup,
        mango_group_pk: &Pubkey,
    ) {
        let mango_program_id = self.mango_program_id;
        let mut root_bank_pks = Vec::new();
        for token in mango_group.tokens.iter() {
            if token.root_bank != Pubkey::default() {
                root_bank_pks.push(token.root_bank);
            }
        }

        let instructions = [cache_root_banks(
            &mango_program_id,
            &mango_group_pk,
            &mango_group.mango_cache,
            &root_bank_pks,
        )
        .unwrap()];
        self.process_transaction(&instructions, None).await.unwrap();
    }

    pub async fn cache_all_prices(
        &mut self,
        mango_group: &MangoGroup,
        mango_group_pk: &Pubkey,
        oracle_pks: &[Pubkey],
    ) {
        let mango_program_id = self.mango_program_id;
        let instructions = [cache_prices(
            &mango_program_id,
            &mango_group_pk,
            &mango_group.mango_cache,
            &oracle_pks,
        )
        .unwrap()];
        self.process_transaction(&instructions, None).await.unwrap();
    }

    pub async fn update_all_root_banks(
        &mut self,
        mango_group: &MangoGroup,
        mango_group_pk: &Pubkey,
    ) {
        let mango_program_id = self.mango_program_id;
        for mint_index in 0..self.num_mints {
            let root_bank_pk = mango_group.tokens[mint_index].root_bank;
            if root_bank_pk != Pubkey::default() {
                let (root_bank_pk, root_bank) = self.with_root_bank(mango_group, mint_index).await;
                let (node_bank_pk, _node_bank) = self.with_node_bank(&root_bank, 0).await;

                let instructions = [update_root_bank(
                    &mango_program_id,
                    &mango_group_pk,
                    &mango_group.mango_cache,
                    &root_bank_pk,
                    &[node_bank_pk],
                )
                .unwrap()];
                self.process_transaction(&instructions, None).await.unwrap();
            }
        }
    }

    pub async fn update_funding(
        &mut self,
        mango_group_cookie: &MangoGroupCookie,
        perp_market_cookie: &PerpMarketCookie,
    ) {
        let mango_group = mango_group_cookie.mango_group;
        let mango_group_pk = mango_group_cookie.address;
        let mango_program_id = self.mango_program_id;
        let perp_market = perp_market_cookie.perp_market;
        let perp_market_pk = perp_market_cookie.address;

        let instructions = [update_funding(
            &mango_program_id,
            &mango_group_pk,
            &mango_group.mango_cache,
            &perp_market_pk,
            &perp_market.bids,
            &perp_market.asks,
        )
        .unwrap()];
        self.process_transaction(&instructions, None).await.unwrap();
    }

    #[allow(dead_code)]
    pub async fn place_spot_order(
        &mut self,
        mango_group_cookie: &MangoGroupCookie,
        spot_market_cookie: &SpotMarketCookie,
        user_index: usize,
        order: NewOrderInstructionV3,
    ) {
        let mango_program_id = self.mango_program_id;
        let serum_program_id = self.serum_program_id;
        let mango_group = mango_group_cookie.mango_group;
        let mango_group_pk = mango_group_cookie.address;
        let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
        let mango_account_pk = mango_group_cookie.mango_accounts[user_index].address;
        let mint_index = spot_market_cookie.mint.index;

        let user = Keypair::from_base58_string(&self.users[user_index].to_base58_string());

        let (signer_pk, _signer_nonce) =
            create_signer_key_and_nonce(&mango_program_id, &mango_group_pk);

        let (mint_root_bank_pk, mint_root_bank) =
            self.with_root_bank(&mango_group, mint_index).await;
        let (mint_node_bank_pk, mint_node_bank) = self.with_node_bank(&mint_root_bank, 0).await;
        let (quote_root_bank_pk, quote_root_bank) =
            self.with_root_bank(&mango_group, self.quote_index).await;
        let (quote_node_bank_pk, quote_node_bank) = self.with_node_bank(&quote_root_bank, 0).await;

        // Only pass in open orders if in margin basket or current market index, and
        // the only writable account should be OpenOrders for current market index
        let mut open_orders_pks = Vec::new();
        for x in 0..mango_account.spot_open_orders.len() {
            if x == mint_index && mango_account.spot_open_orders[x] == Pubkey::default() {
                open_orders_pks.push(
                    self.create_spot_open_orders(
                        &mango_group_pk,
                        &mango_group,
                        &mango_account_pk,
                        user_index,
                        x,
                        None,
                    )
                    .await,
                );
            } else {
                open_orders_pks.push(mango_account.spot_open_orders[x]);
            }
        }

        let (dex_signer_pk, _dex_signer_nonce) =
            create_signer_key_and_nonce(&serum_program_id, &spot_market_cookie.market);

        let instructions = [mango::instruction::place_spot_order(
            &mango_program_id,
            &mango_group_pk,
            &mango_account_pk,
            &user.pubkey(),
            &mango_group.mango_cache,
            &serum_program_id,
            &spot_market_cookie.market,
            &spot_market_cookie.bids,
            &spot_market_cookie.asks,
            &spot_market_cookie.req_q,
            &spot_market_cookie.event_q,
            &spot_market_cookie.coin_vault,
            &spot_market_cookie.pc_vault,
            &mint_root_bank_pk,
            &mint_node_bank_pk,
            &mint_node_bank.vault,
            &quote_root_bank_pk,
            &quote_node_bank_pk,
            &quote_node_bank.vault,
            &signer_pk,
            &dex_signer_pk,
            &mango_group.msrm_vault,
            &open_orders_pks, // oo ais
            mint_index,
            order,
        )
        .unwrap()];

        let signers = vec![&user];

        self.process_transaction(&instructions, Some(&signers)).await.unwrap();
    }

    #[allow(dead_code)]
    pub async fn place_spot_order_with_delegate(
        &mut self,
        mango_group_cookie: &MangoGroupCookie,
        spot_market_cookie: &SpotMarketCookie,
        user_index: usize,
        delegate_user_index: usize,
        order: NewOrderInstructionV3,
    ) -> Result<(), TransportError> {
        let mango_program_id = self.mango_program_id;
        let serum_program_id = self.serum_program_id;
        let mango_group = mango_group_cookie.mango_group;
        let mango_group_pk = mango_group_cookie.address;
        let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
        let mango_account_pk = mango_group_cookie.mango_accounts[user_index].address;
        let mint_index = spot_market_cookie.mint.index;

        let delegate_user =
            Keypair::from_base58_string(&self.users[delegate_user_index].to_base58_string());

        let (signer_pk, _signer_nonce) =
            create_signer_key_and_nonce(&mango_program_id, &mango_group_pk);

        let (mint_root_bank_pk, mint_root_bank) =
            self.with_root_bank(&mango_group, mint_index).await;
        let (mint_node_bank_pk, mint_node_bank) = self.with_node_bank(&mint_root_bank, 0).await;
        let (quote_root_bank_pk, quote_root_bank) =
            self.with_root_bank(&mango_group, self.quote_index).await;
        let (quote_node_bank_pk, quote_node_bank) = self.with_node_bank(&quote_root_bank, 0).await;

        // Only pass in open orders if in margin basket or current market index, and
        // the only writable account should be OpenOrders for current market index
        let mut open_orders_pks = Vec::new();
        for x in 0..mango_account.spot_open_orders.len() {
            if x == mint_index && mango_account.spot_open_orders[x] == Pubkey::default() {
                open_orders_pks.push(
                    self.create_spot_open_orders(
                        &mango_group_pk,
                        &mango_group,
                        &mango_account_pk,
                        user_index,
                        x,
                        None,
                    )
                    .await,
                );
            } else {
                open_orders_pks.push(mango_account.spot_open_orders[x]);
            }
        }

        let (dex_signer_pk, _dex_signer_nonce) =
            create_signer_key_and_nonce(&serum_program_id, &spot_market_cookie.market);

        let instructions = [mango::instruction::place_spot_order(
            &mango_program_id,
            &mango_group_pk,
            &mango_account_pk,
            &delegate_user.pubkey(),
            &mango_group.mango_cache,
            &serum_program_id,
            &spot_market_cookie.market,
            &spot_market_cookie.bids,
            &spot_market_cookie.asks,
            &spot_market_cookie.req_q,
            &spot_market_cookie.event_q,
            &spot_market_cookie.coin_vault,
            &spot_market_cookie.pc_vault,
            &mint_root_bank_pk,
            &mint_node_bank_pk,
            &mint_node_bank.vault,
            &quote_root_bank_pk,
            &quote_node_bank_pk,
            &quote_node_bank.vault,
            &signer_pk,
            &dex_signer_pk,
            &mango_group.msrm_vault,
            &open_orders_pks, // oo ais
            mint_index,
            order,
        )
        .unwrap()];

        let signers = vec![&delegate_user];

        self.process_transaction(&instructions, Some(&signers)).await
    }

    #[allow(dead_code)]
    pub async fn settle_spot_funds(
        &mut self,
        mango_group_cookie: &MangoGroupCookie,
        spot_market_cookie: &SpotMarketCookie,
        user_index: usize,
    ) {
        let mango_program_id = self.mango_program_id;
        let serum_program_id = self.serum_program_id;
        let mango_group = mango_group_cookie.mango_group;
        let mango_group_pk = mango_group_cookie.address;
        let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
        let mango_account_pk = mango_group_cookie.mango_accounts[user_index].address;
        let mint_index = spot_market_cookie.mint.index;

        let user = Keypair::from_base58_string(&self.users[user_index].to_base58_string());

        let (signer_pk, _signer_nonce) =
            create_signer_key_and_nonce(&mango_program_id, &mango_group_pk);

        let (base_root_bank_pk, base_root_bank) =
            self.with_root_bank(&mango_group, mint_index).await;
        let (base_node_bank_pk, base_node_bank) = self.with_node_bank(&base_root_bank, 0).await;
        let (quote_root_bank_pk, quote_root_bank) =
            self.with_root_bank(&mango_group, self.quote_index).await;
        let (quote_node_bank_pk, quote_node_bank) = self.with_node_bank(&quote_root_bank, 0).await;

        let (dex_signer_pk, _dex_signer_nonce) =
            create_signer_key_and_nonce(&serum_program_id, &spot_market_cookie.market);

        let instructions = [mango::instruction::settle_funds(
            &mango_program_id,
            &mango_group_pk,
            &mango_group.mango_cache,
            &user.pubkey(),
            &mango_account_pk,
            &serum_program_id,
            &spot_market_cookie.market,
            &mango_account.spot_open_orders[mint_index],
            &signer_pk,
            &spot_market_cookie.coin_vault,
            &spot_market_cookie.pc_vault,
            &base_root_bank_pk,
            &base_node_bank_pk,
            &quote_root_bank_pk,
            &quote_node_bank_pk,
            &base_node_bank.vault,
            &quote_node_bank.vault,
            &dex_signer_pk,
        )
        .unwrap()];

        let signers = vec![&user];

        self.process_transaction(&instructions, Some(&signers)).await.unwrap();
    }

    #[allow(dead_code)]
    pub async fn perform_deposit(
        &mut self,
        mango_group_cookie: &MangoGroupCookie,
        user_index: usize,
        mint_index: usize,
        amount: u64,
    ) {
        let mango_program_id = self.mango_program_id;
        let mango_group = mango_group_cookie.mango_group;
        let mango_group_pk = mango_group_cookie.address;
        let mango_account_pk = mango_group_cookie.mango_accounts[user_index].address;

        let user = Keypair::from_base58_string(&self.users[user_index].to_base58_string());
        let user_token_account = self.with_user_token_account(user_index, mint_index);

        let (root_bank_pk, root_bank) = self.with_root_bank(&mango_group, mint_index).await;
        let (node_bank_pk, node_bank) = self.with_node_bank(&root_bank, 0).await;

        let instructions = [deposit(
            &mango_program_id,
            &mango_group_pk,
            &mango_account_pk,
            &user.pubkey(),
            &mango_group.mango_cache,
            &root_bank_pk,
            &node_bank_pk,
            &node_bank.vault,
            &user_token_account,
            amount,
        )
        .unwrap()];
        self.process_transaction(&instructions, Some(&[&user])).await.unwrap();
    }

    #[allow(dead_code)]
    pub async fn perform_withdraw(
        &mut self,
        mango_group_cookie: &MangoGroupCookie,
        user_index: usize,
        mint_index: usize,
        quantity: u64,
        allow_borrow: bool,
    ) {
        let mango_program_id = self.mango_program_id;
        let mango_group = mango_group_cookie.mango_group;
        let mango_group_pk = mango_group_cookie.address;
        let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
        let mango_account_pk = mango_group_cookie.mango_accounts[user_index].address;

        let user = Keypair::from_base58_string(&self.users[user_index].to_base58_string());
        let user_token_account = self.with_user_token_account(user_index, mint_index);

        let (signer_pk, _signer_nonce) =
            create_signer_key_and_nonce(&mango_program_id, &mango_group_pk);

        let (root_bank_pk, root_bank) = self.with_root_bank(&mango_group, mint_index).await;
        let (node_bank_pk, node_bank) = self.with_node_bank(&root_bank, 0).await; // Note: not sure if nb_index is ever anything else than 0

        let instructions = [withdraw(
            &mango_program_id,
            &mango_group_pk,
            &mango_account_pk,
            &user.pubkey(),
            &mango_group.mango_cache,
            &root_bank_pk,
            &node_bank_pk,
            &node_bank.vault,
            &user_token_account,
            &signer_pk,
            &mango_account.spot_open_orders,
            quantity,
            allow_borrow,
        )
        .unwrap()];
        self.process_transaction(&instructions, Some(&[&user])).await;
    }

    #[allow(dead_code)]
    pub async fn perform_withdraw_with_delegate(
        &mut self,
        mango_group_cookie: &MangoGroupCookie,
        user_index: usize,
        delegate_user_index: usize,
        mint_index: usize,
        quantity: u64,
        allow_borrow: bool,
    ) -> Result<(), TransportError> {
        let mango_program_id = self.mango_program_id;
        let mango_group = mango_group_cookie.mango_group;
        let mango_group_pk = mango_group_cookie.address;
        let mango_account = mango_group_cookie.mango_accounts[user_index].mango_account;
        let mango_account_pk = mango_group_cookie.mango_accounts[user_index].address;

        let user = Keypair::from_base58_string(&self.users[user_index].to_base58_string());
        let delegate_user =
            Keypair::from_base58_string(&self.users[delegate_user_index].to_base58_string());
        let user_token_account = self.with_user_token_account(user_index, mint_index);

        let (signer_pk, _signer_nonce) =
            create_signer_key_and_nonce(&mango_program_id, &mango_group_pk);

        let (root_bank_pk, root_bank) = self.with_root_bank(&mango_group, mint_index).await;
        let (node_bank_pk, node_bank) = self.with_node_bank(&root_bank, 0).await; // Note: not sure if nb_index is ever anything else than 0

        let instructions = [withdraw(
            &mango_program_id,
            &mango_group_pk,
            &mango_account_pk,
            &delegate_user.pubkey(),
            &mango_group.mango_cache,
            &root_bank_pk,
            &node_bank_pk,
            &node_bank.vault,
            &user_token_account,
            &signer_pk,
            &mango_account.spot_open_orders,
            quantity,
            allow_borrow,
        )
        .unwrap()];
        self.process_transaction(&instructions, Some(&[&delegate_user])).await
    }

    #[allow(dead_code)]
    pub async fn perform_set_delegate(
        &mut self,
        mango_group_cookie: &MangoGroupCookie,
        user_index: usize,
        delegate_user_index: usize,
    ) {
        let mango_program_id = self.mango_program_id;
        let mango_group_pk = mango_group_cookie.address;
        let mango_account_pk = mango_group_cookie.mango_accounts[user_index].address;

        let user = Keypair::from_base58_string(&self.users[user_index].to_base58_string());
        let delegate =
            Keypair::from_base58_string(&self.users[delegate_user_index].to_base58_string());

        let instructions = [set_delegate(
            &mango_program_id,
            &mango_group_pk,
            &mango_account_pk,
            &user.pubkey(),
            &delegate.pubkey(),
        )
        .unwrap()];
        self.process_transaction(&instructions, Some(&[&user])).await.unwrap();
    }

    #[allow(dead_code)]
    pub async fn perform_reset_delegate(
        &mut self,
        mango_group_cookie: &MangoGroupCookie,
        user_index: usize,
        delegate_user_index: usize,
    ) {
        let mango_program_id = self.mango_program_id;
        let mango_group_pk = mango_group_cookie.address;
        let mango_account_pk = mango_group_cookie.mango_accounts[user_index].address;

        let user = Keypair::from_base58_string(&self.users[user_index].to_base58_string());
        let delegate =
            Keypair::from_base58_string(&self.users[delegate_user_index].to_base58_string());

        let instructions = [set_delegate(
            &mango_program_id,
            &mango_group_pk,
            &mango_account_pk,
            &user.pubkey(),
            &Pubkey::default(),
        )
        .unwrap()];
        self.process_transaction(&instructions, Some(&[&user])).await.unwrap();
    }

    #[allow(dead_code)]
    pub async fn perform_liquidate_token_and_token(
        &mut self,
        mango_group_cookie: &mut MangoGroupCookie,
        liqee_index: usize,
        liqor_index: usize,
        asset_index: usize,
        liab_index: usize,
    ) {
        let mango_program_id = self.mango_program_id;
        let mango_group = mango_group_cookie.mango_group;
        let mango_group_pk = mango_group_cookie.address;
        let liqee_mango_account = mango_group_cookie.mango_accounts[liqee_index].mango_account;
        let liqee_mango_account_pk = mango_group_cookie.mango_accounts[liqee_index].address;
        let liqor_mango_account = mango_group_cookie.mango_accounts[liqor_index].mango_account;
        let liqor_mango_account_pk = mango_group_cookie.mango_accounts[liqor_index].address;

        let liqor = Keypair::from_base58_string(&self.users[liqor_index].to_base58_string());

        let (asset_root_bank_pk, asset_root_bank) =
            self.with_root_bank(&mango_group, asset_index).await;
        let (asset_node_bank_pk, _asset_node_bank) = self.with_node_bank(&asset_root_bank, 0).await;

        let (liab_root_bank_pk, liab_root_bank) =
            self.with_root_bank(&mango_group, liab_index).await;
        let (liab_node_bank_pk, _liab_node_bank) = self.with_node_bank(&liab_root_bank, 0).await;

        let max_liab_transfer = I80F48::from_num(10_000); // TODO: This needs to adapt to the situation probably

        let instructions = vec![mango::instruction::liquidate_token_and_token(
            &mango_program_id,
            &mango_group_pk,
            &mango_group.mango_cache,
            &liqee_mango_account_pk,
            &liqor_mango_account_pk,
            &liqor.pubkey(),
            &asset_root_bank_pk,
            &asset_node_bank_pk,
            &liab_root_bank_pk,
            &liab_node_bank_pk,
            &liqee_mango_account.spot_open_orders,
            &liqor_mango_account.spot_open_orders,
            max_liab_transfer,
        )
        .unwrap()];

        self.process_transaction(&instructions, Some(&[&liqor])).await.unwrap();

        mango_group_cookie.mango_accounts[liqee_index].mango_account =
            self.load_account::<MangoAccount>(liqee_mango_account_pk).await;

        mango_group_cookie.mango_accounts[liqor_index].mango_account =
            self.load_account::<MangoAccount>(liqor_mango_account_pk).await;
    }

    #[allow(dead_code)]
    pub async fn perform_liquidate_token_and_perp(
        &mut self,
        mango_group_cookie: &mut MangoGroupCookie,
        liqee_index: usize,
        liqor_index: usize,
        asset_type: AssetType,
        asset_index: usize,
        liab_type: AssetType,
        liab_index: usize,
        max_liab_transfer: I80F48,
    ) {
        let mango_program_id = self.mango_program_id;
        let mango_group = mango_group_cookie.mango_group;
        let mango_group_pk = mango_group_cookie.address;
        let liqee_mango_account = mango_group_cookie.mango_accounts[liqee_index].mango_account;
        let liqee_mango_account_pk = mango_group_cookie.mango_accounts[liqee_index].address;
        let liqor_mango_account = mango_group_cookie.mango_accounts[liqor_index].mango_account;
        let liqor_mango_account_pk = mango_group_cookie.mango_accounts[liqor_index].address;

        let liqor = Keypair::from_base58_string(&self.users[liqor_index].to_base58_string());

        let (root_bank_pk, root_bank) = self.with_root_bank(&mango_group, asset_index).await;
        let (node_bank_pk, _node_bank) = self.with_node_bank(&root_bank, 0).await;

        let instructions = vec![mango::instruction::liquidate_token_and_perp(
            &mango_program_id,
            &mango_group_pk,
            &mango_group.mango_cache,
            &liqee_mango_account_pk,
            &liqor_mango_account_pk,
            &liqor.pubkey(),
            &root_bank_pk,
            &node_bank_pk,
            &liqee_mango_account.spot_open_orders,
            &liqor_mango_account.spot_open_orders,
            asset_type,
            asset_index,
            liab_type,
            liab_index,
            max_liab_transfer,
        )
        .unwrap()];

        self.process_transaction(&instructions, Some(&[&liqor])).await.unwrap();

        mango_group_cookie.mango_accounts[liqee_index].mango_account =
            self.load_account::<MangoAccount>(liqee_mango_account_pk).await;

        mango_group_cookie.mango_accounts[liqor_index].mango_account =
            self.load_account::<MangoAccount>(liqor_mango_account_pk).await;
    }

    #[allow(dead_code)]
    pub async fn perform_liquidate_perp_market(
        &mut self,
        mango_group_cookie: &mut MangoGroupCookie,
        mint_index: usize,
        liqee_index: usize,
        liqor_index: usize,
        base_transfer_request: i64,
    ) {
        let mango_program_id = self.mango_program_id;
        let mango_group = mango_group_cookie.mango_group;
        let mango_group_pk = mango_group_cookie.address;
        let liqee_mango_account = mango_group_cookie.mango_accounts[liqee_index].mango_account;
        let liqee_mango_account_pk = mango_group_cookie.mango_accounts[liqee_index].address;
        let liqor_mango_account = mango_group_cookie.mango_accounts[liqor_index].mango_account;
        let liqor_mango_account_pk = mango_group_cookie.mango_accounts[liqor_index].address;

        let liqor = Keypair::from_base58_string(&self.users[liqor_index].to_base58_string());

        let instructions = vec![mango::instruction::liquidate_perp_market(
            &mango_program_id,
            &mango_group_pk,
            &mango_group.mango_cache,
            &mango_group.perp_markets[mint_index].perp_market.key(),
            &mango_group_cookie.perp_markets[mint_index].perp_market.event_queue.key(),
            &liqee_mango_account_pk,
            &liqor_mango_account_pk,
            &liqor.pubkey(),
            &liqee_mango_account.spot_open_orders,
            &liqor_mango_account.spot_open_orders,
            base_transfer_request,
        )
        .unwrap()];

        self.process_transaction(&instructions, Some(&[&liqor])).await.unwrap();

        mango_group_cookie.mango_accounts[liqee_index].mango_account =
            self.load_account::<MangoAccount>(liqee_mango_account_pk).await;

        mango_group_cookie.mango_accounts[liqor_index].mango_account =
            self.load_account::<MangoAccount>(liqor_mango_account_pk).await;
    }
}

fn process_serum_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    Ok(serum_dex::state::State::process(program_id, accounts, instruction_data)?)
}
