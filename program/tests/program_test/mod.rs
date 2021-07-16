use std::convert::TryInto;
use std::mem::size_of;

use fixed::types::I80F48;
use mango_common::Loadable;
use solana_program::{
    account_info::AccountInfo, program_error::ProgramError, program_option::COption,
    program_pack::Pack, pubkey::*, rent::*, system_instruction,
};
use solana_program_test::*;
use solana_sdk::{
    account::Account,
    instruction::Instruction,
    signature::{Keypair, Signer},
    transaction::Transaction,
    transport::TransportError,
};
use spl_token::{state::*, *};

use mango::{
    entrypoint::*, ids::*, instruction::*, matching::*, oracle::*, queue::*, state::*, utils::*,
};

pub mod group;

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

#[derive(Copy, Clone)]
pub struct MintConfig {
    // pub symbol: String,
    pub index: usize,
    pub decimals: u8,
    pub unit: i64,
    pub base_lot: i64,
    pub quote_lot: i64,
    pub pubkey: Option<Pubkey>,
}

pub struct MangoProgramTestConfig {
    pub compute_limit: u64,
    pub num_users: u64,
    pub num_mints: u64,
}

impl MangoProgramTestConfig {
    pub fn default() -> Self {
        MangoProgramTestConfig { compute_limit: 50_000, num_users: 2, num_mints: 32 }
    }
}

pub struct MangoProgramTest {
    pub context: ProgramTestContext,
    pub rent: Rent,
    pub mango_program_id: Pubkey,
    pub serum_program_id: Pubkey,
    pub users: Vec<Keypair>,
    pub mints: Vec<MintConfig>,
    pub token_accounts: Vec<Pubkey>, // user x mint
}

impl MangoProgramTest {
    pub async fn start_new(config: &MangoProgramTestConfig) -> Self {
        let mango_program_id = Pubkey::new_unique();
        let serum_program_id = Pubkey::new_unique();

        // Note:: Maybe define a const array of all the mint params that replicate real life and use those
        let mut mints: Vec<MintConfig> = vec![
            // ("MNGO", 0, 6, 10i64.pow(6) as i64, 10 as i64),
            // ("MSRM", 0, 6, 10i64.pow(6) as i64, 10 as i64),
            MintConfig {
                index: 0,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 1,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 1000 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "ETH".to_string()
            MintConfig {
                index: 2,
                decimals: 9,
                unit: 10i64.pow(9) as i64,
                base_lot: 100000000 as i64,
                quote_lot: 100 as i64,
                pubkey: None,
            }, // symbol: "SOL".to_string()
            MintConfig {
                index: 3,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100000 as i64,
                quote_lot: 100 as i64,
                pubkey: None,
            }, // symbol: "SRM".to_string()
            MintConfig {
                index: 4,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 0 as i64,
                quote_lot: 0 as i64,
                pubkey: None,
            }, // symbol: "USDC".to_string()
            MintConfig {
                index: 5,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 6,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 7,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 8,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 9,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 10,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 11,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 12,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 13,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 14,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 15,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 16,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 17,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 18,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 19,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 20,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 21,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 22,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 23,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 24,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 25,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 26,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 27,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 28,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 29,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 30,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
            MintConfig {
                index: 31,
                decimals: 6,
                unit: 10i64.pow(6) as i64,
                base_lot: 100 as i64,
                quote_lot: 10 as i64,
                pubkey: None,
            }, // symbol: "BTC".to_string()
        ];

        let mut test = ProgramTest::new("mango", mango_program_id, processor!(process_instruction));

        // passing mango's process instruction just to satisfy the compiler
        test.add_program("serum_dex", serum_program_id, processor!(process_instruction));
        // TODO: add more programs (oracles)

        // limit to track compute unit increase
        test.set_bpf_compute_max_units(config.compute_limit);

        // add mints in loop
        // let mut mints = Vec::new();
        for m in 0..config.num_mints {
            let mint_pk = Pubkey::new_unique();
            test.add_packable_account(
                mint_pk,
                u32::MAX as u64,
                &Mint {
                    is_initialized: true,
                    mint_authority: COption::Some(Pubkey::new_unique()),
                    decimals: mints[m as usize].decimals,
                    ..Mint::default()
                },
                &spl_token::id(),
            );
            mints[m as usize].pubkey = Some(mint_pk);
        }

        // add msrm mint
        test.add_packable_account(
            msrm_token::ID,
            u32::MAX as u64,
            &Mint {
                is_initialized: true,
                mint_authority: COption::Some(Pubkey::new_unique()),
                decimals: 6 as u8,
                ..Mint::default()
            },
            &spl_token::id(),
        );

        // add mngo mint
        test.add_packable_account(
            mngo_token::ID,
            u32::MAX as u64,
            &Mint {
                is_initialized: true,
                mint_authority: COption::Some(Pubkey::new_unique()),
                decimals: 6 as u8,
                ..Mint::default()
            },
            &spl_token::id(),
        );

        // add users in loop
        let mut users = Vec::new();
        let mut token_accounts = Vec::new();
        for _ in 0..config.num_users {
            let user_key = Keypair::new();
            test.add_account(
                user_key.pubkey(),
                solana_sdk::account::Account::new(u32::MAX as u64, 0, &user_key.pubkey()),
            );

            // give every user 10^18 (< 2^60) of every token
            // ~~ 1 trillion in case of 6 decimals
            for mint in &mints {
                let token_key = Pubkey::new_unique();
                test.add_packable_account(
                    token_key,
                    u32::MAX as u64,
                    &spl_token::state::Account {
                        mint: mint.pubkey.unwrap(),
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

        Self { context, rent, mango_program_id, serum_program_id, mints, users, token_accounts }
    }

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

        let recent_blockhash = self.context.banks_client.get_recent_blockhash().await.unwrap();

        transaction.sign(&all_signers, recent_blockhash);

        self.context.banks_client.process_transaction(transaction).await.unwrap();

        Ok(())
    }

    pub async fn get_token_balance(&mut self, address: Pubkey) -> u64 {
        let mut token = self.context.banks_client.get_account(address).await.unwrap().unwrap();
        return spl_token::state::Account::unpack(&token.data[..]).unwrap().amount;
    }

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

    #[allow(dead_code)]
    pub async fn load_account<T: Loadable>(&mut self, acc_pk: Pubkey) -> T {
        let mut acc = self.context.banks_client.get_account(acc_pk).await.unwrap().unwrap();
        let acc_info: AccountInfo = (&acc_pk, &mut acc).into();
        return *T::load(&acc_info).unwrap();
    }

    #[allow(dead_code)]
    pub async fn with_mango_group(&mut self) -> (Pubkey, MangoGroup) {
        let mango_program_id = self.mango_program_id;
        let serum_program_id = self.serum_program_id;

        let mango_group_pk = self.create_account(size_of::<MangoGroup>(), &mango_program_id).await;
        let mango_cache_pk = self.create_account(size_of::<MangoCache>(), &mango_program_id).await;
        let (signer_pk, signer_nonce) =
            create_signer_key_and_nonce(&mango_program_id, &mango_group_pk);
        let admin_pk = self.context.payer.pubkey();

        let quote_mint_pk = self.mints[self.mints.len() - 1].pubkey.unwrap();
        let quote_vault_pk = self.create_token_account(&signer_pk, &quote_mint_pk).await;
        let quote_node_bank_pk =
            self.create_account(size_of::<NodeBank>(), &mango_program_id).await;
        let quote_root_bank_pk =
            self.create_account(size_of::<RootBank>(), &mango_program_id).await;
        let dao_vault_pk = self.create_token_account(&signer_pk, &quote_mint_pk).await;
        let msrm_vault_pk = self.create_token_account(&signer_pk, &msrm_token::ID).await;

        let quote_optimal_util = I80F48::from_num(0.7);
        let quote_optimal_rate = I80F48::from_num(0.06);
        let quote_max_rate = I80F48::from_num(1.5);

        let instructions = [mango::instruction::init_mango_group(
            &mango_program_id,
            &mango_group_pk,
            &signer_pk,
            &admin_pk,
            &quote_mint_pk,
            &quote_vault_pk,
            &quote_node_bank_pk,
            &quote_root_bank_pk,
            &dao_vault_pk,
            &msrm_vault_pk,
            &mango_cache_pk,
            &serum_program_id,
            signer_nonce,
            5,
            quote_optimal_util,
            quote_optimal_rate,
            quote_max_rate,
        )
        .unwrap()];

        self.process_transaction(&instructions, None).await.unwrap();

        let mango_group = self.load_account::<MangoGroup>(mango_group_pk).await;
        return (mango_group_pk, mango_group);
    }

    #[allow(dead_code)]
    pub async fn with_mango_account(
        &mut self,
        mango_group_pk: &Pubkey,
        index: usize,
    ) -> (Pubkey, MangoAccount) {
        let mango_program_id = self.mango_program_id;
        let mango_account_pk =
            self.create_account(size_of::<MangoAccount>(), &mango_program_id).await;
        let admin_pk = self.context.payer.pubkey();
        let user = Keypair::from_base58_string(&self.users[index].to_base58_string());
        let user_pk = user.pubkey();

        let instructions = [mango::instruction::init_mango_account(
            &mango_program_id,
            &mango_group_pk,
            &mango_account_pk,
            &user_pk,
        )
        .unwrap()];
        self.process_transaction(&instructions, Some(&[&user])).await.unwrap();
        let mango_account = self.load_account::<MangoAccount>(mango_account_pk).await;
        return (mango_account_pk, mango_account);
    }

    pub async fn with_mango_cache(&mut self, mango_group: &MangoGroup) -> (Pubkey, MangoCache) {
        let mango_cache = self.load_account::<MangoCache>(mango_group.mango_cache).await;
        return (mango_group.mango_cache, mango_cache);
    }

    pub fn with_mint(&mut self, mint_index: usize) -> MintConfig {
        return self.mints[mint_index];
    }

    #[allow(dead_code)]
    pub fn with_user_token_account(&mut self, user_index: usize, token_index: usize) -> Pubkey {
        return self.token_accounts[(user_index * self.mints.len()) + token_index];
    }

    #[allow(dead_code)]
    pub async fn with_oracles(&mut self, mango_group_pk: &Pubkey, num_oracles: u64) -> Vec<Pubkey> {
        let mango_program_id = self.mango_program_id;
        let oracle_pk = self.create_account(size_of::<StubOracle>(), &mango_program_id).await;
        let admin_pk = self.context.payer.pubkey();
        let mut oracle_pks = Vec::new();
        let mut instructions = Vec::new();
        for _ in 0..num_oracles {
            instructions.push(
                add_oracle(&mango_program_id, &mango_group_pk, &oracle_pk, &admin_pk).unwrap(),
            );
            oracle_pks.push(oracle_pk);
        }
        self.process_transaction(&instructions, None).await.unwrap();
        return oracle_pks;
    }

    pub fn with_oracle_price(
        &mut self,
        quote_mint: &MintConfig,
        base_mint: &MintConfig,
        price: u64,
    ) -> I80F48 {
        return I80F48::from_num(price) * I80F48::from_num(quote_mint.unit)
            / I80F48::from_num(base_mint.unit);
    }

    pub fn with_order_price(
        &mut self,
        quote_mint: &MintConfig,
        base_mint: &MintConfig,
        price: i64,
    ) -> i64 {
        return ((price) * quote_mint.unit * base_mint.base_lot)
            / (base_mint.unit * base_mint.quote_lot);
    }

    pub fn with_order_size(&mut self, base_mint: &MintConfig, quantity: i64) -> i64 {
        return (quantity * base_mint.unit) / base_mint.base_lot;
    }

    pub async fn with_root_bank(
        &mut self,
        mango_group: &MangoGroup,
        token_index: usize,
    ) -> (Pubkey, RootBank) {
        let root_bank_pk = mango_group.tokens[token_index].root_bank;
        let root_bank = self.load_account::<RootBank>(root_bank_pk).await;
        return (root_bank_pk, root_bank);
    }

    pub async fn with_node_bank(
        &mut self,
        root_bank: &RootBank,
        token_index: usize,
    ) -> (Pubkey, NodeBank) {
        let node_bank_pk = root_bank.node_banks[token_index];
        let node_bank = self.load_account::<NodeBank>(node_bank_pk).await;
        return (node_bank_pk, node_bank);
    }

    pub async fn with_perp_market(
        &mut self,
        mango_group_pk: &Pubkey,
        mint_index: usize,
        market_index: usize,
    ) -> (Pubkey, PerpMarket) {
        let mango_program_id = self.mango_program_id;
        let perp_market_pk = self.create_account(size_of::<PerpMarket>(), &mango_program_id).await;
        let (signer_pk, signer_nonce) =
            create_signer_key_and_nonce(&mango_program_id, &mango_group_pk);
        let max_num_events = 32;
        let event_queue_pk = self
            .create_account(
                size_of::<EventQueue>() + size_of::<AnyEvent>() * max_num_events,
                &mango_program_id,
            )
            .await;
        let bids_pk = self.create_account(size_of::<BookSide>(), &mango_program_id).await;
        let asks_pk = self.create_account(size_of::<BookSide>(), &mango_program_id).await;
        let mngo_vault_pk = self.create_token_account(&signer_pk, &mngo_token::ID).await;

        let admin_pk = self.context.payer.pubkey();

        let init_leverage = I80F48::from_num(10);
        let maint_leverage = init_leverage * 2;
        let maker_fee = I80F48::from_num(0.01);
        let taker_fee = I80F48::from_num(0.01);
        let max_depth_bps = I80F48::from_num(1);
        let scaler = I80F48::from_num(1);

        let instructions = [mango::instruction::add_perp_market(
            &mango_program_id,
            &mango_group_pk,
            &perp_market_pk,
            &event_queue_pk,
            &bids_pk,
            &asks_pk,
            &mngo_vault_pk,
            &admin_pk,
            market_index,
            maint_leverage,
            init_leverage,
            maker_fee,
            taker_fee,
            self.mints[mint_index].base_lot,
            self.mints[mint_index].quote_lot,
            max_depth_bps,
            scaler,
        )
        .unwrap()];

        self.process_transaction(&instructions, None).await.unwrap();

        let perp_market = self.load_account::<PerpMarket>(perp_market_pk).await;
        return (perp_market_pk, perp_market);
    }

    pub async fn perform_deposit(
        &mut self,
        mango_group: &MangoGroup,
        mango_group_pk: &Pubkey,
        mango_account_pk: &Pubkey,
        user_index: usize,
        token_index: usize,
        amount: u64,
    ) {
        let mango_program_id = self.mango_program_id;
        let user = Keypair::from_base58_string(&self.users[user_index].to_base58_string());
        let user_token_account = self.with_user_token_account(user_index, token_index);

        let (root_bank_pk, root_bank) = self.with_root_bank(mango_group, token_index).await;
        let (node_bank_pk, node_bank) = self.with_node_bank(&root_bank, 0).await; // Note: not sure if nb_index is ever anything else than 0

        let instructions = [
            cache_root_banks(
                &mango_program_id,
                &mango_group_pk,
                &mango_group.mango_cache,
                &[root_bank_pk],
            )
            .unwrap(),
            deposit(
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
            .unwrap(),
        ];
        self.process_transaction(&instructions, Some(&[&user])).await.unwrap();
        println!("Deposit success");
    }

    pub async fn place_perp_order(
        &mut self,
        mango_group: &MangoGroup,
        mango_group_pk: &Pubkey,
        mango_account: &MangoAccount,
        mango_account_pk: &Pubkey,
        perp_market: &PerpMarket,
        perp_market_pk: &Pubkey,
        order_side: Side,
        order_price: i64,
        order_size: i64,
        order_id: u64,
        order_type: OrderType,
        oracle_pk: &Pubkey,
        user_index: usize,
    ) -> Result<(), TransportError> {
        let mango_program_id = self.mango_program_id;
        let user = Keypair::from_base58_string(&self.users[user_index].to_base58_string());

        let instructions = [
            cache_prices(
                &mango_program_id,
                &mango_group_pk,
                &mango_group.mango_cache,
                &[*oracle_pk],
            )
            .unwrap(),
            cache_perp_markets(
                &mango_program_id,
                &mango_group_pk,
                &mango_group.mango_cache,
                &[*perp_market_pk],
            )
            .unwrap(),
            place_perp_order(
                &mango_program_id,
                &mango_group_pk,
                &mango_account_pk,
                &user.pubkey(),
                &mango_group.mango_cache,
                &perp_market_pk,
                &perp_market.bids,
                &perp_market.asks,
                &perp_market.event_queue,
                &mango_account.spot_open_orders,
                order_side,
                order_price,
                order_size,
                order_id,
                order_type,
            )
            .unwrap(),
        ];

        self.process_transaction(&instructions, Some(&[&user])).await.unwrap();
        Ok(())
    }
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

    pub async fn list_market(
        &mut self,
        base_index: usize,
        quote_index: usize,
    ) -> Result<MarketPubkeys, ProgramError> {
        let serum_program_id = self.serum_program_id;
        let coin_mint = self.mints[base_index].pubkey.unwrap();
        let pc_mint = self.mints[quote_index].pubkey.unwrap();
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
            &bids_key.pubkey(),
            &asks_key.pubkey(),
            &req_q_key.pubkey(),
            &event_q_key.pubkey(),
            self.mints[base_index].base_lot as u64,
            self.mints[base_index].quote_lot as u64,
            vault_signer_nonce,
            100,
        )?;

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

        Ok(MarketPubkeys {
            market: market_key.pubkey(),
            req_q: req_q_key.pubkey(),
            event_q: event_q_key.pubkey(),
            bids: bids_key.pubkey(),
            asks: asks_key.pubkey(),
            coin_vault: coin_vault,
            pc_vault: pc_vault,
            vault_signer_key: vault_signer_pk,
        })
    }
    pub async fn add_markets_to_mango_group(&mut self, mango_group_pk: &Pubkey) {
        let mango_program_id = self.mango_program_id;
        let serum_program_id = self.serum_program_id;

        let quote_index = self.mints.len() - 1;

        for mint_index in 0..quote_index {
            let market_pubkeys =
                self.list_market(mint_index as usize, quote_index as usize).await.unwrap();
            println!("Serum Market PK: {}", market_pubkeys.market.to_string());

            let (signer_pk, signer_nonce) =
                create_signer_key_and_nonce(&mango_program_id, &mango_group_pk);

            let vault_pk = self
                .create_token_account(&signer_pk, &self.mints[mint_index as usize].pubkey.unwrap())
                .await;
            let node_bank_pk = self.create_account(size_of::<NodeBank>(), &mango_program_id).await;
            let root_bank_pk = self.create_account(size_of::<RootBank>(), &mango_program_id).await;
            let init_leverage = I80F48::from_num(10);
            let maint_leverage = init_leverage * 2;
            let optimal_util = I80F48::from_num(0.7);
            let optimal_rate = I80F48::from_num(0.06);
            let max_rate = I80F48::from_num(1.5);

            let admin_pk = self.context.payer.pubkey();

            let instructions = [mango::instruction::add_spot_market(
                &mango_program_id,
                &mango_group_pk,
                &market_pubkeys.market,
                &serum_program_id,
                &self.mints[mint_index as usize].pubkey.unwrap(),
                &node_bank_pk,
                &vault_pk,
                &root_bank_pk,
                &admin_pk,
                mint_index as usize,
                maint_leverage,
                init_leverage,
                optimal_util,
                optimal_rate,
                max_rate,
            )
            .unwrap()];

            self.process_transaction(&instructions, None).await.unwrap();
        }
    }
}
