use super::*;
use mango::{state::*, utils::*};
use solana_program::account_info::AccountInfo;
use solana_sdk::account::Account;
use std::mem::size_of;

pub async fn init_mango_group(test: &mut MangoProgramTest) -> MangoGroup {
    let mango_program_id = test.mango_program_id;
    let serum_program_id = test.serum_program_id;

    let mango_group_pk = test.create_account(size_of::<MangoGroup>(), &mango_program_id).await;
    let mango_cache_pk = test.create_account(size_of::<MangoCache>(), &mango_program_id).await;
    let (signer_pk, signer_nonce) = create_signer_key_and_nonce(&mango_program_id, &mango_group_pk);

    let quote_mint_pk = test.mints[0];
    let quote_vault_pk = test.create_token_account(&signer_pk, &quote_mint_pk).await;
    let quote_node_bank_pk = test.create_account(size_of::<NodeBank>(), &mango_program_id).await;
    let quote_root_bank_pk = test.create_account(size_of::<RootBank>(), &mango_program_id).await;

    let admin_pk = test.context.payer.pubkey();
    let instructions = [mango::instruction::init_mango_group(
        &mango_program_id,
        &mango_group_pk,
        &signer_pk,
        &admin_pk,
        &quote_mint_pk,
        &quote_vault_pk,
        &quote_node_bank_pk,
        &quote_root_bank_pk,
        &mango_cache_pk,
        &serum_program_id,
        signer_nonce,
        5,
    )
    .unwrap()];

    test.process_transaction(&instructions, None).await.unwrap();

    let mango_group = test.load_account::<MangoGroup>(mango_group_pk).await;
    return mango_group;
}
