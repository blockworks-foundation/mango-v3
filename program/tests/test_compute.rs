use solana_program_test::*;

use mango::state::*;
use program_test::*;

mod program_test;

#[tokio::test]
async fn test_add_all_markets_to_mango_group() {
    // Arrange
    let config = MangoProgramTestConfig { compute_limit: 200_000, num_users: 1, num_mints: 32 };
    let mut test = MangoProgramTest::start_new(&config).await;
    solana_logger::setup_with_default(
        "solana_rbpf::vm=info,\
             solana_runtime::message_processor=info,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info",
    );

    let quote_index = config.num_mints - 1;

    let (mango_group_pk, _mango_group) = test.with_mango_group().await;
    test.with_oracles(&mango_group_pk, quote_index).await;
    test.add_spot_markets_to_mango_group(&mango_group_pk).await;

    let mango_group = test.load_account::<MangoGroup>(mango_group_pk).await;

    let user_index = 0;
    let (mango_account_pk, _mango_account) =
        test.with_mango_account(&mango_group_pk, user_index).await;
    println!("Performing deposit");

    for i in 0..config.num_mints {
        test.perform_deposit(
            &mango_group,
            &mango_group_pk,
            &mango_account_pk,
            user_index,
            i as usize,
            1000000,
        )
        .await;
    }
}
