// Tests related to placing orders on a perp market
mod helpers;
use arrayref::array_ref;
use bytemuck::cast_ref;
use fixed::types::I80F48;
use helpers::*;
use std::{mem::size_of, thread::sleep, time::Duration};

use merps::{
    entrypoint::process_instruction, instruction::*, matching::*, oracle::StubOracle, queue::*,
    state::*,
};
use solana_program::{account_info::AccountInfo, pubkey::Pubkey};
use solana_program_test::*;
use solana_sdk::{
    account::Account, commitment_config::CommitmentLevel, signature::Keypair, signer::Signer,
    transaction::Transaction,
};

#[tokio::test]
async fn test_init_perp_market() {
    let program_id = Pubkey::new_unique();
    let mut test = ProgramTest::new("merps", program_id, processor!(process_instruction));

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(50_000);

    let merps_group = add_merps_group_prodlike(&mut test, program_id);
    let merps_group_pk = merps_group.merps_group_pk;

    let user = Keypair::new();
    test.add_account(user.pubkey(), Account::new(u32::MAX as u64, 0, &user.pubkey()));

    let quote_index = 0;
    let quote_decimals = 6;
    let quote_unit = 10i64.pow(quote_decimals);
    let quote_lot = 100;

    let user_initial_amount = 200 * quote_unit;
    let user_quote_account = add_token_account(
        &mut test,
        user.pubkey(),
        merps_group.tokens[quote_index].pubkey,
        user_initial_amount as u64,
    );

    let merps_account_pk = add_test_account_with_owner::<MerpsAccount>(&mut test, &program_id);

    let oracle_pk = add_test_account_with_owner::<StubOracle>(&mut test, &program_id);
    let tsla_decimals = 4;
    let tsla_price = 420;
    let tsla_unit = 10i64.pow(tsla_decimals);
    let tsla_lot = 10;
    let oracle_price =
        I80F48::from_num(tsla_price) * I80F48::from_num(quote_unit) / I80F48::from_num(tsla_unit);

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
    // setup merps group, perp market & merps account
    {
        let mut transaction = Transaction::new_with_payer(
            &[
                merps_group.init_merps_group(&admin.pubkey()),
                init_merps_account(&program_id, &merps_group_pk, &merps_account_pk, &user.pubkey())
                    .unwrap(),
                add_oracle(&program_id, &merps_group_pk, &oracle_pk, &admin.pubkey()).unwrap(),
                set_oracle(&program_id, &merps_group_pk, &oracle_pk, &admin.pubkey(), oracle_price)
                    .unwrap(),
                add_perp_market(
                    &program_id,
                    &merps_group_pk,
                    &perp_market_pk,
                    &event_queue_pk,
                    &bids_pk,
                    &asks_pk,
                    &admin.pubkey(),
                    perp_market_idx,
                    maint_leverage,
                    init_leverage,
                    100,
                    10,
                )
                .unwrap(),
                add_to_basket(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_pk,
                    &user.pubkey(),
                    perp_market_idx,
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
    let mut test = ProgramTest::new("merps", program_id, processor!(process_instruction));

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(50_000);

    let merps_group = add_merps_group_prodlike(&mut test, program_id);
    let merps_group_pk = merps_group.merps_group_pk;

    let user = Keypair::new();
    test.add_account(user.pubkey(), Account::new(u32::MAX as u64, 0, &user.pubkey()));

    // TODO: this still needs to be deposited into the merps account
    let quote_index = 0;
    let quote_index = 0;
    let quote_decimals = 6;
    let quote_unit = 10i64.pow(quote_decimals);
    let quote_lot = 100;

    let user_initial_amount = 1000 * quote_unit;
    let user_quote_account = add_token_account(
        &mut test,
        user.pubkey(),
        merps_group.tokens[quote_index].pubkey,
        user_initial_amount as u64,
    );

    let merps_account_pk = add_test_account_with_owner::<MerpsAccount>(&mut test, &program_id);

    let oracle_pk = add_test_account_with_owner::<StubOracle>(&mut test, &program_id);
    let tsla_decimals = 4;
    let tsla_price = 420;
    let tsla_unit = 10i64.pow(tsla_decimals);
    let tsla_lot = 10;
    let oracle_price =
        I80F48::from_num(tsla_price) * I80F48::from_num(quote_unit) / I80F48::from_num(tsla_unit);

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
    let quantity = 1;

    // setup merps group, perp market & merps account
    {
        let mut transaction = Transaction::new_with_payer(
            &[
                merps_group.init_merps_group(&admin.pubkey()),
                init_merps_account(&program_id, &merps_group_pk, &merps_account_pk, &user.pubkey())
                    .unwrap(),
                add_oracle(&program_id, &merps_group_pk, &oracle_pk, &admin.pubkey()).unwrap(),
                set_oracle(&program_id, &merps_group_pk, &oracle_pk, &admin.pubkey(), oracle_price)
                    .unwrap(),
                add_perp_market(
                    &program_id,
                    &merps_group_pk,
                    &perp_market_pk,
                    &event_queue_pk,
                    &bids_pk,
                    &asks_pk,
                    &admin.pubkey(),
                    perp_market_idx,
                    maint_leverage,
                    init_leverage,
                    tsla_lot,
                    quote_lot,
                )
                .unwrap(),
                add_to_basket(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_pk,
                    &user.pubkey(),
                    perp_market_idx,
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
        let mut merps_group = banks_client.get_account(merps_group_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&merps_group_pk, &mut merps_group).into();
        let merps_group = MerpsGroup::load_mut_checked(&account_info, &program_id).unwrap();

        let mut transaction = Transaction::new_with_payer(
            &[
                cache_prices(&program_id, &merps_group_pk, &merps_group.merps_cache, &[oracle_pk])
                    .unwrap(),
                cache_root_banks(
                    &program_id,
                    &merps_group_pk,
                    &merps_group.merps_cache,
                    &[merps_group.tokens[QUOTE_INDEX].root_bank],
                )
                .unwrap(),
                cache_perp_markets(
                    &program_id,
                    &merps_group_pk,
                    &merps_group.merps_cache,
                    &[perp_market_pk],
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_pk,
                    &user.pubkey(),
                    &merps_group.merps_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    Side::Bid,
                    ((tsla_price - 1) * quote_unit * tsla_lot) / (tsla_unit * quote_lot),
                    (quantity * tsla_unit) / tsla_lot,
                    bid_id,
                    OrderType::Limit,
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_pk,
                    &user.pubkey(),
                    &merps_group.merps_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    Side::Ask,
                    ((tsla_price + 1) * quote_unit * tsla_lot) / (tsla_unit * quote_lot),
                    (quantity * tsla_unit) / tsla_lot,
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
                &merps_group_pk,
                &merps_account_pk,
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
        let mut merps_account = banks_client.get_account(merps_account_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&merps_account_pk, &mut merps_account).into();
        let merps_account =
            MerpsAccount::load_mut_checked(&account_info, &program_id, &merps_group_pk).unwrap();

        let (client_order_id, order_id, side) =
            merps_account.perp_accounts[0].open_orders.orders_with_client_ids().last().unwrap();
        assert_eq!(u64::from(client_order_id), ask_id);
        assert_eq!(side, Side::Ask);

        let mut transaction = Transaction::new_with_payer(
            &[cancel_perp_order(
                &program_id,
                &merps_group_pk,
                &merps_account_pk,
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
                &merps_group_pk,
                &merps_account_pk,
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
        let mut merps_account = banks_client.get_account(merps_account_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&merps_account_pk, &mut merps_account).into();
        let merps_account =
            MerpsAccount::load_mut_checked(&account_info, &program_id, &merps_group_pk).unwrap();

        let (client_order_id, order_id, side) =
            merps_account.perp_accounts[0].open_orders.orders_with_client_ids().last().unwrap();
        assert_eq!(u64::from(client_order_id), ask_id);
        assert_eq!(side, Side::Ask);

        let mut transaction = Transaction::new_with_payer(
            &[cancel_perp_order(
                &program_id,
                &merps_group_pk,
                &merps_account_pk,
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
    let mut test = ProgramTest::new("merps", program_id, processor!(process_instruction));

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(50_000);

    let merps_group = add_merps_group_prodlike(&mut test, program_id);
    let merps_group_pk = merps_group.merps_group_pk;

    let quote_index = 0;
    let quote_decimals = 6;
    let quote_unit = 10i64.pow(quote_decimals);
    let quote_lot = 100;

    let user_bid = Keypair::new();
    test.add_account(user_bid.pubkey(), Account::new(u32::MAX as u64, 0, &user_bid.pubkey()));

    // TODO: this still needs to be deposited into the merps account and should be connected to leverage
    let user_bid_initial_amount = 1 * quote_unit;
    let user_bid_quote_account = add_token_account(
        &mut test,
        user_bid.pubkey(),
        merps_group.tokens[quote_index].pubkey,
        user_bid_initial_amount as u64,
    );

    let merps_account_bid_pk = add_test_account_with_owner::<MerpsAccount>(&mut test, &program_id);

    let user_ask = Keypair::new();
    test.add_account(user_ask.pubkey(), Account::new(u32::MAX as u64, 0, &user_ask.pubkey()));

    // TODO: this still needs to be deposited into the merps account and should be connected to leverage
    let user_ask_initial_amount = 1 * quote_unit;
    let user_ask_quote_account = add_token_account(
        &mut test,
        user_ask.pubkey(),
        merps_group.tokens[quote_index].pubkey,
        user_ask_initial_amount as u64,
    );

    let merps_account_ask_pk = add_test_account_with_owner::<MerpsAccount>(&mut test, &program_id);

    let oracle_pk = add_test_account_with_owner::<StubOracle>(&mut test, &program_id);
    let tsla_decimals = 4;
    let tsla_price = 420;
    let tsla_unit = 10i64.pow(tsla_decimals);
    let tsla_lot = 10;
    let oracle_price =
        I80F48::from_num(tsla_price) * I80F48::from_num(quote_unit) / I80F48::from_num(tsla_unit);

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
    let quantity = 1;

    // setup merps group, perp market & merps account
    {
        let mut transaction = Transaction::new_with_payer(
            &[
                merps_group.init_merps_group(&admin.pubkey()),
                init_merps_account(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_bid_pk,
                    &user_bid.pubkey(),
                )
                .unwrap(),
                init_merps_account(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_ask_pk,
                    &user_ask.pubkey(),
                )
                .unwrap(),
                add_oracle(&program_id, &merps_group_pk, &oracle_pk, &admin.pubkey()).unwrap(),
                set_oracle(&program_id, &merps_group_pk, &oracle_pk, &admin.pubkey(), oracle_price)
                    .unwrap(),
                add_perp_market(
                    &program_id,
                    &merps_group_pk,
                    &perp_market_pk,
                    &event_queue_pk,
                    &bids_pk,
                    &asks_pk,
                    &admin.pubkey(),
                    perp_market_idx,
                    maint_leverage,
                    init_leverage,
                    tsla_lot,
                    quote_lot,
                )
                .unwrap(),
                add_to_basket(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_bid_pk,
                    &user_bid.pubkey(),
                    perp_market_idx,
                )
                .unwrap(),
                add_to_basket(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_ask_pk,
                    &user_ask.pubkey(),
                    perp_market_idx,
                )
                .unwrap(),
            ],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[&payer, &admin, &user_bid, &user_ask], recent_blockhash);

        // Setup transaction succeeded
        assert!(banks_client.process_transaction(transaction).await.is_ok());
    }

    // place an order

    let bid_id = 1337;
    let ask_id = 1338;
    let min_bid_id = 1339;
    {
        let mut merps_group = banks_client.get_account(merps_group_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&merps_group_pk, &mut merps_group).into();
        let merps_group = MerpsGroup::load_mut_checked(&account_info, &program_id).unwrap();

        let mut transaction = Transaction::new_with_payer(
            &[
                cache_prices(&program_id, &merps_group_pk, &merps_group.merps_cache, &[oracle_pk])
                    .unwrap(),
                cache_root_banks(
                    &program_id,
                    &merps_group_pk,
                    &merps_group.merps_cache,
                    &[merps_group.tokens[QUOTE_INDEX].root_bank],
                )
                .unwrap(),
                cache_perp_markets(
                    &program_id,
                    &merps_group_pk,
                    &merps_group.merps_cache,
                    &[perp_market_pk],
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_bid_pk,
                    &user_bid.pubkey(),
                    &merps_group.merps_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    Side::Bid,
                    ((tsla_price + 1) * quote_unit * tsla_lot) / (tsla_unit * quote_lot),
                    (quantity * tsla_unit) / tsla_lot,
                    bid_id,
                    OrderType::Limit,
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_ask_pk,
                    &user_ask.pubkey(),
                    &merps_group.merps_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    Side::Ask,
                    ((tsla_price - 1) * quote_unit * tsla_lot) / (tsla_unit * quote_lot),
                    (quantity * tsla_unit) / tsla_lot,
                    ask_id,
                    OrderType::Limit,
                )
                .unwrap(),
                // place an absolue low-ball bid, just to make sure this
                place_perp_order(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_bid_pk,
                    &user_bid.pubkey(),
                    &merps_group.merps_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    Side::Bid,
                    1,
                    1,
                    min_bid_id,
                    OrderType::Limit,
                )
                .unwrap(),
                consume_events(
                    &program_id,
                    &merps_group_pk,
                    &perp_market_pk,
                    &event_queue_pk,
                    &[merps_account_bid_pk, merps_account_ask_pk],
                    3,
                )
                .unwrap(),
            ],
            Some(&payer.pubkey()),
        );
        transaction.sign(&[&payer, &user_bid, &user_ask], recent_blockhash);
        assert!(banks_client.process_transaction(transaction).await.is_ok());
    }

    let bid_base_position = quantity * tsla_unit / tsla_lot;
    let bid_quote_position = -101 * (quantity * quote_unit);
    {
        let mut merps_account =
            banks_client.get_account(merps_account_bid_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&merps_account_bid_pk, &mut merps_account).into();
        let merps_account =
            MerpsAccount::load_mut_checked(&account_info, &program_id, &merps_group_pk).unwrap();

        let base_position = merps_account.perp_accounts[perp_market_idx].base_position;
        let quote_position = merps_account.perp_accounts[perp_market_idx].quote_position;

        // TODO: verify fees
        assert_eq!(base_position, bid_base_position);
        assert_eq!(quote_position, bid_quote_position);
    }

    let ask_base_position = -1 * quantity * tsla_unit / tsla_lot;
    let ask_quote_position = (101 * quantity * quote_unit);
    {
        let mut merps_account =
            banks_client.get_account(merps_account_ask_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&merps_account_ask_pk, &mut merps_account).into();
        let merps_account =
            MerpsAccount::load_mut_checked(&account_info, &program_id, &merps_group_pk).unwrap();

        let base_position = merps_account.perp_accounts[perp_market_idx].base_position;
        let quote_position = merps_account.perp_accounts[perp_market_idx].quote_position;

        // TODO: add fees
        assert_eq!(base_position, ask_base_position);
        assert_eq!(quote_position, ask_quote_position);
    }

    {
        let mut merps_group = banks_client.get_account(merps_group_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&merps_group_pk, &mut merps_group).into();
        let merps_group = MerpsGroup::load_mut_checked(&account_info, &program_id).unwrap();

        let mut perp_market = banks_client.get_account(perp_market_pk).await.unwrap().unwrap();
        let account_info = (&perp_market_pk, &mut perp_market).into();
        let perp_market =
            PerpMarket::load_mut_checked(&account_info, &program_id, &merps_group_pk).unwrap();

        let mut event_queue = banks_client
            .get_account_with_commitment(event_queue_pk, CommitmentLevel::Processed)
            .await
            .unwrap()
            .unwrap();
        let account_info: AccountInfo = (&event_queue_pk, &mut event_queue).into();
        let event_queue =
            EventQueue::load_mut_checked(&account_info, &program_id, &perp_market).unwrap();

        assert!(event_queue.empty());
        assert_eq!(event_queue.header.head(), 3);

        let [e1, e2, e3] = array_ref![event_queue.debug_buf(), 0, 3];
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

    sleep(Duration::from_secs(1));

    {
        let mut merps_group = banks_client.get_account(merps_group_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&merps_group_pk, &mut merps_group).into();
        let merps_group = MerpsGroup::load_mut_checked(&account_info, &program_id).unwrap();

        let mut transaction = Transaction::new_with_payer(
            &[
                cache_prices(&program_id, &merps_group_pk, &merps_group.merps_cache, &[oracle_pk])
                    .unwrap(),
                update_funding(
                    &program_id,
                    &merps_group_pk,
                    &merps_group.merps_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                )
                .unwrap(),
                cache_root_banks(
                    &program_id,
                    &merps_group_pk,
                    &merps_group.merps_cache,
                    &[merps_group.tokens[QUOTE_INDEX].root_bank],
                )
                .unwrap(),
                cache_perp_markets(
                    &program_id,
                    &merps_group_pk,
                    &merps_group.merps_cache,
                    &[perp_market_pk],
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_ask_pk,
                    &user_ask.pubkey(),
                    &merps_group.merps_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    Side::Bid,
                    ((tsla_price + 1) * quote_unit * tsla_lot) / (tsla_unit * quote_lot),
                    (quantity * tsla_unit) / tsla_lot,
                    ask_id,
                    OrderType::Limit,
                )
                .unwrap(),
                place_perp_order(
                    &program_id,
                    &merps_group_pk,
                    &merps_account_bid_pk,
                    &user_bid.pubkey(),
                    &merps_group.merps_cache,
                    &perp_market_pk,
                    &bids_pk,
                    &asks_pk,
                    &event_queue_pk,
                    Side::Ask,
                    ((tsla_price - 1) * quote_unit * tsla_lot) / (tsla_unit * quote_lot),
                    (quantity * tsla_unit) / tsla_lot,
                    bid_id,
                    OrderType::Limit,
                )
                .unwrap(),
                consume_events(
                    &program_id,
                    &merps_group_pk,
                    &perp_market_pk,
                    &event_queue_pk,
                    &[merps_account_bid_pk, merps_account_ask_pk],
                    3,
                )
                .unwrap(),
            ],
            Some(&payer.pubkey()),
        );
        transaction.sign(&[&payer, &user_bid, &user_ask], recent_blockhash);
        assert!(banks_client.process_transaction(transaction).await.is_ok());
    }

    {
        let mut merps_account =
            banks_client.get_account(merps_account_bid_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&merps_account_bid_pk, &mut merps_account).into();
        let merps_account =
            MerpsAccount::load_mut_checked(&account_info, &program_id, &merps_group_pk).unwrap();

        let base_position = merps_account.perp_accounts[perp_market_idx].base_position;
        let quote_position = merps_account.perp_accounts[perp_market_idx].quote_position;

        // TODO: verify fees
        assert_eq!(base_position, 0);
        assert_eq!(quote_position, 0);
    }

    {
        let mut merps_account =
            banks_client.get_account(merps_account_ask_pk).await.unwrap().unwrap();
        let account_info: AccountInfo = (&merps_account_ask_pk, &mut merps_account).into();
        let merps_account =
            MerpsAccount::load_mut_checked(&account_info, &program_id, &merps_group_pk).unwrap();

        let base_position = merps_account.perp_accounts[perp_market_idx].base_position;
        let quote_position = merps_account.perp_accounts[perp_market_idx].quote_position;

        // TODO: add fees
        assert_eq!(base_position, 0);
        assert_eq!(quote_position, 0);
    }
}
