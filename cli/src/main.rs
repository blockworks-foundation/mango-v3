use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use fixed::types::I80F48;
use mango::state::*;
use mango_common::*;
use serum_dex::state::OpenOrders;
use solana_sdk::pubkey::Pubkey;
use std::mem::size_of;
use std::str::FromStr;
use std::sync::Arc;

#[derive(Parser, Debug, Clone)]
#[clap()]
struct Cli {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Args, Debug, Clone)]
struct EquityFromSnapshotArgs {
    sqlite: String,
    program: Pubkey,
    group: Pubkey,
}

#[derive(Subcommand, Debug, Clone)]
enum Command {
    EquityFromSnapshot(EquityFromSnapshotArgs),
}

fn main() -> anyhow::Result<()> {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    dotenv::dotenv().ok();
    let cli = Cli::parse();

    match cli.command {
        Command::EquityFromSnapshot(args) => EquityFromSnapshot::run(args),
    }
}

fn mango_account_common_checks<T: Sized>(bytes: &[u8], data_type: DataType) -> anyhow::Result<()> {
    if bytes.len() != size_of::<T>() {
        anyhow::bail!("bad size: {}, expected {}", bytes.len(), size_of::<T>());
    }
    if bytes[2] != 1 {
        anyhow::bail!("not initialized: {}", bytes[2]);
    }
    let data_type = data_type as u8;
    if bytes[0] != data_type {
        anyhow::bail!("bad data type: {}, expected {}", bytes[0], data_type);
    }

    Ok(())
}

struct DataSource {
    conn: rusqlite::Connection,
}

impl DataSource {
    fn new(path: String) -> anyhow::Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        Ok(Self { conn })
    }

    fn account_bytes(&self, address: Pubkey) -> anyhow::Result<Vec<u8>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT data FROM account WHERE pubkey = ? ORDER BY write_version DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![address.as_ref()], |row| row.get(0))?;
        if let Some(data) = rows.next() {
            return Ok(data?);
        }
        anyhow::bail!("no data found for pubkey {}", address);
    }

    fn program_account_list(&self, program: Pubkey) -> anyhow::Result<Vec<Pubkey>> {
        let mut stmt =
            self.conn.prepare_cached("SELECT DISTINCT pubkey FROM account WHERE owner = ?")?;
        let mut rows = stmt.query(rusqlite::params![program.as_ref()])?;
        let mut list = Vec::new();
        while let Some(row) = rows.next()? {
            let v: Vec<u8> = row.get(0)?;
            list.push(Pubkey::new(&v));
        }
        Ok(list)
    }

    fn mango_account_list(
        &self,
        program: Pubkey,
        data_type: DataType,
    ) -> anyhow::Result<Vec<Pubkey>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT pubkey FROM account WHERE owner = ? AND hex(substr(data, 1, 1)) = ?",
        )?;
        let data_type_hex = format!("{:#04X}", data_type as u8)[2..].to_string();
        let mut rows = stmt.query(rusqlite::params![program.as_ref(), &data_type_hex])?;
        let mut list = Vec::new();
        while let Some(row) = rows.next()? {
            let v: Vec<u8> = row.get(0)?;
            list.push(Pubkey::new(&v));
        }
        Ok(list)
    }

    fn load_group(&self, address: Pubkey) -> anyhow::Result<MangoGroup> {
        let bytes = self.account_bytes(address)?;
        mango_account_common_checks::<MangoGroup>(&bytes, DataType::MangoGroup)
            .context("loading group")?;
        Ok(MangoGroup::load_from_bytes(&bytes)?.clone())
    }

    fn load_cache(&self, address: Pubkey) -> anyhow::Result<MangoCache> {
        let bytes = self.account_bytes(address)?;
        mango_account_common_checks::<MangoCache>(&bytes, DataType::MangoCache)
            .context("loading cache")?;
        Ok(MangoCache::load_from_bytes(&bytes)?.clone())
    }

    fn load_open_orders(&self, address: Pubkey) -> anyhow::Result<OpenOrders> {
        let bytes = self.account_bytes(address)?;
        if bytes.len() != size_of::<OpenOrders>() + 12 {
            anyhow::bail!("bad open orders size");
        }
        let oo: &OpenOrders = bytemuck::from_bytes(&bytes[5..5 + size_of::<OpenOrders>()]);
        Ok(oo.clone())
    }
}

struct EquityFromSnapshot {
    args: EquityFromSnapshotArgs,
    data: DataSource,
    group: MangoGroup,
    cache: MangoCache,
}

#[derive(Default, Clone)]
struct AccountEquity {
    /// value of per-token equity in usd, ordered by mango group token index
    tokens_usd: [i64; 16],
}

impl EquityFromSnapshot {
    fn run(args: EquityFromSnapshotArgs) -> anyhow::Result<()> {
        let data = DataSource::new(args.sqlite.clone())?;

        let group = data.load_group(args.group)?;
        let cache = data.load_cache(group.mango_cache)?;

        let ctx = EquityFromSnapshot { args, data, group, cache };

        let account_addresses =
            ctx.data.mango_account_list(ctx.args.program, DataType::MangoAccount)?;

        let mut account_equities: Vec<(Pubkey, AccountEquity)> =
            Vec::with_capacity(account_addresses.len());
        for account_address in account_addresses {
            let equity_opt = ctx
                .account_equity(account_address)
                .context(format!("on account {}", account_address))?;
            if equity_opt.is_none() {
                continue;
            }
            account_equities.push((account_address, equity_opt.unwrap()));
        }

        let available_tokens: [bool; 15] = [
            true, true, true, true, false, // usdt is gone
            true, true, false, // cope delisted
            true, false, // no spot ada
            true, true, true, false, // luna delisted
            true,
        ];

        //println!("{:?}", ctx.cache.price_cache.iter().map(|c| c.price).collect::<Vec<_>>());

        // test values only!
        // TODO: pass in the actual prices and amounts!
        let available_native_amounts: [u64; 15] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

        let reimbursement_prices: [I80F48; 16] = [
            // TODO: bad prices
            I80F48::from_num(0.038725),
            I80F48::from_num(19036.47),
            I80F48::from_num(1280.639999999999997),
            I80F48::from_num(0.031244633849997),
            I80F48::from_num(0.999905),
            I80F48::from_num(0.74051845),
            I80F48::from_num(0.511599999999998),
            I80F48::from_num(0.051956999999998),
            I80F48::from_num(23.248483429999997),
            I80F48::from_num(0.393549989999997),
            I80F48::from_num(0.033400008119997),
            I80F48::from_num(2.7067999025),
            I80F48::from_num(0.159774020999997),
            I80F48::from_num(0.000156989999997),
            I80F48::from_num(0.000636922499996),
            I80F48::ONE,
        ];

        let available_amounts: [u64; 15] = available_native_amounts
            .iter()
            .zip(reimbursement_prices.iter())
            .map(|(&native, &price)| (I80F48::from(native) * price).to_num())
            .collect::<Vec<u64>>()
            .try_into()
            .unwrap();

        let mut reimburse_amounts = account_equities.clone();

        // all the equity in unavailable tokens is just considered usdc
        for (_, equity) in reimburse_amounts.iter_mut() {
            for i in 0..15 {
                if !available_tokens[i] {
                    let amount = equity.tokens_usd[i];
                    equity.tokens_usd[QUOTE_INDEX] =
                        equity.tokens_usd[QUOTE_INDEX].checked_add(amount).unwrap();
                    equity.tokens_usd[i] = 0;
                }
            }
        }

        // basic total amount of all positive equities per token (liabs handled later)
        let mut reimburse_totals = [0u64; 16];
        for (_, equity) in account_equities.iter() {
            for (i, value) in equity.tokens_usd.iter().enumerate() {
                if *value >= 0 {
                    reimburse_totals[i] = reimburse_totals[i].checked_add(*value as u64).unwrap();
                }
            }
        }

        println!("sum of positive token equities: {:?}", reimburse_totals);
        println!("sum of available token equities: {:?}", available_amounts);

        // resolve user's liabilities with their assets in a way that aims to bring the
        // needed token amounts <= what's available
        let mut reimburse_amounts = account_equities.clone();
        for (pubkey, equity) in reimburse_amounts.iter_mut() {
            println!("new equity: {:?} {pubkey}", equity.tokens_usd);

            for i in 0..16 {
                let mut value = equity.tokens_usd[i];
                // positive amounts get reimbursed
                if value >= 0 {
                    continue;
                }

                // Negative amounts must be settled against other token balances
                // This is using a greedy strategy, reducing the most requested token first
                let mut weighted_indexes = equity.tokens_usd[0..15]
                    .iter()
                    .enumerate()
                    .filter_map(|(i, v)| (*v > 0).then_some(i))
                    .filter_map(|i| {
                        (available_amounts[i] < reimburse_totals[i])
                            .then(|| (i, reimburse_totals[i] - available_amounts[i]))
                    })
                    .collect::<Vec<(usize, u64)>>();

                weighted_indexes.sort_by(|a, b| a.1.cmp(&b.1));
                for &(j, _) in weighted_indexes.iter() {
                    let start = equity.tokens_usd[j];
                    let amount = if start + value >= 0 { -value } else { start };
                    equity.tokens_usd[j] = equity.tokens_usd[j].checked_sub(amount).unwrap();
                    reimburse_totals[j] = reimburse_totals[j].checked_sub(amount as u64).unwrap();
                    //println!("{value} {start} {amount} {i} {j}");
                    value = value + amount;
                    if value >= 0 {
                        break;
                    }
                }

                // All tokens fine? Try reducing some random one, starting with USDC
                for j in (0..16).rev() {
                    if equity.tokens_usd[j] <= 0 {
                        continue;
                    }
                    let start = equity.tokens_usd[j];
                    let amount = if start + value >= 0 { -value } else { start };
                    equity.tokens_usd[j] = equity.tokens_usd[j].checked_sub(amount).unwrap();
                    reimburse_totals[j] = reimburse_totals[j].checked_sub(amount as u64).unwrap();
                    value = value + amount;
                    if value >= 0 {
                        break;
                    }
                }

                assert!(value == 0);
                equity.tokens_usd[i] = 0;
            }
            println!("---- final: {:?}", equity.tokens_usd);
        }

        // now all reimburse_amounts are >= 0

        // Do a second pass where we scale up or down user reimbursement amounts to fully utilize funds
        for i in 0..15 {
            if reimburse_totals[i] == 0 || reimburse_totals[i] == available_amounts[i] {
                continue;
            }
            let fraction = I80F48::from(available_amounts[i]) / I80F48::from(reimburse_totals[i]);

            // Scale down token reimbursements and replace them with USDC reimbursements
            for (_, equity) in reimburse_amounts.iter_mut() {
                let amount = &mut equity.tokens_usd[i];
                assert!(*amount >= 0);
                if *amount == 0 {
                    continue;
                }

                let new_amount: i64 = (I80F48::from(*amount) * fraction).to_num();
                if fraction < 1 {
                    let decrease = (*amount - new_amount) as u64;
                    *amount = new_amount;
                    reimburse_totals[i] = reimburse_totals[i].checked_sub(decrease).unwrap();
                    equity.tokens_usd[QUOTE_INDEX] += decrease as i64;
                    reimburse_totals[QUOTE_INDEX] =
                        reimburse_totals[QUOTE_INDEX].checked_add(decrease).unwrap();
                } else {
                    let increase = (new_amount - *amount) as u64;
                    *amount = new_amount;
                    reimburse_totals[i] = reimburse_totals[i].checked_add(increase).unwrap();
                    equity.tokens_usd[QUOTE_INDEX] -= increase as i64;
                    reimburse_totals[QUOTE_INDEX] =
                        reimburse_totals[QUOTE_INDEX].checked_sub(increase).unwrap();
                }
            }
        }

        println!("sum of positive token equities: {:?}", reimburse_totals);
        for i in 0..15 {
            println!(
                "{i} {} {} {}",
                available_amounts[i] / 1000000,
                reimburse_totals[i] / 1000000,
                (available_amounts[i] as i64 - reimburse_totals[i] as i64) / 1000000
            );
        }
        println!("15 {}", reimburse_totals[15] / 1000000);

        println!("reimburse total {}", reimburse_totals.iter().sum::<u64>() / 1000000);

        //     println!("{},{:?}", account_address, equity.tokens_usd);
        // }

        Ok(())
    }

    fn account_equity(&self, account_address: Pubkey) -> anyhow::Result<Option<AccountEquity>> {
        if account_address
            != Pubkey::from_str(&"3TXDBTHFwyKywjZj1vGdRVkrF5o4YZ1vZnMf3Hb9qALz").unwrap()
        {
            //return Ok(None);
        }
        let account_bytes = self.data.account_bytes(account_address)?;
        mango_account_common_checks::<MangoAccount>(&account_bytes, DataType::MangoAccount)?;
        let mango_account = MangoAccount::load_from_bytes(&account_bytes)?;
        if mango_account.mango_group != self.args.group {
            return Ok(None);
        }

        let mut equity = [I80F48::ZERO; 16];

        // USDC
        {
            let bank_cache = &self.cache.root_bank_cache[QUOTE_INDEX];
            let usdc = mango_account.get_net(bank_cache, QUOTE_INDEX);
            //println!("usdc {}", usdc);
            equity[QUOTE_INDEX] = usdc;
        }

        // Sum up the deposit/borrow equity
        for oracle_index in 0..self.group.num_oracles {
            if self.group.spot_markets[oracle_index].is_empty() {
                continue;
            }
            let price = self.cache.price_cache[oracle_index].price;
            let bank_cache = &self.cache.root_bank_cache[oracle_index];
            let net = mango_account.get_net(bank_cache, oracle_index);
            let net_usd = net.checked_mul(price).unwrap();
            //println!("token {} {} {} {}", oracle_index, net, net_usd, price);
            equity[oracle_index] = net_usd;
        }

        // Sum up the serum open orders equity
        for oracle_index in 0..self.group.num_oracles {
            if self.group.spot_markets[oracle_index].is_empty() {
                continue;
            }
            let oo_address = mango_account.spot_open_orders[oracle_index];
            if oo_address == Pubkey::default() {
                continue;
            }
            let price = self.cache.price_cache[oracle_index].price;
            let oo_maybe = self.data.load_open_orders(oo_address);
            if oo_maybe.is_err() {
                println!(
                    "Error: can't find oo account {} for mango account {}",
                    oo_address, account_address
                );
                continue;
            }
            let oo = oo_maybe.unwrap();
            let quote = oo.native_pc_total + oo.referrer_rebates_accrued;
            let base = I80F48::from(oo.native_coin_total);
            let base_usd = base.checked_mul(price).unwrap();
            let serum_equity = I80F48::from(quote).checked_add(base_usd).unwrap();
            if !mango_account.in_margin_basket[oracle_index] && serum_equity != 0 {
                println!("Error: mango account {} lists oo account {} with equity {} but in_margin_basket is false", account_address, oo_address, serum_equity);
            }
            //println!("serum {} {} {}", quote, base, serum_equity);
            equity[QUOTE_INDEX] = equity[QUOTE_INDEX].checked_add(I80F48::from(quote)).unwrap();
            equity[oracle_index] = equity[oracle_index].checked_add(base_usd).unwrap();
        }

        // Sum up the perp position equity
        for oracle_index in 0..self.group.num_oracles {
            if self.group.perp_markets[oracle_index].is_empty()
                || !mango_account.perp_accounts[oracle_index].is_active()
            {
                continue;
            }
            let price = self.cache.price_cache[oracle_index].price;
            let pmi = &self.group.perp_markets[oracle_index];
            let pmc = &self.cache.perp_market_cache[oracle_index];
            let pa = &mango_account.perp_accounts[oracle_index];
            let quote =
                pa.get_quote_position(pmc) + I80F48::from_num(pa.taker_quote * pmi.quote_lot_size);
            let base = I80F48::from(pa.base_position + pa.taker_base)
                .checked_mul(I80F48::from(pmi.base_lot_size))
                .unwrap();
            let perp_equity = quote.checked_add(base.checked_mul(price).unwrap()).unwrap();
            //println!("perp {} {} {} {}", oracle_index, quote, base, perp_equity);
            equity[QUOTE_INDEX] = equity[QUOTE_INDEX].checked_add(perp_equity).unwrap();
        }
        // ignore open perp orders

        let mut account_equity = AccountEquity::default();
        for i in 0..16 {
            account_equity.tokens_usd[i] = equity[i].round().to_num();
        }

        Ok(Some(account_equity))
    }
}
