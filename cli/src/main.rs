use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use fixed::types::I80F48;
use mango::state::*;
use mango_common::*;
use serum_dex::state::OpenOrders;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::mem::size_of;
use std::str::FromStr;

#[derive(Parser, Debug, Clone)]
#[clap()]
struct Cli {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Args, Debug, Clone)]
struct EquityFromSnapshotArgs {
    sqlite: String,
    late_changes: String,
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

fn late_deposits_withdrawals(filename: &str) -> anyhow::Result<Vec<(Pubkey, Pubkey, usize, i64)>> {
    // mango token index and decimals
    let tokens: HashMap<&str, (usize, i32)> = HashMap::from([
        ("MNGO", (0, 6)),
        ("BTC", (1, 6)),
        ("ETH", (2, 6)),
        ("SOL", (3, 9)),
        ("USDT", (4, 6)),
        ("SRM", (5, 6)),
        ("RAY", (6, 6)),
        ("FTT", (8, 6)),
        ("MSOL", (10, 9)),
        ("BNB", (11, 8)),
        ("AVAX", (12, 8)),
        ("GMT", (14, 9)),
        ("USDC", (15, 6)),
    ]);

    let mut list = Vec::new();

    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(filename)?;
    for line in BufReader::new(file).lines().skip(1) {
        if let Ok(line) = line {
            let fields = line.split("\t").collect::<Vec<&str>>();
            assert_eq!(fields.len(), 19);
            let account = Pubkey::from_str(fields[5]).unwrap();
            // skip attacker accounts
            if fields[5] == "4ND8FVPjUGGjx9VuGFuJefDWpg3THb58c277hbVRnjNa"
                || fields[5] == "CQvKSNnYtPTZfQRQ5jkHq8q2swJyRsdQLcFcj3EmKFfX"
            {
                continue;
            }
            let owner = Pubkey::from_str(fields[6]).unwrap();
            let token = fields[7];
            let side = fields[8];
            let quantity = f64::from_str(&fields[9].replace(",", "")).unwrap();
            let token_info = tokens.get(token).unwrap();
            let change = (quantity
                * 10f64.powi(token_info.1)
                * (if side == "Withdraw" {
                    -1f64
                } else {
                    assert_eq!(side, "Deposit");
                    1f64
                })) as i64;
            list.push((account, owner, token_info.0, change));
        }
    }
    Ok(list)
}

struct EquityFromSnapshot {
    args: EquityFromSnapshotArgs,
    data: DataSource,
    group: MangoGroup,
    cache: MangoCache,
}

fn cache_price(cache: &MangoCache, index: usize) -> I80F48 {
    if index == QUOTE_INDEX {
        I80F48::ONE
    } else {
        cache.price_cache[index].price
    }
}

/// value of per-token equity in usd, ordered by mango group token index
type AccountTokenAmounts = [i64; 16];

impl EquityFromSnapshot {
    fn run(args: EquityFromSnapshotArgs) -> anyhow::Result<()> {
        let late_changes = late_deposits_withdrawals(&args.late_changes)?;

        let data = DataSource::new(args.sqlite.clone())?;

        let group = data.load_group(args.group)?;
        let cache = data.load_cache(group.mango_cache)?;

        let ctx = EquityFromSnapshot { args, data, group, cache };

        let account_addresses =
            ctx.data.mango_account_list(ctx.args.program, DataType::MangoAccount)?;

        let mut account_equities: Vec<(Pubkey, Pubkey, AccountTokenAmounts)> =
            Vec::with_capacity(account_addresses.len());

        // get the snapshot account equities
        for account_address in account_addresses {
            let equity_opt = ctx
                .account_equity(account_address)
                .context(format!("on account {}", account_address))?;
            if equity_opt.is_none() {
                continue;
            }
            let (owner, equity) = equity_opt.unwrap();
            account_equities.push((account_address, owner, equity));
        }

        // apply the late deposits/withdrawals
        for &(address, owner, token_index, change_native) in late_changes.iter() {
            let change_usd =
                (I80F48::from(change_native) * cache_price(&cache, token_index)).to_num();
            // slow, but just ran a handful times
            let account_opt = account_equities.iter_mut().find(|(a, _, _)| a == &address);
            if let Some((_, _, equity)) = account_opt {
                equity[token_index] += change_usd;
            } else {
                assert!(change_usd > 0);
                let mut equity = AccountTokenAmounts::default();
                equity[token_index] = change_usd;
                account_equities.push((address, owner, equity));
            }
        }

        // Some accounts already cached out on a MNGO PERP position that started to be valuable after the
        // snapshot was taken, no reimbursements
        {
            let odd_accounts = [
                "9A6YVfa66kBEeCLtt6wyqdmjpib7UrybA5mHr3X3kyvf",
                "AEYWfmFVu1huajTkT3UUbvhCZx92kZXwgpWgrMtocnzv",
                "AZVbGBJ1DU2RnZNhZ72fcpo191DX3k1uyqDiPiaWoF1q",
                "C19JAyRLUjkTWmj9VpYu5eVVCbSXcbqqhyF5588ERSSf",
                "C9rprN4zcP7Wx87UcbLarTEAGCmGiPZp8gaFXPhY9HYm",
            ];
            for odd_one in odd_accounts {
                let address = Pubkey::from_str(odd_one).unwrap();
                let (_, _, equity) =
                    account_equities.iter_mut().find(|(a, _, _)| a == &address).unwrap();
                assert!(late_changes.iter().any(|(a, _, _, c)| a == &address && *c < 0));
                let total = equity.iter().sum::<i64>();
                assert!(total < 0);
                assert!(total > -10_000_000_000); // none of these was bigger than 10000 USD
                *equity = AccountTokenAmounts::default();
            }
        }

        // Some accounts withdrew everything after the snapshot was taken. When doing that they
        // probably withdrew a tiny bit more than their snapshot equity due to interest.
        // These accounts have already cached out, no need to reimburse.
        for (address, _, equity) in account_equities.iter_mut() {
            let total = equity.iter().sum::<i64>();
            if total >= 0 {
                continue;
            }
            assert!(late_changes.iter().any(|(a, _, _, c)| a == address && *c < 0));
            assert!(equity.iter().sum::<i64>() < 0);
            // only up to -10 USD is expected, otherwise investigate manually!
            assert!(equity.iter().sum::<i64>() > -10_000_000);
            *equity = AccountTokenAmounts::default();
        }

        let token_names: [&str; 16] = [
            "MNGO", "BTC", "ETH", "SOL", "USDT", "SRM", "RAY", "COPE", "FTT", "ADA", "MSOL", "BNB",
            "AVAX", "LUNA", "GMT", "USDC",
        ];

        let available_tokens: [bool; 15] = [
            true, true, true, true, false, // usdt is gone
            true, true, false, // cope delisted
            true, false, // no spot ada
            true, true, true, false, // luna delisted
            true,
        ];

        // TODO: tentative numbers from "Repay bad Debt #2" proposal
        let available_native_amounts: [u64; 15] = [
            32409565000000,
            281498000,
            226000000,
            761577910000000,
            0,
            2354260000000,
            98295000000,
            0,
            11774000000,
            0,
            799155000000000,
            60800000000,
            180900000000,
            0,
            152843000000000,
        ];

        // Token prices at time of reimbursement
        // Note that user equity at snapshot time is computed from the prices from the
        // mango cache in the snapshot.
        let reimbursement_prices: [I80F48; 16] = [
            // TODO: bad prices, must be updated when time comes!
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

        // USD amounts in each token that can be used for reimbursement
        let available_amounts: [u64; 15] = available_native_amounts
            .iter()
            .zip(reimbursement_prices.iter())
            .map(|(&native, &price)| (I80F48::from(native) * price).to_num())
            .collect::<Vec<u64>>()
            .try_into()
            .unwrap();

        // Amounts each user should be reimbursed
        let mut reimburse_amounts = account_equities.clone();

        // all the equity in unavailable tokens is just considered usdc
        for (_, _, equity) in reimburse_amounts.iter_mut() {
            for i in 0..15 {
                if !available_tokens[i] {
                    let amount = equity[i];
                    equity[QUOTE_INDEX] += amount;
                    equity[i] = 0;
                }
            }
        }

        // basic total amount of all positive equities per token (liabs handled later)
        let mut reimburse_totals = [0u64; 16];
        for (_, _, equity) in account_equities.iter() {
            for (i, value) in equity.iter().enumerate() {
                if *value >= 0 {
                    reimburse_totals[i] += *value as u64;
                }
            }
        }

        println!("sum of positive token equities: {:?}", reimburse_totals);
        println!("sum of available token equities: {:?}", available_amounts);

        // resolve user's liabilities with their assets in a way that aims to bring the
        // needed token amounts <= what's available
        let mut reimburse_amounts = account_equities.clone();
        for (_, _, equity) in reimburse_amounts.iter_mut() {
            for i in 0..16 {
                let mut value = equity[i];
                // positive amounts get reimbursed
                if value >= 0 {
                    continue;
                }

                // Negative amounts must be settled against other token balances
                // This is using a greedy strategy, reducing the most requested token first
                let mut weighted_indexes = equity[0..15]
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
                    let start = equity[j];
                    let amount = if start + value >= 0 { -value } else { start };
                    equity[j] -= amount;
                    reimburse_totals[j] -= amount as u64;
                    value += amount;
                    if value >= 0 {
                        break;
                    }
                }

                // All tokens fine? Try reducing some random one, starting with USDC
                // (mSOL is last because it looks like we will have a lot of it and want
                // to prefer giving it out to users that had it before)
                for j in [15, 14, 13, 12, 11, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 10] {
                    if equity[j] <= 0 {
                        continue;
                    }
                    let start = equity[j];
                    let amount = if start + value >= 0 { -value } else { start };
                    equity[j] -= amount;
                    reimburse_totals[j] -= amount as u64;
                    value += amount;
                    if value >= 0 {
                        break;
                    }
                }

                assert!(value == 0);
                equity[i] = 0;
            }
        }

        // now all reimburse_amounts are >= 0

        // Do a pass where we scale down user reimbursement token amounts and instead
        // reimburse with USDC if there's not enough tokens to give out
        for i in 0..15 {
            if reimburse_totals[i] == 0 || reimburse_totals[i] == available_amounts[i] {
                continue;
            }
            let fraction = I80F48::from(available_amounts[i]) / I80F48::from(reimburse_totals[i]);
            if fraction >= 1 {
                continue;
            }

            // Scale down token reimbursements and replace them with USDC reimbursements
            for (_, _, equity) in reimburse_amounts.iter_mut() {
                let amount = &mut equity[i];
                assert!(*amount >= 0);
                if *amount == 0 {
                    continue;
                }

                let new_amount: i64 = (I80F48::from(*amount) * fraction).to_num();
                let decrease = (*amount - new_amount) as u64;
                *amount = new_amount;
                reimburse_totals[i] -= decrease;
                let target = if i == 3 {
                    10 // SOL -> mSOL
                } else {
                    QUOTE_INDEX
                };
                equity[target] += decrease as i64;
                reimburse_totals[target] += decrease;
            }
        }

        // Do passes where we scale up token reimbursement amounts to try to fully utilize funds
        //
        // The idea here is that we have say 1000 SOL but only need 500 SOL to reimburse.
        // To leave the DAO with fewer SOL at the end we prefer to give people who already
        // had some SOL more of it (and compensate by giving them less of another token).
        for _ in 0..100 {
            for i in 0..15 {
                if reimburse_totals[i] == 0 || reimburse_totals[i] == available_amounts[i] {
                    continue;
                }

                let fraction =
                    I80F48::from(available_amounts[i]) / I80F48::from(reimburse_totals[i]);
                if fraction <= 1 {
                    continue;
                }

                // Scale up token reimbursements and take away USDC reimbursements
                for (_, _, equity) in reimburse_amounts.iter_mut() {
                    let amount = equity[i];
                    assert!(amount >= 0);
                    if amount == 0 {
                        continue;
                    }

                    let new_amount: i64 = (I80F48::from(amount) * fraction).to_num();
                    let mut remaining_increase = new_amount - amount; // positive

                    for j in (0..16).rev() {
                        let other_amount = equity[j];
                        if (j != 15 && available_amounts[j] >= reimburse_totals[j])
                            || other_amount == 0
                        {
                            continue;
                        }
                        let increase = remaining_increase.min(other_amount);
                        equity[j] -= increase;
                        reimburse_totals[j] -= increase as u64;
                        equity[i] += increase;
                        reimburse_totals[i] += increase as u64;
                        remaining_increase -= increase;
                    }
                }
            }
        }

        // Double check that total user equity is unchanged
        for ((_, ownerl, equity), (_, ownerr, reimburse)) in
            account_equities.iter().zip(reimburse_amounts.iter())
        {
            let eqsum = equity.iter().sum::<i64>();
            let resum = reimburse.iter().sum::<i64>();
            assert_eq!(eqsum, resum);
            assert_eq!(ownerl, ownerr);
        }

        for i in 0..15 {
            println!(
                "{}: available {}, used {}, left over {}",
                token_names[i],
                available_amounts[i] / 1000000,
                reimburse_totals[i] / 1000000,
                (available_amounts[i] as i64 - reimburse_totals[i] as i64) / 1000000
            );
        }
        println!("USDC: used {}", reimburse_totals[15] / 1000000);
        println!("reimburse total {}", reimburse_totals.iter().sum::<u64>() / 1000000);

        println!("account,owner,{}", token_names.join(","));
        for (account, owner, amounts) in reimburse_amounts.iter() {
            println!(
                "{account},{owner},{}",
                amounts
                    .iter()
                    .enumerate()
                    .map(|(index, v)| (I80F48::from(*v) / reimbursement_prices[index])
                        .floor()
                        .to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }

        Ok(())
    }

    fn account_equity(
        &self,
        account_address: Pubkey,
    ) -> anyhow::Result<Option<(Pubkey, AccountTokenAmounts)>> {
        if account_address
            != Pubkey::from_str(&"rwxRFn2S1DHkbA8wiCDxzMRMncgjUaa4LiJTagvLBr9").unwrap()
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

        let mut account_equity = AccountTokenAmounts::default();
        for i in 0..16 {
            account_equity[i] = equity[i].round().to_num();
        }

        Ok(Some((mango_account.owner, account_equity)))
    }
}
