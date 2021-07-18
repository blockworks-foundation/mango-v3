// use mango::oracle::StubOracle;
// use safe_transmute::{self, to_bytes::transmute_one_to_bytes};
// use std::any::type_name;
// use std::mem::size_of;
//
// use fixed::types::I80F48;
// use flux_aggregator::borsh_state::BorshState;
// use flux_aggregator::borsh_utils;
// use flux_aggregator::state::{Aggregator, AggregatorConfig, Answer};
// use solana_program::program_option::COption;
// use solana_program::program_pack::Pack;
// use solana_program::pubkey::Pubkey;
// use solana_program_test::{BanksClient, ProgramTest};
//
// use solana_sdk::{
//     account::Account,
//     account_info::IntoAccountInfo,
//     instruction::Instruction,
//     signature::{Keypair, Signer},
// };
//
// use serum_dex::state::{AccountFlag, MarketState, ToAlignedBytes};
// use spl_token::state::{Account as Token, AccountState, Mint};
//
// use mango::instruction::init_mango_group;
// use mango::state::{MangoCache, MangoGroup, NodeBank, RootBank, ONE_I80F48, ZERO_I80F48};
// use mango::utils::create_signer_key_and_nonce;
// use solana_program::hash::Hash;
// use solana_sdk::transaction::Transaction;
//
// trait AddPacked {
//     fn add_packable_account<T: Pack>(
//         &mut self,
//         pubkey: Pubkey,
//         amount: u64,
//         data: &T,
//         owner: &Pubkey,
//     );
// }
//
// impl AddPacked for ProgramTest {
//     fn add_packable_account<T: Pack>(
//         &mut self,
//         pubkey: Pubkey,
//         amount: u64,
//         data: &T,
//         owner: &Pubkey,
//     ) {
//         let mut account = Account::new(amount, T::get_packed_len(), owner);
//         data.pack_into_slice(&mut account.data);
//         self.add_account(pubkey, account);
//     }
// }
//
// pub struct TestMint {
//     pub pubkey: Pubkey,
//     pub authority: Keypair,
//     pub decimals: u8,
// }
//
// pub fn add_mint(test: &mut ProgramTest, decimals: u8) -> TestMint {
//     let authority = Keypair::new();
//     let pubkey = Pubkey::new_unique();
//     test.add_packable_account(
//         pubkey,
//         u32::MAX as u64,
//         &Mint {
//             is_initialized: true,
//             mint_authority: COption::Some(authority.pubkey()),
//             decimals,
//             ..Mint::default()
//         },
//         &spl_token::id(),
//     );
//     TestMint { pubkey, authority, decimals }
// }
//
// pub struct TestDex {
//     pub pubkey: Pubkey,
// }
//
// pub fn add_dex_empty(
//     test: &mut ProgramTest,
//     base_mint: Pubkey,
//     quote_mint: Pubkey,
//     dex_prog_id: Pubkey,
// ) -> TestDex {
//     let pubkey = Pubkey::new_unique();
//     let mut acc = Account::new(u32::MAX as u64, 0, &dex_prog_id);
//     let ms = MarketState {
//         account_flags: (AccountFlag::Initialized | AccountFlag::Market).bits(),
//         own_address: pubkey.to_aligned_bytes(),
//         vault_signer_nonce: 0,
//         coin_mint: base_mint.to_aligned_bytes(),
//         pc_mint: quote_mint.to_aligned_bytes(),
//
//         coin_vault: Pubkey::new_unique().to_aligned_bytes(),
//         coin_deposits_total: 0,
//         coin_fees_accrued: 0,
//
//         pc_vault: Pubkey::new_unique().to_aligned_bytes(),
//         pc_deposits_total: 0,
//         pc_fees_accrued: 0,
//         pc_dust_threshold: 0,
//
//         req_q: Pubkey::new_unique().to_aligned_bytes(),
//         event_q: Pubkey::new_unique().to_aligned_bytes(),
//         bids: Pubkey::new_unique().to_aligned_bytes(),
//         asks: Pubkey::new_unique().to_aligned_bytes(),
//
//         coin_lot_size: 1,
//         pc_lot_size: 1,
//
//         fee_rate_bps: 1,
//         referrer_rebates_accrued: 0,
//     };
//     let head: &[u8; 5] = b"serum";
//     let tail: &[u8; 7] = b"padding";
//     let data = transmute_one_to_bytes(&ms);
//     let mut accdata = vec![];
//     accdata.extend(head);
//     accdata.extend(data);
//     accdata.extend(tail);
//     acc.data = accdata;
//
//     test.add_account(pubkey, acc);
//     TestDex { pubkey }
// }
//
// pub struct TestTokenAccount {
//     pub pubkey: Pubkey,
// }
//
// pub fn add_token_account(
//     test: &mut ProgramTest,
//     owner: Pubkey,
//     mint: Pubkey,
//     initial_balance: u64,
// ) -> TestTokenAccount {
//     let pubkey = Pubkey::new_unique();
//     test.add_packable_account(
//         pubkey,
//         u32::MAX as u64,
//         &Token {
//             mint,
//             owner,
//             amount: initial_balance,
//             state: AccountState::Initialized,
//             ..Token::default()
//         },
//         &spl_token::id(),
//     );
//     TestTokenAccount { pubkey }
// }
//
// pub struct TestAggregator {
//     pub name: String,
//     pub pubkey: Pubkey,
//     pub price: u64,
// }
//
// pub fn add_aggregator(
//     test: &mut ProgramTest,
//     name: &str,
//     decimals: u8,
//     price: u64,
//     owner: &Pubkey,
// ) -> TestAggregator {
//     let pubkey = Pubkey::new_unique();
//
//     let mut description = [0u8; 32];
//     let size = name.len().min(description.len());
//     description[0..size].copy_from_slice(&name.as_bytes()[0..size]);
//
//     let aggregator = Aggregator {
//         config: AggregatorConfig { description, decimals, ..AggregatorConfig::default() },
//         is_initialized: true,
//         answer: Answer {
//             median: price,
//             created_at: 1, // set to > 0 to initialize
//             ..Answer::default()
//         },
//         ..Aggregator::default()
//     };
//
//     let mut account =
//         Account::new(u32::MAX as u64, borsh_utils::get_packed_len::<Aggregator>(), &owner);
//     let account_info = (&pubkey, false, &mut account).into_account_info();
//     aggregator.save(&account_info).unwrap();
//     test.add_account(pubkey, account);
//
//     TestAggregator { name: name.to_string(), pubkey, price }
// }
//
// #[derive(Copy, Clone)]
// pub struct TestNodeBank {
//     pub pubkey: Pubkey,
//
//     pub deposits: I80F48,
//     pub borrows: I80F48,
//     pub vault: Pubkey,
// }
//
// pub fn add_node_bank(
//     test: &mut ProgramTest,
//     program_id: &Pubkey,
//     vault_pk: Pubkey,
// ) -> TestNodeBank {
//     let pubkey = Pubkey::new_unique();
//     test.add_account(pubkey, Account::new(u32::MAX as u64, size_of::<NodeBank>(), &program_id));
//
//     TestNodeBank { pubkey, vault: vault_pk, deposits: ZERO_I80F48, borrows: ZERO_I80F48 }
// }
//
// pub struct TestRootBank {
//     pub pubkey: Pubkey,
//
//     pub num_node_banks: usize,
//     pub node_banks: Vec<TestNodeBank>,
//     pub deposit_index: I80F48,
//     pub borrow_index: I80F48,
// }
//
// pub fn add_root_bank(
//     test: &mut ProgramTest,
//     program_id: &Pubkey,
//     node_bank: TestNodeBank,
// ) -> TestRootBank {
//     let pubkey = Pubkey::new_unique();
//     test.add_account(pubkey, Account::new(u32::MAX as u64, size_of::<RootBank>(), &program_id));
//
//     let node_banks = vec![node_bank];
//
//     TestRootBank {
//         num_node_banks: 1,
//         pubkey,
//         node_banks,
//         deposit_index: ONE_I80F48,
//         borrow_index: ONE_I80F48,
//     }
// }
//
// // Holds all of the dependencies for a MangoGroup
// pub struct TestMangoGroup {
//     pub program_id: Pubkey,
//     pub mango_group_pk: Pubkey,
//     pub signer_pk: Pubkey,
//     pub signer_nonce: u64,
//     pub admin_pk: Pubkey,
//     pub dex_program_pk: Pubkey,
//     pub mango_cache_pk: Pubkey,
//
//     pub num_tokens: usize,
//     pub num_oracles: usize, // Note: does not increase if there is a spot and perp market for same base token
//
//     pub tokens: Vec<TestMint>,
//     // pub oracles: Vec<TestAggregator>,
//     // Note: oracle used for perps mark price is same as the one for spot. This is not ideal so it may change
//
//     // Right now Serum dex spot markets. TODO make this general to an interface
//     // pub spot_markets: Vec<TestDex>,
//     pub root_banks: Vec<TestRootBank>,
//
//     pub valid_interval: u8,
// }
//
// impl TestMangoGroup {
//     pub fn init_mango_group(&self, payer: &Pubkey) -> Instruction {
//         init_mango_group(
//             &self.program_id,
//             &self.mango_group_pk,
//             &self.signer_pk,
//             payer,
//             &self.tokens[0].pubkey,
//             &self.root_banks[0].node_banks[0].vault,
//             &self.root_banks[0].node_banks[0].pubkey,
//             &self.root_banks[0].pubkey,
//             &self.mango_cache_pk,
//             &self.dex_program_pk,
//             self.signer_nonce,
//             5, // valid_interval
//         )
//         .unwrap()
//     }
// }
//
// pub fn add_mango_group_prodlike(test: &mut ProgramTest, program_id: Pubkey) -> TestMangoGroup {
//     let mango_group_pk = Pubkey::new_unique();
//     let (signer_pk, signer_nonce) = create_signer_key_and_nonce(&program_id, &mango_group_pk);
//     test.add_account(
//         mango_group_pk,
//         Account::new(u32::MAX as u64, size_of::<MangoGroup>(), &program_id),
//     );
//
//     let admin = Keypair::new();
//     let dex_program_pk = Pubkey::new_unique();
//
//     let mango_cache_pk = Pubkey::new_unique();
//     test.add_account(
//         mango_cache_pk,
//         Account::new(u32::MAX as u64, size_of::<MangoCache>(), &program_id),
//     );
//
//     let quote_mint = add_mint(test, 6);
//     let quote_vault = add_token_account(test, signer_pk, quote_mint.pubkey, 0);
//     let quote_node_bank = add_node_bank(test, &program_id, quote_vault.pubkey);
//     let quote_root_bank = add_root_bank(test, &program_id, quote_node_bank);
//
//     let tokens = vec![quote_mint];
//     let root_banks = vec![quote_root_bank];
//
//     TestMangoGroup {
//         program_id,
//         mango_group_pk,
//         signer_pk,
//         signer_nonce,
//         admin_pk: admin.pubkey(),
//         dex_program_pk,
//         mango_cache_pk,
//         tokens,
//         root_banks,
//         num_tokens: 1,
//         num_oracles: 0,
//         valid_interval: 5,
//     }
// }
//
// pub async fn add_mango_group(
//     test: &mut ProgramTest,
//     banks_client: &mut BanksClient,
//     payer: &Keypair,
//     recent_blockhash: &Hash,
//     program_id: &Pubkey,
//     admin: &Keypair,
// ) -> Pubkey {
//     let mango_group_pk = add_test_account_with_owner::<MangoGroup>(test, program_id);
//     let (signer_pk, signer_nonce) = create_signer_key_and_nonce(&program_id, &mango_group_pk);
//
//     let dex_program_pk = Pubkey::new_unique();
//     let mango_cache_pk = add_test_account_with_owner::<MangoCache>(test, program_id);
//
//     let quote_mint = add_mint(test, 6);
//     let quote_vault = add_token_account(test, signer_pk, quote_mint.pubkey, 0);
//     let quote_node_bank = add_node_bank(test, &program_id, quote_vault.pubkey);
//     let quote_root_bank = add_root_bank(test, &program_id, quote_node_bank);
//
//     let mut transaction = Transaction::new_with_payer(
//         &[init_mango_group(
//             program_id,
//             &mango_group_pk,
//             &signer_pk,
//             &admin.pubkey(),
//             &quote_mint.pubkey,
//             &quote_vault.pubkey,
//             &quote_node_bank.pubkey,
//             &quote_root_bank.pubkey,
//             &mango_cache_pk,
//             &dex_program_pk,
//             signer_nonce,
//             5,
//         )
//         .unwrap()],
//         Some(&payer.pubkey()),
//     );
//
//     transaction.sign(&[payer, admin], *recent_blockhash);
//     assert!(banks_client.process_transaction(transaction).await.is_ok());
//
//     mango_group_pk
// }
//
// #[allow(dead_code)] // Compiler complains about this even tho it is used
// pub async fn get_token_balance(banks_client: &mut BanksClient, pubkey: Pubkey) -> u64 {
//     let token: Account = banks_client.get_account(pubkey).await.unwrap().unwrap();
//
//     spl_token::state::Account::unpack(&token.data[..]).unwrap().amount
// }
//
// pub fn add_test_account_with_owner<T>(test: &mut ProgramTest, owner: &Pubkey) -> Pubkey {
//     let pk = Pubkey::new_unique();
//     let size = size_of::<T>();
//     println!("add_test_account_with_owner type={} size={}", type_name::<T>(), size);
//     test.add_account(pk, Account::new(u32::MAX as u64, size, owner));
//     return pk;
// }
//
// pub fn add_test_account_with_owner_and_extra_size<T>(
//     test: &mut ProgramTest,
//     owner: &Pubkey,
//     extra_size: usize,
// ) -> Pubkey {
//     let pk = Pubkey::new_unique();
//     let size = size_of::<T>() + extra_size;
//     println!("add_test_account_with_owner type={} size={}", type_name::<T>(), size);
//     test.add_account(pk, Account::new(u32::MAX as u64, size, owner));
//     return pk;
// }
