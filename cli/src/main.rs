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

impl EquityFromSnapshot {
    fn run(args: EquityFromSnapshotArgs) -> anyhow::Result<()> {
        let data = DataSource::new(args.sqlite.clone())?;

        let group = data.load_group(args.group)?;
        let cache = data.load_cache(group.mango_cache)?;

        let ctx = EquityFromSnapshot { args, data, group, cache };

        let account_addresses =
            ctx.data.mango_account_list(ctx.args.program, DataType::MangoAccount)?;

        for account_address in account_addresses {
            let equity_opt = ctx
                .account_equity(account_address)
                .context(format!("on account {}", account_address))?;
            if let Some(equity) = equity_opt {
                println!("{},{}", account_address, equity);
            }
        }

        Ok(())
    }

    fn account_equity(&self, account_address: Pubkey) -> anyhow::Result<Option<I80F48>> {
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

        let mut equity = I80F48::ZERO;

        // USDC
        {
            let bank_cache = &self.cache.root_bank_cache[QUOTE_INDEX];
            let usdc = mango_account.get_net(bank_cache, QUOTE_INDEX);
            //println!("usdc {}", usdc);
            equity = equity.checked_add(usdc).unwrap();
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
            equity = equity.checked_add(net_usd).unwrap();
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
            let quote = I80F48::from(oo.native_pc_total + oo.referrer_rebates_accrued);
            let base = I80F48::from(oo.native_coin_total);
            let serum_equity = quote.checked_add(base.checked_mul(price).unwrap()).unwrap();
            if !mango_account.in_margin_basket[oracle_index] && serum_equity != 0 {
                println!("Error: mango account {} lists oo account {} with equity {} but in_margin_basket is false", account_address, oo_address, serum_equity);
            }
            //println!("serum {} {} {}", quote, base, serum_equity);
            equity = equity.checked_add(serum_equity).unwrap();
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
            equity = equity.checked_add(perp_equity).unwrap();
        }
        // ignore open perp orders

        Ok(Some(equity))
    }
}
