// Tests related to placing orders on a perp market
mod helpers;
use std::mem::size_of;

use fixed::types::I80F48;
use helpers::*;

use merps::{entrypoint::process_instruction, instruction::*, matching::*, queue::*, state::*};
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
    let user_initial_amount = 200;
    let user_quote_account = add_token_account(
        &mut test,
        user.pubkey(),
        merps_group.tokens[quote_index].pubkey,
        user_initial_amount,
    );

    let merps_account_pk = add_test_account_with_owner::<MerpsAccount>(&mut test, &program_id);

    let tsla_decimals: u8 = 4;
    let tsla_price = 9000;
    let unit = 10u64.pow(tsla_decimals as u32);
    let tsla_usd =
        add_aggregator(&mut test, "TSLA:USD", tsla_decimals, tsla_price * unit, &program_id);

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
                add_oracle(&program_id, &merps_group_pk, &tsla_usd.pubkey, &admin.pubkey())
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
    let user_initial_amount = 200;
    let user_quote_account = add_token_account(
        &mut test,
        user.pubkey(),
        merps_group.tokens[quote_index].pubkey,
        user_initial_amount,
    );

    let merps_account_pk = add_test_account_with_owner::<MerpsAccount>(&mut test, &program_id);

    let tsla_decimals: u8 = 4;
    let tsla_price = 100;
    let unit = 10u64.pow(tsla_decimals as u32);
    let tsla_usd =
        add_aggregator(&mut test, "TSLA:USD", tsla_decimals, tsla_price * unit, &program_id);

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
                add_oracle(&program_id, &merps_group_pk, &tsla_usd.pubkey, &admin.pubkey())
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
                cache_prices(
                    &program_id,
                    &merps_group_pk,
                    &merps_group.merps_cache,
                    &[tsla_usd.pubkey],
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
                    &[merps_group.perp_markets[perp_market_idx].perp_market],
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
                    ((tsla_price - 1) * unit) as i64,
                    10,
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
                    ((tsla_price + 1) * unit) as i64,
                    10,
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

    let user = Keypair::new();
    test.add_account(user.pubkey(), Account::new(u32::MAX as u64, 0, &user.pubkey()));

    // TODO: this still needs to be deposited into the merps account
    let quote_index = 0;
    let user_initial_amount = 200;
    let user_quote_account = add_token_account(
        &mut test,
        user.pubkey(),
        merps_group.tokens[quote_index].pubkey,
        user_initial_amount,
    );

    let merps_account_pk = add_test_account_with_owner::<MerpsAccount>(&mut test, &program_id);

    let tsla_decimals: u8 = 4;
    let tsla_price = 100;
    let unit = 10u64.pow(tsla_decimals as u32);
    let tsla_usd =
        add_aggregator(&mut test, "TSLA:USD", tsla_decimals, tsla_price * unit, &program_id);

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
                add_oracle(&program_id, &merps_group_pk, &tsla_usd.pubkey, &admin.pubkey())
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
                cache_prices(
                    &program_id,
                    &merps_group_pk,
                    &merps_group.merps_cache,
                    &[tsla_usd.pubkey],
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
                    &[merps_group.perp_markets[perp_market_idx].perp_market],
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
                    ((tsla_price + 1) * unit) as i64,
                    10,
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
                    ((tsla_price - 1) * unit) as i64,
                    10,
                    ask_id,
                    OrderType::Limit,
                )
                .unwrap(),
                consume_events(&program_id, &merps_group_pk, &perp_market_pk, &event_queue_pk, 1)
                    .unwrap(),
            ],
            Some(&payer.pubkey()),
        );
        transaction.sign(&[&payer, &user], recent_blockhash);
        assert!(banks_client.process_transaction(transaction).await.is_ok());
    }

    // TODO verify position
}
