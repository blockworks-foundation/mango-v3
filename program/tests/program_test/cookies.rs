use std::mem::size_of;
use std::num::NonZeroU64;
use fixed::types::I80F48;

use solana_sdk::{
    instruction::Instruction,
    signature::{Keypair, Signer},
    transaction::Transaction,
    transport::TransportError,
};
use solana_program::{
    account_info::AccountInfo,
    clock::{Clock, UnixTimestamp},
    program_error::ProgramError,
    program_option::COption,
    program_pack::Pack,
    pubkey::*,
    rent::*,
    system_instruction, sysvar,
};

use mango::{
    entrypoint::*, ids::*, instruction::*, matching::*, oracle::*, queue::*, state::*, utils::*,
};

use crate::*;

#[derive(Copy, Clone)]
pub struct MintCookie {
    // pub symbol: String,
    pub index: usize,
    pub decimals: u8,
    pub unit: u64,
    pub base_lot: u64,
    pub quote_lot: u64,
    pub pubkey: Option<Pubkey>,
}

pub struct MangoGroupCookie {

    pub address: Option<Pubkey>,

    pub mango_group: Option<MangoGroup>,

    pub mango_cache: MangoCache,

    // oracles are available from mango_group

    pub mango_accounts: Vec<MangoAccountCookie>,

    pub spot_markets: Vec<SpotMarketCookie>,

    pub perp_markets: Vec<PerpMarketCookie>,

    pub quote_mint: Option<MintCookie>,

}

impl MangoGroupCookie {

    #[allow(dead_code)]
    pub async fn default(
        test: &mut MangoProgramTest,
    ) -> Self {
        let mango_program_id = test.mango_program_id;
        let serum_program_id = test.serum_program_id;

        let mango_group_pk = test.create_account(size_of::<MangoGroup>(), &mango_program_id).await;
        let mango_cache_pk = test.create_account(size_of::<MangoCache>(), &mango_program_id).await;
        let (signer_pk, signer_nonce) =
            create_signer_key_and_nonce(&mango_program_id, &mango_group_pk);
        let admin_pk = test.get_payer_pk();

        let quote_mint_pk = test.mints[test.quote_index].pubkey.unwrap();
        let quote_vault_pk = test.create_token_account(&signer_pk, &quote_mint_pk).await;
        let quote_node_bank_pk =
            test.create_account(size_of::<NodeBank>(), &mango_program_id).await;
        let quote_root_bank_pk =
            test.create_account(size_of::<RootBank>(), &mango_program_id).await;
        let dao_vault_pk = test.create_token_account(&signer_pk, &quote_mint_pk).await;
        let msrm_vault_pk = test.create_token_account(&signer_pk, &msrm_token::ID).await;

        let quote_optimal_util = I80F48::from_num(0.7);
        let quote_optimal_rate = I80F48::from_num(0.06);
        let quote_max_rate = I80F48::from_num(1.5);

        let instructions = [
            mango::instruction::init_mango_group(
                &mango_program_id,
                &mango_group_pk,
                &signer_pk,
                &admin_pk,
                &quote_mint_pk,
                &quote_vault_pk,
                &quote_node_bank_pk,
                &quote_root_bank_pk,
                &dao_vault_pk,
                &msrm_vault_pk,
                &mango_cache_pk,
                &serum_program_id,
                signer_nonce,
                5,
                quote_optimal_util,
                quote_optimal_rate,
                quote_max_rate,
            )
            .unwrap()
        ];

        test.process_transaction(&instructions, None).await.unwrap();

        let mango_group = test.load_account::<MangoGroup>(mango_group_pk).await;
        let mango_cache = test.load_account::<MangoCache>(mango_group.mango_cache).await;
        let quote_mint = test.with_mint(test.quote_index);

        MangoGroupCookie { address: Some(mango_group_pk), mango_group: Some(mango_group), mango_cache: mango_cache, mango_accounts: vec![], spot_markets: vec![], perp_markets: vec![], quote_mint: Some(quote_mint) }
    }

    #[allow(dead_code)]
    pub async fn full_setup(
        &mut self,
        test: &mut MangoProgramTest,
        num_users: usize,
        num_markets: usize,
    ) {
        test.add_oracles_to_mango_group(&self.address.unwrap()).await;
        self.mango_accounts = self.add_mango_accounts(test, num_users).await;
        self.spot_markets = self.add_spot_markets(test, num_markets).await;
        self.perp_markets = self.add_perp_markets(test, num_markets).await;
        self.mango_group = Some(test.load_account::<MangoGroup>(self.address.unwrap()).await);
    }

    #[allow(dead_code)]
    pub async fn add_mango_accounts(
        &mut self,
        test: &mut MangoProgramTest,
        num_users: usize,
    ) -> Vec<MangoAccountCookie> {
        let mut mango_accounts = Vec::new();
        for i in 0..num_users {
            mango_accounts.push(MangoAccountCookie::init(test, self, i).await);
        }
        mango_accounts
    }

    #[allow(dead_code)]
    pub async fn add_perp_markets(
        &mut self,
        test: &mut MangoProgramTest,
        num_markets: usize,
    ) -> (Vec<PerpMarketCookie>) {
        let mut perp_markets = Vec::new();
        for i in 0..num_markets {
            perp_markets.push(PerpMarketCookie::init(test, self, i).await);
        }
        perp_markets
    }

    #[allow(dead_code)]
    pub async fn add_spot_markets(
        &mut self,
        test: &mut MangoProgramTest,
        num_markets: usize,
    ) -> (Vec<SpotMarketCookie>) {
        let mut spot_markets = Vec::new();
        for i in 0..num_markets {
            spot_markets.push(SpotMarketCookie::init(test, self, i).await);
        }
        spot_markets
    }

    #[allow(dead_code)]
    pub async fn set_oracle(
        &mut self,
        test: &mut MangoProgramTest,
        oracle_index: usize,
        price: u64,
    ) {
        let mint = test.with_mint(oracle_index);
        let oracle_price = test.with_oracle_price(&mint, price);
        let mango_program_id = test.mango_program_id;
        let admin_pk = test.get_payer_pk();
        let oracle_pk = self.mango_group.unwrap().oracles[oracle_index];
        let instructions = [
            mango::instruction::set_oracle(
                &mango_program_id,
                &self.address.unwrap(),
                &oracle_pk,
                &admin_pk,
                oracle_price,
            )
            .unwrap(),
        ];
        test.process_transaction(&instructions, None).await.unwrap();
    }

    #[allow(dead_code)]
    pub async fn run_keeper(
        &mut self,
        test: &mut MangoProgramTest,
    ) {
        let mango_group = self.mango_group.unwrap();
        let mango_group_pk = self.address.unwrap();
        let oracle_pks = mango_group.oracles.iter()
            .filter(|x| **x != Pubkey::default())
            .map(|x| *x).collect::<Vec<Pubkey>>();
        let perp_market_pks = self.perp_markets.iter().map(|x| x.address.unwrap()).collect::<Vec<Pubkey>>();

        test.advance_clock().await;
        test.cache_all_prices(&mango_group, &mango_group_pk, &oracle_pks[..]).await;
        test.update_all_root_banks(&mango_group, &mango_group_pk).await;
        test.cache_all_root_banks(&mango_group, &mango_group_pk).await;
        test.cache_all_perp_markets(&mango_group, &mango_group_pk, &perp_market_pks).await;
        self.mango_cache =
            test.load_account::<MangoCache>(mango_group.mango_cache).await;
    }

}


pub struct MangoAccountCookie {

    pub address: Option<Pubkey>,

    pub mango_account: Option<MangoAccount>,

}

impl MangoAccountCookie {

    #[allow(dead_code)]
    pub fn default() -> Self {
        MangoAccountCookie { address: None, mango_account: None }
    }

    #[allow(dead_code)]
    pub async fn init(
        test: &mut MangoProgramTest,
        mango_group_cookie: &mut MangoGroupCookie,
        user_index: usize,
    ) -> Self {

        let mango_program_id = test.mango_program_id;
        let mango_account_pk =
            test.create_account(size_of::<MangoAccount>(), &mango_program_id).await;
        let user = Keypair::from_base58_string(&test.users[user_index].to_base58_string());
        let user_pk = user.pubkey();

        let instructions = [
            mango::instruction::init_mango_account(
                &mango_program_id,
                &mango_group_cookie.address.unwrap(),
                &mango_account_pk,
                &user_pk,
            ).unwrap()
        ];
        test.process_transaction(&instructions, Some(&[&user])).await.unwrap();
        let mango_account = test.load_account::<MangoAccount>(mango_account_pk).await;
        MangoAccountCookie { address: Some(mango_account_pk), mango_account: Some(mango_account) }

    }

}

#[derive(Copy, Clone)]
pub struct SpotMarketCookie {

    pub market: Pubkey,

    pub req_q: Pubkey,

    pub event_q: Pubkey,

    pub bids: Pubkey,

    pub asks: Pubkey,

    pub coin_vault: Pubkey,

    pub pc_vault: Pubkey,

    pub vault_signer_key: Pubkey,

    pub mint: MintCookie,

}

impl SpotMarketCookie {

    pub async fn init(
        test: &mut MangoProgramTest,
        mango_group_cookie: &mut MangoGroupCookie,
        mint_index: usize,
    ) -> Self {
        let mango_program_id = test.mango_program_id;
        let serum_program_id = test.serum_program_id;

        let mango_group = mango_group_cookie.mango_group.unwrap();
        let mango_group_pk = mango_group_cookie.address.unwrap();

        let mut spot_market_cookie =
            test.list_spot_market(mint_index).await;

        let (signer_pk, _signer_nonce) =
            create_signer_key_and_nonce(&mango_program_id, &mango_group_pk);

        let vault_pk = test
            .create_token_account(&signer_pk, &test.mints[mint_index].pubkey.unwrap())
            .await;
        let node_bank_pk = test.create_account(size_of::<NodeBank>(), &mango_program_id).await;
        let root_bank_pk = test.create_account(size_of::<RootBank>(), &mango_program_id).await;
        let init_leverage = I80F48::from_num(10);
        let maint_leverage = init_leverage * 2;
        let optimal_util = I80F48::from_num(0.7);
        let optimal_rate = I80F48::from_num(0.06);
        let max_rate = I80F48::from_num(1.5);

        let admin_pk = test.get_payer_pk();

        let instructions = [
            mango::instruction::add_spot_market(
                &mango_program_id,
                &mango_group_pk,
                &spot_market_cookie.market,
                &serum_program_id,
                &test.mints[mint_index].pubkey.unwrap(),
                &node_bank_pk,
                &vault_pk,
                &root_bank_pk,
                &admin_pk,
                mint_index,
                maint_leverage,
                init_leverage,
                optimal_util,
                optimal_rate,
                max_rate,
            ).unwrap(),
        ];

        test.process_transaction(&instructions, None).await.unwrap();
        spot_market_cookie.mint = test.with_mint(mint_index);
        spot_market_cookie

    }

    pub async fn place_order(
        &mut self,
        test: &mut MangoProgramTest,
        mango_group_cookie: &mut MangoGroupCookie,
        user_index: usize,
        order_id: u64,
        side: serum_dex::matching::Side,
        size: u64,
        price: u64,
    ) {
        let order = serum_dex::instruction::NewOrderInstructionV3 {
            side: side, //serum_dex::matching::Side::Bid,
            limit_price: NonZeroU64::new(price as u64).unwrap(),
            max_coin_qty: NonZeroU64::new(test.baseSizeNumberToLots(&self.mint, size) as u64).unwrap(),
            max_native_pc_qty_including_fees: NonZeroU64::new(test.quoteSizeNumberToLots(&self.mint, size * price) as u64).unwrap(),
            self_trade_behavior: serum_dex::instruction::SelfTradeBehavior::DecrementTake,
            order_type: serum_dex::matching::OrderType::Limit,
            client_order_id: order_id,
            limit: u16::MAX,
        };

        test.place_spot_order(
            &mango_group_cookie,
            self,
            user_index,
            order,
        ).await;

        mango_group_cookie.mango_accounts[user_index].mango_account =
            Some(test.load_account::<MangoAccount>(mango_group_cookie.mango_accounts[user_index].address.unwrap()).await);
    }

    pub async fn settle_funds(
        &mut self,
        test: &mut MangoProgramTest,
        mango_group_cookie: &mut MangoGroupCookie,
        user_index: usize,
    ) {
        test.settle_funds(
            &mango_group_cookie,
            self,
            user_index,
        ).await;

        mango_group_cookie.mango_accounts[user_index].mango_account =
            Some(test.load_account::<MangoAccount>(mango_group_cookie.mango_accounts[user_index].address.unwrap()).await);
    }
}

#[derive(Copy, Clone)]
pub struct PerpMarketCookie {

    pub address: Option<Pubkey>,

    pub perp_market: Option<PerpMarket>,

    pub mint: MintCookie,

}

impl PerpMarketCookie {

    pub async fn init(
        test: &mut MangoProgramTest,
        mango_group_cookie: &mut MangoGroupCookie,
        mint_index: usize,
    ) -> Self {
        let mango_program_id = test.mango_program_id;
        let mango_group_pk = mango_group_cookie.address.unwrap();
        let perp_market_pk = test.create_account(size_of::<PerpMarket>(), &mango_program_id).await;
        let (signer_pk, _signer_nonce) =
            create_signer_key_and_nonce(&mango_program_id, &mango_group_pk);
        let max_num_events = 32;
        let event_queue_pk = test
            .create_account(
                size_of::<EventQueue>() + size_of::<AnyEvent>() * max_num_events,
                &mango_program_id,
            )
            .await;
        let bids_pk = test.create_account(size_of::<BookSide>(), &mango_program_id).await;
        let asks_pk = test.create_account(size_of::<BookSide>(), &mango_program_id).await;
        let mngo_vault_pk = test.create_token_account(&signer_pk, &mngo_token::ID).await;

        let admin_pk = test.get_payer_pk();

        let init_leverage = I80F48::from_num(10);
        let maint_leverage = init_leverage * 2;
        let liquidation_fee = I80F48::from_num(0.025);
        let maker_fee = I80F48::from_num(0.01);
        let taker_fee = I80F48::from_num(0.01);
        let rate = I80F48::from_num(1);
        let max_depth_bps = I80F48::from_num(200);
        let target_period_length = 3600;
        let mngo_per_period = 11400;

        let instructions = [mango::instruction::add_perp_market(
            &mango_program_id,
            &mango_group_pk,
            &perp_market_pk,
            &event_queue_pk,
            &bids_pk,
            &asks_pk,
            &mngo_vault_pk,
            &admin_pk,
            mint_index,
            maint_leverage,
            init_leverage,
            liquidation_fee,
            maker_fee,
            taker_fee,
            test.mints[mint_index].base_lot as i64,
            test.mints[mint_index].quote_lot as i64,
            rate,
            max_depth_bps,
            target_period_length,
            mngo_per_period,
        )
        .unwrap()];

        test.process_transaction(&instructions, None).await.unwrap();

        let perp_market = test.load_account::<PerpMarket>(perp_market_pk).await;
        PerpMarketCookie {
            address: Some(perp_market_pk),
            perp_market: Some(perp_market),
            mint: test.with_mint(mint_index),
        }
    }

    pub async fn place_order(
        &mut self,
        test: &mut MangoProgramTest,
        mango_group_cookie: &mut MangoGroupCookie,
        user_index: usize,
        order_id: u64,
        side: mango::matching::Side,
        size: u64,
        price: u64,
    ) {
        let order_size = test.baseSizeNumberToLots(&self.mint, size);
        let order_price = test.with_order_price(&self.mint, price);

        test.place_perp_order(
            &mango_group_cookie,
            self,
            user_index,
            side,
            order_size,
            order_price,
            order_id,
            mango::matching::OrderType::Limit,
        ).await;

        mango_group_cookie.mango_accounts[user_index].mango_account =
            Some(test.load_account::<MangoAccount>(mango_group_cookie.mango_accounts[user_index].address.unwrap()).await);

    }

}
