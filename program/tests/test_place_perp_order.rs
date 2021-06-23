// Tests related to placing orders on a perp market
mod helpers;
use arrayref::array_ref;
use bytemuck::cast_ref;
use fixed::types::I80F48;
use helpers::*;
use mango_common::Loadable;
use std::{mem::size_of, thread::sleep, time::Duration};

use mango::{
    entrypoint::process_instruction, instruction::*, matching::*, oracle::StubOracle, queue::*,
    state::*,
};
use solana_program::{account_info::AccountInfo, pubkey::Pubkey};
use solana_program_test::*;
use solana_sdk::account::ReadableAccount;
use solana_sdk::{
    account::Account, commitment_config::CommitmentLevel, signature::Keypair, signer::Signer,
    transaction::Transaction,
};

#[tokio::test]
async fn test_init_perp_market() {
    let program_id = Pubkey::new_unique();
    let mut test = ProgramTest::new("mango", program_id, processor!(process_instruction));

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(50_000);

    let mango_group = add_mango_group_prodlike(&mut test, program_id);
    let mango_group_pk = mango_group.mango_group_pk;

    let user = Keypair::new();
    test.add_account(user.pubkey(), Account::new(u32::MAX as u64, 0, &user.pubkey()));

    let quote_index = 0;
    let quote_decimals = 6;
    let quote_unit = 10i64.pow(quote_decimals);
    let quote_lot = 10;

    let mango_account_pk = add_test_account_with_owner::<MangoAccount>(&mut test, &program_id);

    let oracle_pk = add_test_account_with_owner::<StubOracle>(&mut test, &program_id);
    let base_decimals = 4;
    let base_price = 420;
    let base_unit = 10i64.pow(base_decimals);
    let base_lot = 100;
    let oracle_price =
        I80F48::from_num(base_price) * I80F48::from_num(quote_unit) / I80F48::from_num(base_unit);

    let perp_market_idx = 0;
    let perp_market_pk = add_test_account_with_owner::<PerpMarket>(&mut test, &program_id);

    let event_queue_pk = add_test_account_with_owner_and_extra_size::<EventQueue>(
        &mut test,
        &program_id,
        size_of::<AnyEvent>() * 32,
    );

    let bids_pk = add_test_account_with_owner::<BookSide>(&mut test, &program_id);
    let asks_pk = add_test_account_with_owner::<BookSide>(&mut test, &program_id);

    let admin = Keypair::new();

    let (mut banks_client, payer, recent_blockhash) = test.start().await;

    let init_leverage = I80F48::from_num(10);
    let maint_leverage = init_leverage * 2;
    // setup mango group, perp market & mango account
    {
        let mut transaction = Transaction::new_with_payer(
            &[
                mango_group.init_mango_group(&admin.pubkey()),
                init_mango_account(&program_id, &mango_group_pk, &mango_account_pk, &user.pubkey())
                    .unwrap(),
                add_oracle(&program_id, &mango_group_pk, &oracle_pk, &admin.pubkey()).unwrap(),
                set_oracle(&program_id, &mango_group_pk, &oracle_pk, &admin.pubkey(), oracle_price)
                    .unwrap(),
                add_perp_market(
                    &program_id,
                    &mango_group_pk,
                    &perp_market_pk,
                    &event_queue_pk,
                    &bids_pk,
                    &asks_pk,
                    &admin.pubkey(),
                    perp_market_idx,
                    maint_leverage,
                    init_leverage,
                    base_lot,
                    quote_lot,
                )
                .unwrap(),
            ],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[&payer, &admin, &user], recent_blockhash);

        // Test transaction succeeded
        assert!(banks_client.process_transaction(transaction).await.is_ok());
    }
}

#[tokio::test]
async fn test_place_and_cancel_order() {
    let program_id = Pubkey::new_unique();
    let mut test = ProgramTest::new("mango", program_id, processor!(process_instruction));

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(50_000);

    let mango_group = add_mango_group_prodlike(&mut test, program_id);
    let mango_group_pk = mango_group.mango_group_pk;

    let user = Keypair::new();
    test.add_account(user.pubkey(), Account::new(u32::MAX as u64, 0, &user.pubkey()));

    // TODO: this still needs to be deposited into the mango account
    let quote_index = 0;
    let quote_index = 0;
    let quote_decimals = 6;
    let quote_unit = 10i64.pow(quote_decimals);
    let quote_lot = 10;

    let user_initial_amount = 10000 * quote_unit;
    let user_quote_account = add_token_account(
        &mut test,
        user.pubkey(),
        mango_group.tokens[quote_index].pubkey,
        user_initial_amount as u64,
    );

    let mango_account_pk = add_test_account_with_owner::<MangoAccount>(&mut test, &program_id);

    let oracle_pk = add_test_account_with_owner::<StubOracle>(&mut test, &program_id);
    let base_decimals = 6;
    let base_price = 40000;
    let base_unit = 10i64.pow(base_decimals);
    let base_lot = 100;
    let oracle_price =
        I80F48::from_num(base_price) * I80F48::from_num(quote_unit) / I80F48::from_num(base_unit);

    let perp_market_idx = 0;
    let perp_market_pk = add_test_account_with_owner::<PerpMarket>(&mut test, &program_id);

    let event_queue_pk = add_test_account_with_owner_and_extra_size::<EventQueue>(
        &mut test,
        &program_id,
        size_of::<AnyEvent>() * 32,
    );

    let bids_pk = add_test_account_with_owner::<BookSide>(&mut test, &program_id);
    let asks_pk = add_test_account_with_owner::<BookSide>(&mut test, &program_id);

    let admin = Keypair::new();

    let (mut banks_client, payer, recent_blockhash) = test.start().await;

    let init_leverage = I80F48::from_num(4);
    let maint_leverage = init_leverage * 2;
    let quantity = 1;

    // setup mango group, perp market & mango account
    {
        let mut transaction = Transaction::new_with_payer(
            &[
                mango_group.init_mango_group(&admin.pubkey()),
                init_mango_account(&program_id, &mango_group_pk, &mango_account_pk, &user.pubkey())
                    .unwrap(),
                cache_root_banks(
                    &program_id,
                    &mango_group.mango_group_pk,
                    &mango_group.mango_cache_pk,
                    &[mango_group.root_banks[quote_index].pubkey],
                )
                .unwrap(),
                deposit(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_pk,
                    &user.pubkey(),
                    &mango_group.mango_cache_pk,
                    &mango_group.root_banks[quote_index].pubkey,
                    &mango_group.root_banks[quote_index].node_banks[quote_index].pubkey,
                    &mango_group.root_banks[quote_index].node_banks[quote_index].vault,
                    &user_quote_account.pubkey,
                    user_initial_amount as u64,
                )
                .unwrap(),
                add_oracle(&program_id, &mango_group_pk, &oracle_pk, &admin.pubkey()).unwrap(),
                set_oracle(&program_id, &mango_group_pk, &oracle_pk, &admin.pubkey(), oracle_price)
                    .unwrap(),
                add_perp_market(
                    &program_id,
                    &mango_group_pk,
                    &perp_market_pk,
                    &event_queue_pk,
                    &bids_pk,
                    &asks_pk,
                    &admin.pubkey(),
                    perp_market_idx,
                    maint_leverage,
                    init_leverage,
                    base_lot,
                    quote_lot,
                )
                .unwrap(),
            ],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[&payer, &admin, &user], recent_blockhash);

        // Setup transaction succeeded
        assert!(banks_client.process_transaction(transaction).await.is_ok());
    }

    // place an order

    let bid_id = 1337;
    let ask_id = 1338;
    {
        let mut mango_group = banks_client.get_account(mango_group_pk).await.unwrap().unwrap();
        let mango_account = banks_client.get_account(mango_account_pk).await.unwrap().unwrap();
        let mango_account: &MangoAccount =
            MangoAccount::load_from_bytes(mango_account.data()).unwrap();

        let account_info: AccountInfo = (&mango_group_pk, &mut mango_group).into();
        let mango_group = MangoGroup::load_mut_checked(&account_info, &program_id).unwrap();

        let mut transaction = Transaction::new_with_payer(
            &[
                cache_prices(&program_id, &mango_group_pk, &mango_group.mango_cache, &[oracle_pk])
                    .unwrap(),
                cache_root_banks(
                    &program_id,
                    &mango_group_pk,
                    &mango_group.mango_cache,
                    &[mango_group.tokens[QUOTE_INDEX].root_bank],
                )
                .unwrap(),
                cache_perp_markets(
                    &program_id,
                    &mango_group_pk,
                    &mango_group.mango_cache,
                    &[perp_market_pk],
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_pk,
                    &user.pubkey(),
                    &mango_group.mango_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    &mango_account.spot_open_orders,
                    Side::Bid,
                    ((base_price - 1) * quote_unit * base_lot) / (base_unit * quote_lot),
                    (quantity * base_unit) / base_lot,
                    bid_id,
                    OrderType::Limit,
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_pk,
                    &user.pubkey(),
                    &mango_group.mango_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    &mango_account.spot_open_orders,
                    Side::Ask,
                    ((base_price + 1) * quote_unit * base_lot) / (base_unit * quote_lot),
                    (quantity * base_unit) / base_lot,
                    ask_id,
                    OrderType::Limit,
                )
                .unwrap(),
            ],
            Some(&payer.pubkey()),
        );
        transaction.sign(&[&payer, &user], recent_blockhash);
        assert!(banks_client.process_transaction(transaction).await.is_ok());
    }

    // cancel bid by client_id
    {
        let mut transaction = Transaction::new_with_payer(
            &[cancel_perp_order_by_client_id(
                &program_id,
                &mango_group_pk,
                &mango_account_pk,
                &user.pubkey(),
                &perp_market_pk,
                &bids_pk,
                &asks_pk,
                &event_queue_pk,
                bid_id,
            )
            .unwrap()],
            Some(&payer.pubkey()),
        );
        transaction.sign(&[&payer, &user], recent_blockhash);
        assert!(banks_client.process_transaction(transaction).await.is_ok());
    }

    // cancel ask directly
    {
        let mut mango_account = banks_client.get_account(mango_account_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&mango_account_pk, &mut mango_account).into();
        let mango_account =
            MangoAccount::load_mut_checked(&account_info, &program_id, &mango_group_pk).unwrap();

        let (client_order_id, order_id, side) =
            mango_account.perp_accounts[0].open_orders.orders_with_client_ids().last().unwrap();
        assert_eq!(u64::from(client_order_id), ask_id);
        assert_eq!(side, Side::Ask);

        let mut transaction = Transaction::new_with_payer(
            &[cancel_perp_order(
                &program_id,
                &mango_group_pk,
                &mango_account_pk,
                &user.pubkey(),
                &perp_market_pk,
                &bids_pk,
                &asks_pk,
                &event_queue_pk,
                order_id,
                side,
            )
            .unwrap()],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[&payer, &user], recent_blockhash);
        assert!(banks_client.process_transaction(transaction).await.is_ok());
    }

    // update blockhash so that instructions do not get filtered as duplicates
    let (_, recent_blockhash, _) = banks_client
        .get_fees_with_commitment_and_context(tarpc::context::current(), CommitmentLevel::Processed)
        .await
        .unwrap();

    // error when cancelling bid twice
    {
        let mut transaction = Transaction::new_with_payer(
            &[cancel_perp_order_by_client_id(
                &program_id,
                &mango_group_pk,
                &mango_account_pk,
                &user.pubkey(),
                &perp_market_pk,
                &bids_pk,
                &asks_pk,
                &event_queue_pk,
                bid_id,
            )
            .unwrap()],
            Some(&payer.pubkey()),
        );
        transaction.sign(&[&payer, &user], recent_blockhash);
        assert!(banks_client.process_transaction(transaction).await.is_err());
    }

    // error when cancelling ask twice
    {
        let mut mango_account = banks_client.get_account(mango_account_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&mango_account_pk, &mut mango_account).into();
        let mango_account =
            MangoAccount::load_mut_checked(&account_info, &program_id, &mango_group_pk).unwrap();

        let (client_order_id, order_id, side) =
            mango_account.perp_accounts[0].open_orders.orders_with_client_ids().last().unwrap();
        assert_eq!(u64::from(client_order_id), ask_id);
        assert_eq!(side, Side::Ask);

        let mut transaction = Transaction::new_with_payer(
            &[cancel_perp_order(
                &program_id,
                &mango_group_pk,
                &mango_account_pk,
                &user.pubkey(),
                &perp_market_pk,
                &bids_pk,
                &asks_pk,
                &event_queue_pk,
                order_id,
                side,
            )
            .unwrap()],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[&payer, &user], recent_blockhash);
        assert!(banks_client.process_transaction(transaction).await.is_err());
    }
}

#[tokio::test]
async fn test_place_and_match_order() {
    let program_id = Pubkey::new_unique();
    let mut test = ProgramTest::new("mango", program_id, processor!(process_instruction));

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(50_000);

    let mango_group = add_mango_group_prodlike(&mut test, program_id);
    let mango_group_pk = mango_group.mango_group_pk;

    let quote_index = 0;
    let quote_decimals = 6;
    let quote_unit = 10i64.pow(quote_decimals);
    let quote_lot = 10;

    let user_bid = Keypair::new();
    test.add_account(user_bid.pubkey(), Account::new(u32::MAX as u64, 0, &user_bid.pubkey()));

    // TODO: this still needs to be deposited into the mango account and should be connected to leverage
    let user_bid_initial_amount = 100000 * quote_unit;
    let user_bid_quote_account = add_token_account(
        &mut test,
        user_bid.pubkey(),
        mango_group.tokens[quote_index].pubkey,
        user_bid_initial_amount as u64,
    );

    let mango_account_bid_pk = add_test_account_with_owner::<MangoAccount>(&mut test, &program_id);

    let user_ask = Keypair::new();
    test.add_account(user_ask.pubkey(), Account::new(u32::MAX as u64, 0, &user_ask.pubkey()));

    // TODO: this still needs to be deposited into the mango account and should be connected to leverage
    let user_ask_initial_amount = 100000 * quote_unit;
    let user_ask_quote_account = add_token_account(
        &mut test,
        user_ask.pubkey(),
        mango_group.tokens[quote_index].pubkey,
        user_ask_initial_amount as u64,
    );

    let mango_account_ask_pk = add_test_account_with_owner::<MangoAccount>(&mut test, &program_id);

    let oracle_pk = add_test_account_with_owner::<StubOracle>(&mut test, &program_id);
    let base_decimals = 6;
    let base_price = 40000;
    let base_unit = 10i64.pow(base_decimals);
    let base_lot = 100;
    let oracle_price =
        I80F48::from_num(base_price) * I80F48::from_num(quote_unit) / I80F48::from_num(base_unit);

    let perp_market_idx = 0;
    let perp_market_pk = add_test_account_with_owner::<PerpMarket>(&mut test, &program_id);

    let event_queue_pk = add_test_account_with_owner_and_extra_size::<EventQueue>(
        &mut test,
        &program_id,
        size_of::<AnyEvent>() * 32,
    );

    let bids_pk = add_test_account_with_owner::<BookSide>(&mut test, &program_id);
    let asks_pk = add_test_account_with_owner::<BookSide>(&mut test, &program_id);

    let admin = Keypair::new();

    let mut test_context = test.start_with_context().await;
    let init_leverage = I80F48::from_num(10);
    let maint_leverage = init_leverage * 2;
    let quantity = 1;

    // setup mango group, perp market & mango account
    {
        let mut transaction = Transaction::new_with_payer(
            &[
                mango_group.init_mango_group(&admin.pubkey()),
                init_mango_account(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_bid_pk,
                    &user_bid.pubkey(),
                )
                .unwrap(),
                init_mango_account(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_ask_pk,
                    &user_ask.pubkey(),
                )
                .unwrap(),
                cache_root_banks(
                    &program_id,
                    &mango_group.mango_group_pk,
                    &mango_group.mango_cache_pk,
                    &[mango_group.root_banks[0].pubkey],
                )
                .unwrap(),
                deposit(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_bid_pk,
                    &user_bid.pubkey(),
                    &mango_group.mango_cache_pk,
                    &mango_group.root_banks[quote_index].pubkey,
                    &mango_group.root_banks[quote_index].node_banks[quote_index].pubkey,
                    &mango_group.root_banks[quote_index].node_banks[quote_index].vault,
                    &user_bid_quote_account.pubkey,
                    user_bid_initial_amount as u64,
                )
                .unwrap(),
                deposit(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_ask_pk,
                    &user_ask.pubkey(),
                    &mango_group.mango_cache_pk,
                    &mango_group.root_banks[quote_index].pubkey,
                    &mango_group.root_banks[quote_index].node_banks[quote_index].pubkey,
                    &mango_group.root_banks[quote_index].node_banks[quote_index].vault,
                    &user_ask_quote_account.pubkey,
                    user_ask_initial_amount as u64,
                )
                .unwrap(),
                add_oracle(&program_id, &mango_group_pk, &oracle_pk, &admin.pubkey()).unwrap(),
                set_oracle(&program_id, &mango_group_pk, &oracle_pk, &admin.pubkey(), oracle_price)
                    .unwrap(),
                add_perp_market(
                    &program_id,
                    &mango_group_pk,
                    &perp_market_pk,
                    &event_queue_pk,
                    &bids_pk,
                    &asks_pk,
                    &admin.pubkey(),
                    perp_market_idx,
                    maint_leverage,
                    init_leverage,
                    base_lot,
                    quote_lot,
                )
                .unwrap(),
            ],
            Some(&test_context.payer.pubkey()),
        );

        transaction.sign(
            &[&test_context.payer, &admin, &user_bid, &user_ask],
            test_context.last_blockhash,
        );

        // Setup transaction succeeded
        assert!(test_context.banks_client.process_transaction(transaction).await.is_ok());
    }

    // place an order

    let bid_id = 1337;
    let ask_id = 1338;
    let min_bid_id = 1339;
    {
        let mut mango_group =
            test_context.banks_client.get_account(mango_group_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&mango_group_pk, &mut mango_group).into();
        let mango_group = MangoGroup::load_mut_checked(&account_info, &program_id).unwrap();

        let user_ask_ma_account =
            test_context.banks_client.get_account(mango_account_ask_pk).await.unwrap().unwrap();
        let user_ask_ma: &MangoAccount =
            MangoAccount::load_from_bytes(user_ask_ma_account.data()).unwrap();

        let user_bid_ma_account =
            test_context.banks_client.get_account(mango_account_bid_pk).await.unwrap().unwrap();
        let user_bid_ma: &MangoAccount =
            MangoAccount::load_from_bytes(user_bid_ma_account.data()).unwrap();

        let mut transaction = Transaction::new_with_payer(
            &[
                cache_prices(&program_id, &mango_group_pk, &mango_group.mango_cache, &[oracle_pk])
                    .unwrap(),
                cache_root_banks(
                    &program_id,
                    &mango_group_pk,
                    &mango_group.mango_cache,
                    &[mango_group.tokens[QUOTE_INDEX].root_bank],
                )
                .unwrap(),
                cache_perp_markets(
                    &program_id,
                    &mango_group_pk,
                    &mango_group.mango_cache,
                    &[perp_market_pk],
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_bid_pk,
                    &user_bid.pubkey(),
                    &mango_group.mango_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    &user_bid_ma.spot_open_orders,
                    Side::Bid,
                    ((base_price + 1) * quote_unit * base_lot) / (base_unit * quote_lot),
                    (quantity * base_unit) / base_lot,
                    bid_id,
                    OrderType::Limit,
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_ask_pk,
                    &user_ask.pubkey(),
                    &mango_group.mango_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    &user_ask_ma.spot_open_orders,
                    Side::Ask,
                    ((base_price - 1) * quote_unit * base_lot) / (base_unit * quote_lot),
                    (quantity * base_unit) / base_lot,
                    ask_id,
                    OrderType::Limit,
                )
                .unwrap(),
                // place an absolue low-ball bid, just to make sure this
                place_perp_order(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_bid_pk,
                    &user_bid.pubkey(),
                    &mango_group.mango_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    &user_bid_ma.spot_open_orders,
                    Side::Bid,
                    1,
                    1,
                    min_bid_id,
                    OrderType::Limit,
                )
                .unwrap(),
                consume_events(
                    &program_id,
                    &mango_group_pk,
                    &perp_market_pk,
                    &event_queue_pk,
                    &mut [mango_account_bid_pk, mango_account_ask_pk],
                    3,
                )
                .unwrap(),
            ],
            Some(&test_context.payer.pubkey()),
        );
        transaction.sign(&[&test_context.payer, &user_bid, &user_ask], test_context.last_blockhash);
        assert!(test_context.banks_client.process_transaction(transaction).await.is_ok());
    }

    let bid_base_position = quantity * base_unit / base_lot;
    let bid_quote_position = -40001 * (quantity * quote_unit);
    {
        let mut mango_account =
            test_context.banks_client.get_account(mango_account_bid_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&mango_account_bid_pk, &mut mango_account).into();
        let mango_account =
            MangoAccount::load_mut_checked(&account_info, &program_id, &mango_group_pk).unwrap();

        let base_position = mango_account.perp_accounts[perp_market_idx].base_position;
        let quote_position = mango_account.perp_accounts[perp_market_idx].quote_position;

        // TODO: verify fees
        assert_eq!(base_position, bid_base_position);
        assert_eq!(quote_position, bid_quote_position);
    }
    println!("u1 base={} quoute={}", bid_base_position, bid_quote_position);

    let ask_base_position = -1 * quantity * base_unit / base_lot;
    let ask_quote_position = (40001 * quantity * quote_unit);
    {
        let mut mango_account =
            test_context.banks_client.get_account(mango_account_ask_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&mango_account_ask_pk, &mut mango_account).into();
        let mango_account =
            MangoAccount::load_mut_checked(&account_info, &program_id, &mango_group_pk).unwrap();

        let base_position = mango_account.perp_accounts[perp_market_idx].base_position;
        let quote_position = mango_account.perp_accounts[perp_market_idx].quote_position;

        // TODO: add fees
        assert_eq!(base_position, ask_base_position);
        assert_eq!(quote_position, ask_quote_position);
    }
    println!("u2 base={} quoute={}", ask_base_position, ask_quote_position);

    {
        let mut mango_group =
            test_context.banks_client.get_account(mango_group_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&mango_group_pk, &mut mango_group).into();
        let mango_group = MangoGroup::load_mut_checked(&account_info, &program_id).unwrap();

        let mut perp_market =
            test_context.banks_client.get_account(perp_market_pk).await.unwrap().unwrap();
        let account_info = (&perp_market_pk, &mut perp_market).into();
        let perp_market =
            PerpMarket::load_mut_checked(&account_info, &program_id, &mango_group_pk).unwrap();

        let mut event_queue = test_context
            .banks_client
            .get_account_with_commitment(event_queue_pk, CommitmentLevel::Processed)
            .await
            .unwrap()
            .unwrap();
        let account_info: AccountInfo = (&event_queue_pk, &mut event_queue).into();
        let event_queue =
            EventQueue::load_mut_checked(&account_info, &program_id, &perp_market).unwrap();

        assert!(event_queue.empty());
        assert_eq!(event_queue.header.head(), 3);

        let [e1, e2, e3] = array_ref![event_queue.buf, 0, 3];
        assert_eq!(e1.event_type, EventType::Fill as u8);
        assert_eq!(e2.event_type, EventType::Fill as u8);
        assert_eq!(e3.event_type, EventType::Out as u8);

        let e1: &FillEvent = cast_ref(e1);
        let e2: &FillEvent = cast_ref(e2);
        let _e3: &OutEvent = cast_ref(e3);

        assert!(e1.maker);
        assert_eq!(e1.base_change, bid_base_position);
        assert_eq!(e1.quote_change, bid_quote_position / quote_lot);

        assert!(!e2.maker);
        assert_eq!(e2.base_change, ask_base_position);
        assert_eq!(e2.quote_change, ask_quote_position / quote_lot);

        // TODO: verify out-event
    }

    // move to slot 10
    test_context.warp_to_slot(10).unwrap();

    {
        let mut mango_group =
            test_context.banks_client.get_account(mango_group_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&mango_group_pk, &mut mango_group).into();
        let mango_group = MangoGroup::load_mut_checked(&account_info, &program_id).unwrap();
        let user_ask_ma_account =
            test_context.banks_client.get_account(mango_account_ask_pk).await.unwrap().unwrap();
        let user_ask_ma: &MangoAccount =
            MangoAccount::load_from_bytes(user_ask_ma_account.data()).unwrap();

        let user_bid_ma_account =
            test_context.banks_client.get_account(mango_account_bid_pk).await.unwrap().unwrap();
        let user_bid_ma: &MangoAccount =
            MangoAccount::load_from_bytes(user_bid_ma_account.data()).unwrap();

        let mut transaction = Transaction::new_with_payer(
            &[
                cache_prices(&program_id, &mango_group_pk, &mango_group.mango_cache, &[oracle_pk])
                    .unwrap(),
                update_funding(
                    &program_id,
                    &mango_group_pk,
                    &mango_group.mango_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                )
                .unwrap(),
                cache_root_banks(
                    &program_id,
                    &mango_group_pk,
                    &mango_group.mango_cache,
                    &[mango_group.tokens[QUOTE_INDEX].root_bank],
                )
                .unwrap(),
                cache_perp_markets(
                    &program_id,
                    &mango_group_pk,
                    &mango_group.mango_cache,
                    &[perp_market_pk],
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_ask_pk,
                    &user_ask.pubkey(),
                    &mango_group.mango_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    &user_ask_ma.spot_open_orders,
                    Side::Bid,
                    ((base_price + 1) * quote_unit * base_lot) / (base_unit * quote_lot),
                    (quantity * base_unit) / base_lot,
                    ask_id,
                    OrderType::Limit,
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_bid_pk,
                    &user_bid.pubkey(),
                    &mango_group.mango_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    &user_bid_ma.spot_open_orders,
                    Side::Ask,
                    ((base_price - 1) * quote_unit * base_lot) / (base_unit * quote_lot),
                    (quantity * base_unit) / base_lot,
                    bid_id,
                    OrderType::Limit,
                )
                .unwrap(),
                consume_events(
                    &program_id,
                    &mango_group_pk,
                    &perp_market_pk,
                    &event_queue_pk,
                    &mut [mango_account_bid_pk, mango_account_ask_pk],
                    3,
                )
                .unwrap(),
            ],
            Some(&test_context.payer.pubkey()),
        );
        transaction.sign(&[&test_context.payer, &user_bid, &user_ask], test_context.last_blockhash);
        assert!(test_context.banks_client.process_transaction(transaction).await.is_ok());
    }

    {
        let mut mango_account =
            test_context.banks_client.get_account(mango_account_bid_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&mango_account_bid_pk, &mut mango_account).into();
        let mango_account =
            MangoAccount::load_mut_checked(&account_info, &program_id, &mango_group_pk).unwrap();

        let base_position = mango_account.perp_accounts[perp_market_idx].base_position;
        let quote_position = mango_account.perp_accounts[perp_market_idx].quote_position;

        // TODO: add fees & funding
        assert_eq!(base_position, 0);
        assert_eq!(quote_position, 0);
        println!("u1: base={} quote={}", base_position, quote_position)
    }

    {
        let mut mango_account =
            test_context.banks_client.get_account(mango_account_ask_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&mango_account_ask_pk, &mut mango_account).into();
        let mango_account =
            MangoAccount::load_mut_checked(&account_info, &program_id, &mango_group_pk).unwrap();

        let base_position = mango_account.perp_accounts[perp_market_idx].base_position;
        let quote_position = mango_account.perp_accounts[perp_market_idx].quote_position;

        // TODO: add fees & funding
        assert_eq!(base_position, 0);
        assert_eq!(quote_position, 0);
        println!("u2: base={} quote={}", base_position, quote_position)
    }
}

#[tokio::test]
async fn test_place_and_match_multiple_orders() {
    let program_id = Pubkey::new_unique();
    let mut test = ProgramTest::new("mango", program_id, processor!(process_instruction));

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(50_000);

    let mango_group = add_mango_group_prodlike(&mut test, program_id);
    let mango_group_pk = mango_group.mango_group_pk;

    let quote_index = 0;
    let quote_decimals = 6;
    let quote_unit = 10i64.pow(quote_decimals);
    let quote_lot = 10;

    let user_bid = Keypair::new();
    test.add_account(user_bid.pubkey(), Account::new(u32::MAX as u64, 0, &user_bid.pubkey()));

    // TODO: this still needs to be deposited into the mango account and should be connected to leverage
    let user_bid_initial_amount = 100000 * quote_unit;
    let user_bid_quote_account = add_token_account(
        &mut test,
        user_bid.pubkey(),
        mango_group.tokens[quote_index].pubkey,
        user_bid_initial_amount as u64,
    );

    let mango_account_bid_pk = add_test_account_with_owner::<MangoAccount>(&mut test, &program_id);

    let user_ask = Keypair::new();
    test.add_account(user_ask.pubkey(), Account::new(u32::MAX as u64, 0, &user_ask.pubkey()));

    // TODO: this still needs to be deposited into the mango account and should be connected to leverage
    let user_ask_initial_amount = 100000 * quote_unit;
    let user_ask_quote_account = add_token_account(
        &mut test,
        user_ask.pubkey(),
        mango_group.tokens[quote_index].pubkey,
        user_ask_initial_amount as u64,
    );

    let mango_account_ask_pk = add_test_account_with_owner::<MangoAccount>(&mut test, &program_id);

    let oracle_pk = add_test_account_with_owner::<StubOracle>(&mut test, &program_id);
    let base_decimals = 6;
    let base_price = 40000;
    let base_unit = 10i64.pow(base_decimals);
    let base_lot = 100;
    let oracle_price =
        I80F48::from_num(base_price) * I80F48::from_num(quote_unit) / I80F48::from_num(base_unit);

    let perp_market_idx = 0;
    let perp_market_pk = add_test_account_with_owner::<PerpMarket>(&mut test, &program_id);

    let event_queue_pk = add_test_account_with_owner_and_extra_size::<EventQueue>(
        &mut test,
        &program_id,
        size_of::<AnyEvent>() * 32,
    );

    let bids_pk = add_test_account_with_owner::<BookSide>(&mut test, &program_id);
    let asks_pk = add_test_account_with_owner::<BookSide>(&mut test, &program_id);

    let admin = Keypair::new();

    let mut test_context = test.start_with_context().await;

    let init_leverage = I80F48::from_num(10);
    let maint_leverage = init_leverage * 2;
    let quantity = 1;

    // setup mango group, perp market & mango account
    {
        let mut transaction = Transaction::new_with_payer(
            &[
                mango_group.init_mango_group(&admin.pubkey()),
                init_mango_account(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_bid_pk,
                    &user_bid.pubkey(),
                )
                .unwrap(),
                init_mango_account(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_ask_pk,
                    &user_ask.pubkey(),
                )
                .unwrap(),
                cache_root_banks(
                    &program_id,
                    &mango_group_pk,
                    &mango_group.mango_cache_pk,
                    &[mango_group.root_banks[quote_index].pubkey],
                )
                .unwrap(),
                deposit(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_bid_pk,
                    &user_bid.pubkey(),
                    &mango_group.mango_cache_pk,
                    &mango_group.root_banks[quote_index].pubkey,
                    &mango_group.root_banks[quote_index].node_banks[quote_index].pubkey,
                    &mango_group.root_banks[quote_index].node_banks[quote_index].vault,
                    &user_bid_quote_account.pubkey,
                    user_bid_initial_amount as u64,
                )
                .unwrap(),
                deposit(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_ask_pk,
                    &user_ask.pubkey(),
                    &mango_group.mango_cache_pk,
                    &mango_group.root_banks[quote_index].pubkey,
                    &mango_group.root_banks[quote_index].node_banks[quote_index].pubkey,
                    &mango_group.root_banks[quote_index].node_banks[quote_index].vault,
                    &user_ask_quote_account.pubkey,
                    user_ask_initial_amount as u64,
                )
                .unwrap(),
                add_oracle(&program_id, &mango_group_pk, &oracle_pk, &admin.pubkey()).unwrap(),
                set_oracle(&program_id, &mango_group_pk, &oracle_pk, &admin.pubkey(), oracle_price)
                    .unwrap(),
                add_perp_market(
                    &program_id,
                    &mango_group_pk,
                    &perp_market_pk,
                    &event_queue_pk,
                    &bids_pk,
                    &asks_pk,
                    &admin.pubkey(),
                    perp_market_idx,
                    maint_leverage,
                    init_leverage,
                    base_lot,
                    quote_lot,
                )
                .unwrap(),
            ],
            Some(&test_context.payer.pubkey()),
        );

        transaction.sign(
            &[&test_context.payer, &admin, &user_bid, &user_ask],
            test_context.last_blockhash,
        );

        // Setup transaction succeeded
        assert!(test_context.banks_client.process_transaction(transaction).await.is_ok());
    }

    // place an order

    let bid_id = 1337;
    let ask_id = 1438;
    let min_bid_id = 1539;
    {
        let mut mango_group =
            test_context.banks_client.get_account(mango_group_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&mango_group_pk, &mut mango_group).into();
        let mango_group = MangoGroup::load_mut_checked(&account_info, &program_id).unwrap();
        let user_ask_ma_account =
            test_context.banks_client.get_account(mango_account_ask_pk).await.unwrap().unwrap();
        let user_ask_ma: &MangoAccount =
            MangoAccount::load_from_bytes(user_ask_ma_account.data()).unwrap();

        let user_bid_ma_account =
            test_context.banks_client.get_account(mango_account_bid_pk).await.unwrap().unwrap();
        let user_bid_ma: &MangoAccount =
            MangoAccount::load_from_bytes(user_bid_ma_account.data()).unwrap();

        let mut transaction = Transaction::new_with_payer(
            &[
                cache_prices(&program_id, &mango_group_pk, &mango_group.mango_cache, &[oracle_pk])
                    .unwrap(),
                cache_root_banks(
                    &program_id,
                    &mango_group_pk,
                    &mango_group.mango_cache,
                    &[mango_group.tokens[QUOTE_INDEX].root_bank],
                )
                .unwrap(),
                cache_perp_markets(
                    &program_id,
                    &mango_group_pk,
                    &mango_group.mango_cache,
                    &[perp_market_pk],
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_bid_pk,
                    &user_bid.pubkey(),
                    &mango_group.mango_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    &user_bid_ma.spot_open_orders,
                    Side::Bid,
                    ((base_price + 1) * quote_unit * base_lot) / (base_unit * quote_lot),
                    (quantity * base_unit) / base_lot,
                    bid_id,
                    OrderType::Limit,
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_bid_pk,
                    &user_bid.pubkey(),
                    &mango_group.mango_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    &user_bid_ma.spot_open_orders,
                    Side::Bid,
                    ((base_price - 10) * quote_unit * base_lot) / (base_unit * quote_lot),
                    (quantity * base_unit) / base_lot,
                    bid_id,
                    OrderType::Limit,
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_ask_pk,
                    &user_ask.pubkey(),
                    &mango_group.mango_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    &user_ask_ma.spot_open_orders,
                    Side::Ask,
                    ((base_price - 1) * quote_unit * base_lot) / (base_unit * quote_lot),
                    (quantity * base_unit) / base_lot,
                    ask_id,
                    OrderType::Limit,
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_ask_pk,
                    &user_ask.pubkey(),
                    &mango_group.mango_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    &user_ask_ma.spot_open_orders,
                    Side::Ask,
                    ((base_price + 10) * quote_unit * base_lot) / (base_unit * quote_lot),
                    (quantity * base_unit) / base_lot,
                    ask_id,
                    OrderType::Limit,
                )
                .unwrap(),
                // place an absolue low-ball bid, just to make sure this
                place_perp_order(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_bid_pk,
                    &user_bid.pubkey(),
                    &mango_group.mango_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    &user_bid_ma.spot_open_orders,
                    Side::Bid,
                    1,
                    1,
                    min_bid_id,
                    OrderType::Limit,
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_ask_pk,
                    &user_ask.pubkey(),
                    &mango_group.mango_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    &user_ask_ma.spot_open_orders,
                    Side::Bid,
                    ((base_price - 1) * quote_unit * base_lot) / (base_unit * quote_lot),
                    (quantity * base_unit) / base_lot,
                    ask_id,
                    OrderType::Limit,
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &mango_group_pk,
                    &mango_account_bid_pk,
                    &user_bid.pubkey(),
                    &mango_group.mango_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    &user_bid_ma.spot_open_orders,
                    Side::Ask,
                    ((base_price - 10) * quote_unit * base_lot) / (base_unit * quote_lot),
                    (quantity * base_unit) / base_lot,
                    bid_id,
                    OrderType::Limit,
                )
                .unwrap(),
                consume_events(
                    &program_id,
                    &mango_group_pk,
                    &perp_market_pk,
                    &event_queue_pk,
                    &mut [mango_account_bid_pk, mango_account_ask_pk],
                    3,
                )
                .unwrap(),
            ],
            Some(&test_context.payer.pubkey()),
        );
        transaction.sign(&[&test_context.payer, &user_bid, &user_ask], test_context.last_blockhash);
        assert!(test_context.banks_client.process_transaction(transaction).await.is_ok());
    }
}
