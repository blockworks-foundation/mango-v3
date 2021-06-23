use solana_program::{system_instruction, rent::*, pubkey::*};
use solana_program_test::*;
use solana_sdk::{
  instruction::Instruction,
  signature::{Signer,Keypair},
  transaction::Transaction,
  transport::TransportError,
};
use merps::entrypoint::process_instruction;

pub mod group;


pub struct MangoProgramTestConfig {
  pub compute_limit: u64,
  pub num_users: u64,
  pub num_mints: u64,
}

impl MangoProgramTestConfig {
  pub fn default() -> Self {
    MangoProgramTestConfig {
      compute_limit: 10_000,
      num_users: 2,
      num_mints: 2,
    }
  }
}


pub struct MangoProgramTest {
  pub context: ProgramTestContext,
  pub rent: Rent,
  pub mango_program_id: Pubkey,
  pub serum_program_id: Pubkey,
  pub users: Vec<KeyPair>,
  pub mints: Vec<Pubkey>,
  pub tokenAccounts: Vec<Pubkey>, // user x mint
}


impl MangoProgramTest {
  pub async fn start_new(config: &MangoProgramTestConfig) -> Self {
    let mango_program_id = Pubkey::new_unique();
    let serum_program_id = Pubkey::new_unique();

    let mut test = ProgramTest::new(
        "merps",
        mango_program_id,
        processor!(process_instruction),
    );

    // passing mango's process instruction just to satisfy the compiler
    test.add_program("serum_dex", serum_program_id, processor!(process_instruction));
    // TODO: add more programs (oracles)

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(compute_limit);

    // add mints in loop
    let mint_decimals = 6;
    let mint_pk = Pubkey::new_unique();
    test.add_packable_account(
      mint_pk,
      u32::MAX as u64,
      &spl_token::Mint {
          is_initialized: true,
          mint_authority: COption::Some(Pubkey::new_unique),
          decimals: mint_decimals,
          ..spl_token::Mint::default()
      },
      &spl_token::id(),
    );
    // add mint_pk to mints

    // add users in loop
    let user_key = Keypair::new();
    test.add_account(user.pubkey(), Account::new(u32::MAX as u64, 0, &user.pubkey()));


    // add user vaults in loop
    let pubkey = Pubkey::new_unique();
    test.add_packable_account(
        pubkey,
        u32::MAX as u64,
        &Token {
            mint: mint,
            owner: owner,
            amount: 1_000_000_000_000,
            state: AccountState::Initialized,
            ..Token::default()
        },
        &spl_token::id(),
    );

    /*
    test.add_account(user.pubkey(), Account::new(u32::MAX as u64, 0, &user.pubkey()));

    let user_quote_account = add_token_account(
        &mut test,
        user.pubkey(),
        merps_group.tokens[quote_index].pubkey,
        user_initial_amount,
    );
     */ 


    let mut context = test.start_with_context().await;
    let rent = context.banks_client.get_rent().await.unwrap();

    Self {
        context,
        rent,
        mango_program_id,
        serum_program_id
    }
  }

    pub async fn process_transaction(
      &mut self,
      instructions: &[Instruction],
      signers: Option<&[&Keypair]>,
  ) -> Result<(), TransportError> {
      let mut transaction =
          Transaction::new_with_payer(&instructions, Some(&self.context.payer.pubkey()));
  
      let mut all_signers = vec![&self.context.payer];
  
      if let Some(signers) = signers {
          all_signers.extend_from_slice(signers);
      }
  
      let recent_blockhash = self
          .context
          .banks_client
          .get_recent_blockhash()
          .await
          .unwrap();
  
      transaction.sign(&all_signers, recent_blockhash);
  
      self.context
          .banks_client
          .process_transaction(transaction)
          .await
          .unwrap();
  
      Ok(())
  }

  pub async fn create_account(&mut self, size: usize, owner: &Pubkey) -> Keypair {
    let keypair = Keypair::new();
    let rent = self.rent.minimum_balance(size);

    let instructions = [
        system_instruction::create_account(
            &self.context.payer.pubkey(),
            &keypair.pubkey(),
            rent as u64,
            size as u64,
            owner,
        )
    ];

    self.process_transaction(&instructions, Some(&[&keypair]))
        .await
        .unwrap();

    return keypair;
  }

  pub async fn create_mint(&mut self, mint_authority: &Pubkey) {
    let keypair = Keypair::new();
    let mint_rent = self.rent.minimum_balance(spl_token::state::Mint::LEN);

    let instructions = [
        system_instruction::create_account(
            &self.context.payer.pubkey(),
            &mint_keypair.pubkey(),
            mint_rent,
            spl_token::state::Mint::LEN as u64,
            &spl_token::id(),
        ),
        spl_token::instruction::initialize_mint(
            &spl_token::id(),
            &mint_keypair.pubkey(),
            &mint_authority,
            None,
            0,
        )
        .unwrap(),
    ];

    self.process_transaction(&instructions, Some(&[&mint_keypair]))
        .await
        .unwrap();

    return keypair;
}


}

  