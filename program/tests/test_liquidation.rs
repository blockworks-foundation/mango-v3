// Tests related to liquidations
mod program_test;
use mango::state::*;
use program_test::*;
use program_test::cookies::*;
use program_test::scenarios::*;
use solana_program_test::*;
use fixed::types::I80F48;

#[tokio::test]
async fn test_token_and_token_liquidation() {
    // === Arrange ===
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 3, num_mints: 2 };
    let mut test = MangoProgramTest::start_new(&config).await;
    // Supress some of the logs
    solana_logger::setup_with_default(
        "solana_rbpf::vm=info,\
             solana_runtime::message_processor=debug,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info",
    );

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let bidder_user_index: usize = 0;
    let asker_user_index: usize = 1;
    let liqor_user_index: usize = 2;
    let mint_index: usize = 0;
    let base_price = 15_000;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, mint_index, base_price as f64).await;

    // Deposit amounts
    let user_deposits = vec![
        (bidder_user_index, test.quote_index, 10_000),
        (asker_user_index, mint_index, 1),
        (asker_user_index, test.quote_index, 10_000),
        (liqor_user_index, test.quote_index, 10_000),
    ];

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, user_deposits).await;

    // Step 2: Place and match an order for 1 BTC @ 15_000
    match_single_spot_order_scenario(
        &mut test,
        &mut mango_group_cookie,
        bidder_user_index,
        asker_user_index,
        mint_index,
        base_price,
    ).await;

    // Step 3: Assert that the order has been matched and the bidder has 1 BTC in deposits
    mango_group_cookie.run_keeper(&mut test).await;

    let bidder_base_deposit =
        &mango_group_cookie.mango_accounts[bidder_user_index].mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index).unwrap();
    let asker_base_deposit =
        &mango_group_cookie.mango_accounts[asker_user_index].mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index).unwrap();


    assert_eq!(bidder_base_deposit.to_string(), I80F48::from_num(1000000).to_string());
    assert_eq!(asker_base_deposit.to_string(), I80F48::from_num(0).to_string());

    // Step 4: Change the oracle price so that bidder becomes liqee
    mango_group_cookie.set_oracle(&mut test, mint_index, (base_price / 15) as f64).await;

    // Step 5: Perform a coulple liquidations
    for _ in 0..5 {
        mango_group_cookie.run_keeper(&mut test).await;
        test.perform_liquidate_token_and_token(
            &mut mango_group_cookie,
            bidder_user_index, // The liqee
            liqor_user_index,
            mint_index, // Asset index
            QUOTE_INDEX, // Liab index
        ).await;
    }

    // === Assert ===
    mango_group_cookie.run_keeper(&mut test).await;

    let bidder_base_deposit =
        &mango_group_cookie.mango_accounts[bidder_user_index].mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index).unwrap();
    let liqor_base_deposit =
        &mango_group_cookie.mango_accounts[liqor_user_index].mango_account
        .get_native_deposit(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index).unwrap();

    let bidder_base_borrow =
        &mango_group_cookie.mango_accounts[bidder_user_index].mango_account
        .get_native_borrow(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index).unwrap();
    let liqor_base_borrow =
        &mango_group_cookie.mango_accounts[liqor_user_index].mango_account
        .get_native_borrow(&mango_group_cookie.mango_cache.root_bank_cache[mint_index], mint_index).unwrap();

    println!("bidder_base_deposit: {}", bidder_base_deposit.to_string());
    println!("liqor_base_deposit: {}", liqor_base_deposit.to_string());
    println!("bidder_base_borrow: {}", bidder_base_borrow.to_string());
    println!("liqor_base_borrow: {}", liqor_base_borrow.to_string());
    // TODO: Actually assert here

}
