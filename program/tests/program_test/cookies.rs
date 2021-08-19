use std::mem::size_of;
use std::num::NonZeroU64;
use fixed::types::I80F48;

use solana_program::pubkey::Pubkey;
use solana_sdk::{
    signature::{Keypair, Signer},
};

use mango::{
    ids::*, matching::*, queue::*, state::*, utils::*,
};

use crate::*;

pub const STARTING_SPOT_ORDER_ID: u64 = 0;
pub const STARTING_PERP_ORDER_ID: u64 = 10_000;

#[derive(Copy, Clone)]
pub struct MintCookie {

    pub index: usize,
    pub decimals: u8,
    pub unit: f64,
    pub base_lot: f64,
    pub quote_lot: f64,
    pub pubkey: Option<Pubkey>,

}

pub struct MangoGroupCookie {

    pub address: Pubkey,

    pub mango_group: MangoGroup,

    pub mango_cache: MangoCache,

    // oracles are available from mango_group

    pub mango_accounts: Vec<MangoAccountCookie>,

    pub spot_markets: Vec<SpotMarketCookie>,

    pub perp_markets: Vec<PerpMarketCookie>,

    pub current_spot_order_id: u64,

    pub current_perp_order_id: u64,

    pub users_with_spot_event: Vec<Vec<usize>>,

    pub users_with_perp_event: Vec<Vec<usize>>,

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
        let fees_vault_pk = test.create_token_account(&signer_pk, &quote_mint_pk).await;

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
                &fees_vault_pk,
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

        MangoGroupCookie {
            address: mango_group_pk,
            mango_group: mango_group,
            mango_cache: mango_cache,
            mango_accounts: vec![],
            spot_markets: vec![],
            perp_markets: vec![],
            current_spot_order_id: STARTING_SPOT_ORDER_ID,
            current_perp_order_id: STARTING_PERP_ORDER_ID,
            users_with_spot_event: vec![Vec::new(); test.num_mints - 1],
            users_with_perp_event: vec![Vec::new(); test.num_mints - 1],
        }

    }

    #[allow(dead_code)]
    pub async fn full_setup(
        &mut self,
        test: &mut MangoProgramTest,
        num_users: usize,
        num_markets: usize,
    ) {

        test.add_oracles_to_mango_group(&self.address).await;
        self.mango_accounts = self.add_mango_accounts(test, num_users).await;
        self.spot_markets = self.add_spot_markets(test, num_markets).await;
        self.perp_markets = self.add_perp_markets(test, num_markets).await;
        self.mango_group = test.load_account::<MangoGroup>(self.address).await;

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
    ) -> Vec<PerpMarketCookie> {

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
    ) -> Vec<SpotMarketCookie> {

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
        price: f64,
    ) {

        let mint = test.with_mint(oracle_index);
        let oracle_price = test.with_oracle_price(&mint, price);
        let mango_program_id = test.mango_program_id;
        let admin_pk = test.get_payer_pk();
        let oracle_pk = self.mango_group.oracles[oracle_index];
        let instructions = [
            mango::instruction::set_oracle(
                &mango_program_id,
                &self.address,
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

        let mango_group = self.mango_group;
        let mango_group_pk = self.address;
        let oracle_pks = mango_group.oracles.iter()
            .filter(|x| **x != Pubkey::default())
            .map(|x| *x).collect::<Vec<Pubkey>>();
        let perp_market_pks = self.perp_markets.iter().map(|x| x.address).collect::<Vec<Pubkey>>();

        test.advance_clock().await;
        test.cache_all_prices(&mango_group, &mango_group_pk, &oracle_pks[..]).await;
        test.update_all_root_banks(&mango_group, &mango_group_pk).await;
        test.cache_all_root_banks(&mango_group, &mango_group_pk).await;
        test.cache_all_perp_markets(&mango_group, &mango_group_pk, &perp_market_pks).await;
        self.mango_cache =
            test.load_account::<MangoCache>(mango_group.mango_cache).await;
        for user_index in 0..self.mango_accounts.len() {
            self.mango_accounts[user_index].mango_account =
                test.load_account::<MangoAccount>(self.mango_accounts[user_index].address).await;
        }

    }

    #[allow(dead_code)]
    pub async fn consume_spot_events(
        &mut self,
        test: &mut MangoProgramTest,
    ) {

        for spot_market_index in 0..self.users_with_spot_event.len() {
            let users_with_spot_event = &self.users_with_spot_event[spot_market_index];
            if users_with_spot_event.len() > 0 {
                let spot_market_cookie = self.spot_markets[spot_market_index];
                let mut open_orders = Vec::new();
                for user_index in users_with_spot_event {
                    open_orders.push(&self.mango_accounts[*user_index].mango_account.spot_open_orders[spot_market_index]);
                }
                test.consume_spot_events(
                    &spot_market_cookie,
                    open_orders,
                    0, // TODO: Change coin_fee_receivable_account, pc_fee_receivable_account to owner of test
                ).await;
            }
            self.users_with_spot_event[spot_market_index] = Vec::new();
        }
        self.run_keeper(test).await;

    }

    #[allow(dead_code)]
    pub async fn settle_spot_funds(
        &mut self,
        test: &mut MangoProgramTest,
        spot_orders: &Vec<(usize, usize, serum_dex::matching::Side, f64, f64)>,
    ) {

        for spot_order in spot_orders {
            let (user_index, market_index, order_side, order_size, order_price) = *spot_order;
            let spot_market_cookie = self.spot_markets[market_index];
            test.settle_spot_funds(self, &spot_market_cookie, user_index).await;
        }

    }

    #[allow(dead_code)]
    pub async fn consume_perp_events(
        &mut self,
        test: &mut MangoProgramTest,
    ) {
        for perp_market_index in 0..self.users_with_perp_event.len() {
            let users_with_perp_event = &self.users_with_perp_event[perp_market_index];
            if users_with_perp_event.len() > 0 {
                let perp_market_cookie = self.perp_markets[perp_market_index];
                let mut mango_account_pks = Vec::new();
                for user_index in users_with_perp_event {
                    mango_account_pks.push(self.mango_accounts[*user_index].address);
                }
                test.consume_perp_events(
                    &self,
                    &perp_market_cookie,
                    &mut mango_account_pks,
                ).await;
            }
            self.users_with_perp_event[perp_market_index] = Vec::new();
        }
        self.run_keeper(test).await;
    }

    // NOTE: This function assumes an array of perp orders for the same market (coming from match_perp_order_scenario)
    #[allow(dead_code)]
    pub async fn settle_perp_funds(
        &mut self,
        test: &mut MangoProgramTest,
        perp_orders: &Vec<(usize, usize, mango::matching::Side, f64, f64)>,
    ) {

        if perp_orders.len() > 0 {

            let mut bidders = Vec::new();
            let mut askers = Vec::new();
            let (_, market_index, _, _, _) = perp_orders[0];
            let perp_market_cookie = self.perp_markets[market_index];

            for perp_order in perp_orders {
                let (user_index, _, order_side, _, _) = *perp_order;
                if order_side == mango::matching::Side::Bid {
                    bidders.push(user_index);
                } else {
                    askers.push(user_index);
                }
            }

            for user_a_index in &bidders {
                for user_b_index in &askers {
                    test.settle_perp_funds(self, &perp_market_cookie, *user_a_index, *user_b_index).await;
                    self.run_keeper(test).await;
                }
            }
        }

    }
}


pub struct MangoAccountCookie {

    pub address: Pubkey,

    pub mango_account: MangoAccount,

}

impl MangoAccountCookie {
    // TODO: Maybe move deposit and withdraw here
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
                &mango_group_cookie.address,
                &mango_account_pk,
                &user_pk,
            ).unwrap()
        ];
        test.process_transaction(&instructions, Some(&[&user])).await.unwrap();
        let mango_account = test.load_account::<MangoAccount>(mango_account_pk).await;
        MangoAccountCookie { address: mango_account_pk, mango_account: mango_account }

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

    #[allow(dead_code)]
    pub async fn init(
        test: &mut MangoProgramTest,
        mango_group_cookie: &mut MangoGroupCookie,
        mint_index: usize,
    ) -> Self {

        let mango_program_id = test.mango_program_id;
        let serum_program_id = test.serum_program_id;

        let mango_group_pk = mango_group_cookie.address;

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
        let liquidation_fee = I80F48::from_num(0.025);
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
                liquidation_fee,
                optimal_util,
                optimal_rate,
                max_rate,
            ).unwrap(),
        ];

        test.process_transaction(&instructions, None).await.unwrap();
        spot_market_cookie.mint = test.with_mint(mint_index);
        spot_market_cookie

    }

    #[allow(dead_code)]
    pub async fn place_order(
        &mut self,
        test: &mut MangoProgramTest,
        mango_group_cookie: &mut MangoGroupCookie,
        user_index: usize,
        side: serum_dex::matching::Side,
        size: f64,
        price: f64,
    ) {

        let limit_price = test.price_number_to_lots(&self.mint, price);
        let max_coin_qty = test.base_size_number_to_lots(&self.mint, size);
        let max_native_pc_qty_including_fees = match side {
            serum_dex::matching::Side::Bid => self.mint.quote_lot as u64 * limit_price * max_coin_qty,
            serum_dex::matching::Side::Ask => std::u64::MAX
        };

        let order = serum_dex::instruction::NewOrderInstructionV3 {
            side: side, //serum_dex::matching::Side::Bid,
            limit_price: NonZeroU64::new(limit_price).unwrap(),
            max_coin_qty: NonZeroU64::new(max_coin_qty).unwrap(),
            max_native_pc_qty_including_fees: NonZeroU64::new(max_native_pc_qty_including_fees).unwrap(),
            self_trade_behavior: serum_dex::instruction::SelfTradeBehavior::DecrementTake,
            order_type: serum_dex::matching::OrderType::Limit,
            client_order_id: mango_group_cookie.current_spot_order_id,
            limit: u16::MAX,
        };

        test.place_spot_order(
            &mango_group_cookie,
            self,
            user_index,
            order,
        ).await;

        mango_group_cookie.mango_accounts[user_index].mango_account =
            test.load_account::<MangoAccount>(mango_group_cookie.mango_accounts[user_index].address).await;
        mango_group_cookie.current_spot_order_id += 1;

    }

}

#[derive(Copy, Clone)]
pub struct PerpMarketCookie {

    pub address: Pubkey,

    pub perp_market: PerpMarket,

    pub mint: MintCookie,

}

impl PerpMarketCookie {

    #[allow(dead_code)]
    pub async fn init(
        test: &mut MangoProgramTest,
        mango_group_cookie: &mut MangoGroupCookie,
        mint_index: usize,
    ) -> Self {

        let mango_program_id = test.mango_program_id;
        let mango_group_pk = mango_group_cookie.address;
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
            address: perp_market_pk,
            perp_market: perp_market,
            mint: test.with_mint(mint_index),
        }

    }

    #[allow(dead_code)]
    pub async fn place_order(
        &mut self,
        test: &mut MangoProgramTest,
        mango_group_cookie: &mut MangoGroupCookie,
        user_index: usize,
        side: mango::matching::Side,
        size: f64,
        price: f64,
    ) {

        let order_size = test.base_size_number_to_lots(&self.mint, size);
        let order_price = test.price_number_to_lots(&self.mint, price);

        test.place_perp_order(
            &mango_group_cookie,
            self,
            user_index,
            side,
            order_size,
            order_price,
            mango_group_cookie.current_perp_order_id,
            mango::matching::OrderType::Limit,
        ).await;

        mango_group_cookie.mango_accounts[user_index].mango_account =
            test.load_account::<MangoAccount>(mango_group_cookie.mango_accounts[user_index].address).await;
        mango_group_cookie.current_perp_order_id += 1;

    }

}
