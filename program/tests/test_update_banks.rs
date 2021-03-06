// #![cfg(feature = "test-bpf")]
//
// mod helpers;
//
// use helpers::*;
// use solana_program::account_info::AccountInfo;
// use solana_program_test::*;
// use solana_sdk::{
//     account::Account,
//     pubkey::Pubkey,
//     signature::{Keypair, Signer},
//     transaction::Transaction,
// };
// use std::mem::size_of;
//
// use mango::instruction::cache_root_banks;
// use mango::{
//     entrypoint::process_instruction,
//     instruction::{deposit, init_mango_account, update_root_bank},
//     state::{MangoAccount, NodeBank, RootBank, QUOTE_INDEX},
// };
//
// #[tokio::test]
// async fn test_root_bank_update_succeeds() {
//     let program_id = Pubkey::new_unique();
//
//     let mut test = ProgramTest::new("mango", program_id, processor!(process_instruction));
//
//     // limit to track compute unit increase
//     test.set_bpf_compute_max_units(50_000);
//
//     let initial_amount = 2;
//     let deposit_amount = 1;
//
//     // setup mango group
//     let mango_group = add_mango_group_prodlike(&mut test, program_id);
//
//     // setup user account
//     let user = Keypair::new();
//     test.add_account(user.pubkey(), Account::new(u32::MAX as u64, 0, &user.pubkey()));
//
//     // setup user token accounts
//     let user_account =
//         add_token_account(&mut test, user.pubkey(), mango_group.tokens[0].pubkey, initial_amount);
//
//     let mango_account_pk = Pubkey::new_unique();
//     test.add_account(
//         mango_account_pk,
//         Account::new(u32::MAX as u64, size_of::<MangoAccount>(), &program_id),
//     );
//
//     let (mut banks_client, payer, recent_blockhash) = test.start().await;
//
//     {
//         let mut transaction = Transaction::new_with_payer(
//             &[
//                 mango_group.init_mango_group(&payer.pubkey()),
//                 init_mango_account(
//                     &program_id,
//                     &mango_group.mango_group_pk,
//                     &mango_account_pk,
//                     &user.pubkey(),
//                 )
//                 .unwrap(),
//                 cache_root_banks(
//                     &program_id,
//                     &mango_group.mango_group_pk,
//                     &mango_group.mango_cache_pk,
//                     &[mango_group.root_banks[0].pubkey],
//                 )
//                 .unwrap(),
//                 deposit(
//                     &program_id,
//                     &mango_group.mango_group_pk,
//                     &mango_account_pk,
//                     &user.pubkey(),
//                     &mango_group.mango_cache_pk,
//                     &mango_group.root_banks[0].pubkey,
//                     &mango_group.root_banks[0].node_banks[0].pubkey,
//                     &mango_group.root_banks[0].node_banks[0].vault,
//                     &user_account.pubkey,
//                     deposit_amount,
//                 )
//                 .unwrap(),
//             ],
//             Some(&payer.pubkey()),
//         );
//
//         transaction.sign(&[&payer, &user], recent_blockhash);
//
//         let result = banks_client.process_transaction(transaction).await;
//
//         let mut node_bank = banks_client
//             .get_account(mango_group.root_banks[0].node_banks[0].pubkey)
//             .await
//             .unwrap()
//             .unwrap();
//         let account_info: AccountInfo =
//             (&mango_group.root_banks[0].node_banks[0].pubkey, &mut node_bank).into();
//         let node_bank = NodeBank::load_mut_checked(&account_info, &program_id).unwrap();
//
//         assert_eq!(node_bank.deposits, 1);
//         assert_eq!(node_bank.borrows, 0);
//
//         // Test transaction succeeded
//         assert!(result.is_ok());
//     }
//
//     {
//         let node_bank_pks: Vec<Pubkey> =
//             mango_group.root_banks[0].node_banks.iter().map(|node_bank| node_bank.pubkey).collect();
//         let mut transaction = Transaction::new_with_payer(
//             &[update_root_bank(
//                 &program_id,
//                 &mango_group.mango_group_pk,
//                 &mango_group.root_banks[0].pubkey,
//                 &node_bank_pks.as_slice(),
//             )
//             .unwrap()],
//             Some(&payer.pubkey()),
//         );
//
//         transaction.sign(&[&payer], recent_blockhash);
//
//         let result = banks_client.process_transaction(transaction).await;
//
//         // Test transaction succeeded
//         assert!(result.is_ok());
//
//         let mut root_bank =
//             banks_client.get_account(mango_group.root_banks[0].pubkey).await.unwrap().unwrap();
//         let account_info: AccountInfo = (&mango_group.root_banks[0].pubkey, &mut root_bank).into();
//         let root_bank = RootBank::load_mut_checked(&account_info, &program_id).unwrap();
//
//         assert_eq!(root_bank.deposit_index, 1);
//         assert_eq!(root_bank.borrow_index, 1);
//     }
//
//     {
//         let node_bank_pks: Vec<Pubkey> = vec![];
//         let mut transaction = Transaction::new_with_payer(
//             &[update_root_bank(
//                 &program_id,
//                 &mango_group.mango_group_pk,
//                 &mango_group.root_banks[0].pubkey,
//                 &node_bank_pks.as_slice(),
//             )
//             .unwrap()],
//             Some(&payer.pubkey()),
//         );
//
//         transaction.sign(&[&payer], recent_blockhash);
//
//         let result = banks_client.process_transaction(transaction).await;
//
//         // Test transaction fails when no node bank accounts are passed in
//         assert!(result.is_err());
//     }
//
//     {
//         let node_bank_pks: Vec<Pubkey> = vec![Pubkey::new_unique()];
//         let mut transaction = Transaction::new_with_payer(
//             &[update_root_bank(
//                 &program_id,
//                 &mango_group.mango_group_pk,
//                 &mango_group.root_banks[0].pubkey,
//                 &node_bank_pks.as_slice(),
//             )
//             .unwrap()],
//             Some(&payer.pubkey()),
//         );
//
//         transaction.sign(&[&payer], recent_blockhash);
//
//         let result = banks_client.process_transaction(transaction).await;
//
//         // Test transaction fails when invalid node bank accounts are passed in
//         assert!(result.is_err());
//     }
// }
