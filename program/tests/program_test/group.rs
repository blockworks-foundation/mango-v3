// use super::*;
// use mango::{state::*, instruction::*, oracle::*, utils::*};
// use solana_program::account_info::AccountInfo;
// use solana_sdk::account::Account;
// use std::mem::size_of;
//
// pub async fn init_mango_group(test: &mut MangoProgramTest) -> (Pubkey, MangoGroup) {
//     let mango_program_id = test.mango_program_id;
//     let serum_program_id = test.serum_program_id;
//
//     let mango_group_pk = test.create_account(size_of::<MangoGroup>(), &mango_program_id).await;
//     let mango_cache_pk = test.create_account(size_of::<MangoCache>(), &mango_program_id).await;
//     let (signer_pk, signer_nonce) = create_signer_key_and_nonce(&mango_program_id, &mango_group_pk);
//
//     let quote_mint_pk = test.mints[0];
//     let quote_vault_pk = test.create_token_account(&signer_pk, &quote_mint_pk).await;
//     let quote_node_bank_pk = test.create_account(size_of::<NodeBank>(), &mango_program_id).await;
//     let quote_root_bank_pk = test.create_account(size_of::<RootBank>(), &mango_program_id).await;
//
//     let admin_pk = test.context.payer.pubkey();
//     let instructions = [mango::instruction::init_mango_group(
//         &mango_program_id,
//         &mango_group_pk,
//         &signer_pk,
//         &admin_pk,
//         &quote_mint_pk,
//         &quote_vault_pk,
//         &quote_node_bank_pk,
//         &quote_root_bank_pk,
//         &mango_cache_pk,
//         &serum_program_id,
//         signer_nonce,
//         5,
//     )
//     .unwrap()];
//
//     test.process_transaction(&instructions, None).await.unwrap();
//
//     let mango_group = test.load_account::<MangoGroup>(mango_group_pk).await;
//     return (mango_group_pk, mango_group);
// }
//
// pub async fn with_account(test: &mut MangoProgramTest) -> Pubkey {
//     let user_pk = Pubkey::new_unique();
//     return user_pk;
// }
//
// pub async fn with_mango_account(test: &mut MangoProgramTest, mango_group_pk: &Pubkey, user_pk: &Pubkey) -> (Pubkey, MangoAccount) {
//     let mango_program_id = test.mango_program_id;
//     let mango_account_pk = test.create_account(size_of::<MangoAccount>(), &mango_program_id).await;
//     let admin_pk = test.context.payer.pubkey();
//     let instructions = [mango::instruction::init_mango_account(
//         &mango_program_id,
//         &mango_group_pk,
//         &mango_account_pk,
//         &user_pk
//     )
//     .unwrap()];
//     test.process_transaction(&instructions, None).await.unwrap();
//     let mango_account = test.load_account::<MangoAccount>(mango_account_pk).await;
//     return (mango_account_pk, mango_account);
// }
//
// pub async fn add_oracles_to_mango_group(test: &mut MangoProgramTest, mango_group_pk: &Pubkey, num_oracles: u64) -> Vec<(Pubkey)> {
//     let mango_program_id = test.mango_program_id;
//     let oracle_pk = test.create_account(size_of::<StubOracle>(), &mango_program_id).await;
//     let admin_pk = test.context.payer.pubkey();
//     let mut oracle_pks = Vec::new();
//     for _ in 0..num_oracles {
//         add_oracle(&mango_program_id, &mango_group_pk, &oracle_pk, &admin_pk).unwrap();
//         oracle_pks.push(oracle_pk);
//     }
//     return (oracle_pks);
// }
