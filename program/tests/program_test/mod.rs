use std::convert::TryInto;

use mango::entrypoint::process_instruction;
use mango_common::Loadable;
use solana_program::{
    account_info::AccountInfo, program_option::COption, program_pack::Pack, pubkey::*, rent::*,
    system_instruction,
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

pub struct MangoProgramTestConfig {
    pub compute_limit: u64,
    pub num_users: u64,
    pub num_mints: u64,
}

impl MangoProgramTestConfig {
    pub fn default() -> Self {
        MangoProgramTestConfig { compute_limit: 10_000, num_users: 2, num_mints: 10 }
    }
}

pub struct MangoProgramTest {
    pub context: ProgramTestContext,
    pub rent: Rent,
    pub mango_program_id: Pubkey,
    pub serum_program_id: Pubkey,
    pub users: Vec<Keypair>,
    pub mints: Vec<Pubkey>,
    pub token_accounts: Vec<Pubkey>, // user x mint
}

impl MangoProgramTest {
    pub async fn start_new(config: &MangoProgramTestConfig) -> Self {
        let mango_program_id = Pubkey::new_unique();
        let serum_program_id = Pubkey::new_unique();

        let mut test = ProgramTest::new("mango", mango_program_id, processor!(process_instruction));

        // passing mango's process instruction just to satisfy the compiler
        test.add_program("serum_dex", serum_program_id, processor!(process_instruction));
        // TODO: add more programs (oracles)

        // limit to track compute unit increase
        test.set_bpf_compute_max_units(config.compute_limit);

        // add mints in loop
        let mut mints = Vec::new();
        for m in 0..config.num_mints {
            let mint_decimals = 6; // TODO use different decimals
            let mint_pk = Pubkey::new_unique();
            test.add_packable_account(
                mint_pk,
                u32::MAX as u64,
                &Mint {
                    is_initialized: true,
                    mint_authority: COption::Some(Pubkey::new_unique()),
                    decimals: mint_decimals,
                    ..Mint::default()
                },
                &spl_token::id(),
            );
            mints.push(mint_pk);
        }

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
            for mint_key in &mints {
                let token_key = Pubkey::new_unique();
                test.add_packable_account(
                    token_key,
                    u32::MAX as u64,
                    &spl_token::state::Account {
                        mint: *mint_key,
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
}
