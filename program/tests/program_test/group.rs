use super::*;
use merps::{instruction::init_merps_group, state::*, utils::*};
use solana_program::account_info::AccountInfo;
use solana_sdk::account::Account;
use std::mem::size_of;

pub async fn init_mango_group(test: &mut MangoProgramTest) -> MerpsGroup {
    let mango_program_id = test.mango_program_id;
    let serum_program_id = test.serum_program_id;

    let merps_group_pk = test.create_account(size_of::<MerpsGroup>(), &mango_program_id).await;
    let merps_cache_pk = test.create_account(size_of::<MerpsCache>(), &mango_program_id).await;
    let (signer_pk, signer_nonce) = create_signer_key_and_nonce(&mango_program_id, &merps_group_pk);

    let quote_mint_pk = test.mints[0];
    let quote_vault_pk = test.create_token_account(&signer_pk, &quote_mint_pk).await;
    let quote_node_bank_pk = test.create_account(size_of::<NodeBank>(), &mango_program_id).await;
    let quote_root_bank_pk = test.create_account(size_of::<RootBank>(), &mango_program_id).await;

    let admin_pk = test.context.payer.pubkey();
    let instructions = [init_merps_group(
        &mango_program_id,
        &merps_group_pk,
        &signer_pk,
        &admin_pk,
        &quote_mint_pk,
        &quote_vault_pk,
        &quote_node_bank_pk,
        &quote_root_bank_pk,
        &merps_cache_pk,
        &serum_program_id,
        signer_nonce,
        5,
    )
    .unwrap()];

    test.process_transaction(&instructions, None).await.unwrap();

    let merps_group = test.load_account::<MerpsGroup>(merps_group_pk).await;
    return merps_group;
}
