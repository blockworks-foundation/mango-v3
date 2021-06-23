use std::{
    mem::{size_of},
};
use solana_sdk::{
  account::Account
};
use merps::{
  state::*,
  utils::*
};
use super::*;

pub fn init_mango_group(test: &mut MangoProgramTest) {

    let mango_program_id = test.mango_program_id;

    let merps_group = test.create_account(size_of::<MerpsGroup>(), &mango_program_id);
    let merps_cache = test.create_account(size_of::<MerpsCache>(), &mango_program_id);

    /*
      let quote_mint = add_mint(test, 6);
    let quote_vault = add_token_account(test, signer_pk, quote_mint.pubkey, 0);
    let quote_node_bank = add_node_bank(test, &program_id, quote_vault.pubkey);
    let quote_root_bank = add_root_bank(test, &program_id, quote_node_bank);
 */




    // let (signer_pk, signer_nonce) = create_signer_key_and_nonce(&mango_program_id, &merps_group_key.pubkey());
}