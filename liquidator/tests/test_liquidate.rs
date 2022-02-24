use fixed::types::I80F48;
use fixed_macro::types::I80F48;

use mango::state::AssetType;
use mango::state::QUOTE_INDEX;
use safe_transmute::transmute_one_to_bytes;
use solana_program::clock::Epoch;
use solana_program_test::*;
use solana_sdk::account::WritableAccount;
use std::str::FromStr;

use mango_test::cookies::*;
use mango_test::scenarios::*;
use mango_test::*;

#[tokio::test]
async fn test_liquidate() {
    // === Arrange ===
    let config = MangoProgramTestConfig::default();
    let mut test = MangoProgramTest::start_new(&config).await;

    let mut mango_group_cookie = MangoGroupCookie::default(&mut test).await;
    mango_group_cookie.full_setup(&mut test, config.num_users, config.num_mints - 1).await;

    // General parameters
    let liqor_index: usize = 0;
    let liqee_index: usize = 1;
    let btc_index = 2;
    let eth_index = 3;

    // Set oracles
    mango_group_cookie.set_oracle(&mut test, btc_index, 50_000.0).await;
    mango_group_cookie.set_oracle(&mut test, eth_index, 2_500.0).await;

    // Deposit amounts
    let liqor_deposits = vec![
        (liqor_index, btc_index, 10.0),
        (liqor_index, eth_index, 100.0),
        (liqor_index, test.quote_index, 1_000_000.0),
    ];

    let mut liqee_account_override = mango_group_cookie.mango_accounts[liqee_index].mango_account;
    liqee_account_override.deposits[btc_index] = I80F48!(1);
    liqee_account_override.deposits[test.quote_index] = I80F48!(100_000);
    liqee_account_override.perp_accounts[btc_index].base_position = -20_000;
    liqee_account_override.perp_accounts[btc_index].quote_position = I80F48!(50_000_000_000);
    liqee_account_override.perp_accounts[eth_index].quote_position = I80F48!(-50_000_000_000);

    // === Act ===
    // Step 1: Make deposits
    deposit_scenario(&mut test, &mut mango_group_cookie, &liqor_deposits).await;

    let acc = WritableAccount::create(
        10_000_000,
        transmute_one_to_bytes(&liqee_account_override).to_vec(),
        test.mango_program_id,
        bool::default(),
        Epoch::default(),
    );
    // AccountSharedData::new_data(10_000_000, &liqee_account_override, &test.mango_program_id);
    test.context.set_account(&mango_group_cookie.mango_accounts[liqee_index].address, &acc);

    // account net is 100k usdc -1 btc, so guaranteed liq at btc=100k
    mango_group_cookie.set_oracle(&mut test, btc_index, 100_000.0).await;

    mango_group_cookie.run_keeper(&mut test).await;

    test.perform_liquidate_perp_market(
        &mut mango_group_cookie,
        btc_index,
        liqee_index,
        liqor_index,
        -100000,
    )
    .await;

    test.perform_liquidate_token_and_perp(
        &mut mango_group_cookie,
        liqee_index,
        liqor_index,
        AssetType::Token,
        btc_index,
        AssetType::Perp,
        btc_index,
        I80F48!(1_000_000_000_000),
    )
    .await;

    test.perform_liquidate_token_and_perp(
        &mut mango_group_cookie,
        liqee_index,
        liqor_index,
        AssetType::Token,
        test.quote_index,
        AssetType::Perp,
        eth_index,
        I80F48!(1_000_000_000_000),
    )
    .await;
}
