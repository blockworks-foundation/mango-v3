use anyhow::Context;
use clap::{Args, Parser, Subcommand, ValueEnum};
use fixed::types::I80F48;
use mango::state::*;
use mango_common::*;
use serum_dex::state::OpenOrders;
use solana_sdk::pubkey::Pubkey;
use std::fs::File;
use std::io::Write;
use std::mem::size_of;
use std::str::FromStr;

#[derive(Parser, Debug, Clone)]
#[clap()]
struct Cli {
    #[clap(subcommand)]
    command: Command,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum OutType {
    Csv,
    Binary,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum DistributionMode {
    UsdcOnly,
    Tokens,
}

#[derive(Args, Debug, Clone)]
struct EquityFromSnapshotArgs {
    #[arg(long)]
    snapshot: String,
    #[arg(long)]
    late_changes: String,
    #[arg(long)]
    program: Pubkey,
    #[arg(long)]
    group: Pubkey,

    #[arg(long)]
    outtype: OutType,
    #[arg(long)]
    outfile: String,

    #[arg(long, default_value = "tokens")]
    distribution_mode: DistributionMode,

    #[arg(long, default_value = "false")]
    enable_scale_down: bool,

    #[arg(long, default_value = "false")]
    enable_greedy: bool,
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

struct TokenInfo {
    name: String,
    index: usize,
    decimals: i32,
    available_native: u64,
    reimbursement_price: I80F48,
}

impl TokenInfo {
    fn is_active(&self) -> bool {
        self.decimals > 0
    }
}

struct Constants {
    token_infos: Vec<TokenInfo>,
}

impl Constants {
    fn new(mode: DistributionMode) -> Self {
        let mut out = Self {
            // Reimbursement prices: Coingecko market close prices of 2022-10-16
            // (Note that user equity at snapshot time is computed from the prices from the
            //  mango cache in the snapshot, not the reimbursement_price)

            // The available_native amounts are the sum of tokens recovered from the attacker
            // (left) and tokens recovered from serum3 open orders (right).
            token_infos: vec![
                TokenInfo {
                    name: "MNGO".into(),
                    index: 0,
                    decimals: 6,
                    available_native: 32904328899472 + 17926416000000,
                    reimbursement_price: I80F48::from_num(0.02425361),
                },
                TokenInfo {
                    name: "BTC".into(),
                    index: 1,
                    decimals: 6,
                    available_native: 281498500 + 15849599,
                    reimbursement_price: I80F48::from_num(19272.92),
                },
                TokenInfo {
                    name: "ETH".into(),
                    index: 2,
                    decimals: 6,
                    available_native: 226000000 + 7431000,
                    reimbursement_price: I80F48::from_num(1306.94),
                },
                TokenInfo {
                    name: "SOL".into(),
                    index: 3,
                    decimals: 9,
                    available_native: 761577000000000 + 4778699999999,
                    reimbursement_price: I80F48::from_num(0.03017),
                },
                TokenInfo {
                    name: "USDT".into(),
                    index: 4,
                    decimals: 6,
                    available_native: 0 + 14646000000,
                    reimbursement_price: I80F48::from_num(1.000),
                },
                TokenInfo {
                    name: "SRM".into(),
                    index: 5,
                    decimals: 6,
                    available_native: 2354260000000 + 10258200000,
                    reimbursement_price: I80F48::from_num(0.720998),
                },
                TokenInfo {
                    name: "RAY".into(),
                    index: 6,
                    decimals: 6,
                    available_native: 98295000000 + 10605100000,
                    reimbursement_price: I80F48::from_num(0.491728),
                },
                TokenInfo {
                    name: "COPE".into(),
                    index: 7,
                    decimals: i32::MIN,
                    available_native: 0,
                    reimbursement_price: I80F48::MIN,
                },
                TokenInfo {
                    name: "FTT".into(),
                    index: 8,
                    decimals: 6,
                    available_native: 11774000000 + 214800000,
                    reimbursement_price: I80F48::from_num(23.73),
                },
                TokenInfo {
                    name: "ADA".into(),
                    index: 9,
                    decimals: i32::MIN,
                    available_native: 0,
                    reimbursement_price: I80F48::MIN,
                },
                TokenInfo {
                    name: "MSOL".into(),
                    index: 10,
                    decimals: 9,
                    available_native: 799155000000000 + 179378000000,
                    reimbursement_price: I80F48::from_num(0.03227),
                },
                TokenInfo {
                    name: "BNB".into(),
                    index: 11,
                    decimals: 8,
                    available_native: 60800000000 + 151100000,
                    reimbursement_price: I80F48::from_num(2.7236),
                },
                TokenInfo {
                    name: "AVAX".into(),
                    index: 12,
                    decimals: 8,
                    available_native: 180900000000 + 10225000000,
                    reimbursement_price: I80F48::from_num(0.1576),
                },
                TokenInfo {
                    name: "LUNA".into(),
                    index: 13,
                    decimals: i32::MIN,
                    available_native: 0,
                    reimbursement_price: I80F48::MIN,
                },
                TokenInfo {
                    name: "GMT".into(),
                    index: 14,
                    decimals: 9,
                    available_native: 152843000000000 + 0,
                    reimbursement_price: I80F48::from_num(0.000578548),
                },
                TokenInfo {
                    name: "USDC".into(),
                    index: 15,
                    decimals: 6,
                    available_native: u64::MAX,
                    reimbursement_price: I80F48::ONE,
                },
            ],
        };
        assert!(out.token_infos.iter().map(|ti| ti.index).eq(0..16));

        if mode == DistributionMode::UsdcOnly {
            for ti in out.token_infos[0..15].iter_mut() {
                ti.available_native = 0;
            }
        }

        out
    }

    fn token_info_by_name(&self, name: &str) -> Option<&TokenInfo> {
        self.token_infos.iter().find(|ti| ti.name == name)
    }

    fn token_names(&self) -> Vec<String> {
        self.token_infos.iter().map(|ti| ti.name.clone()).collect()
    }

    fn usd_to_tokens_ui(&self, index: usize, usd: i64) -> I80F48 {
        let ti = &self.token_infos[index];
        if !ti.is_active() {
            assert!(usd == 0);
            return I80F48::ZERO;
        }
        (I80F48::from(usd) / ti.reimbursement_price).floor()
            / I80F48::from(10u64.pow(ti.decimals as u32))
    }
}

fn late_deposits_withdrawals(
    filename: &str,
    constants: &Constants,
) -> anyhow::Result<Vec<(Pubkey, Pubkey, usize, i64)>> {
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
            let token_info = constants.token_info_by_name(&token).unwrap();
            let change = (quantity
                * 10f64.powi(token_info.decimals)
                * (if side == "Withdraw" {
                    -1f64
                } else {
                    assert_eq!(side, "Deposit");
                    1f64
                })) as i64;
            list.push((account, owner, token_info.index, change));
        }
    }
    Ok(list)
}

struct EquityFromSnapshot {
    args: EquityFromSnapshotArgs,
    data: DataSource,
    late_changes: Vec<(Pubkey, Pubkey, usize, i64)>,
    group: MangoGroup,
    cache: MangoCache,
    constants: Constants,
    mngo_perp_price: I80F48,
}

#[derive(bytemuck::Pod, bytemuck::Zeroable, Clone, Copy)]
#[repr(C)]
struct BinaryRow {
    owner: Pubkey,
    amounts: [u64; 16],
}

struct OutWriter {
    file: File,
    outtype: OutType,
}

impl OutWriter {
    fn new(outfile: &str, outtype: OutType) -> Self {
        let file = File::create(outfile).unwrap();
        Self { file, outtype }
    }

    fn write(&mut self, account: &AccountData) {
        match self.outtype {
            OutType::Csv => {
                write!(
                    &mut self.file,
                    "{},{},{}\n",
                    account.mango_account,
                    account.owner,
                    account.amounts.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
                )
                .unwrap();
            }
            OutType::Binary => {
                let row = BinaryRow {
                    owner: account.owner,
                    amounts: account
                        .amounts
                        .iter()
                        .map(|&v| v.try_into().unwrap())
                        .collect::<Vec<u64>>()
                        .try_into()
                        .unwrap(),
                };
                self.file.write_all(bytemuck::bytes_of(&row)).unwrap();
            }
        }
    }

    fn write_header(&mut self, constants: &Constants) {
        match self.outtype {
            OutType::Csv => {
                write!(&mut self.file, "account,owner,{}\n", constants.token_names().join(","))
                    .unwrap();
            }
            OutType::Binary => {
                // buffer accounts have a 37 byte header -- add 3 bytes to 8-byte align the data
                self.file.write_all(&[0u8; 3]).unwrap();
            }
        }
    }
}

/// value of per-token equity in usd, ordered by mango group token index
type AccountTokenAmounts = [i64; 16];

#[derive(Clone, Debug)]
struct AccountData {
    mango_account: Pubkey,
    owner: Pubkey,
    amounts: AccountTokenAmounts,
}

fn pay_liab(
    amounts: &mut AccountTokenAmounts,
    liab: usize,
    asset: usize,
    amount: i64,
    totals: &mut [u64; 16],
) {
    assert!(liab != asset);
    assert!(amount >= 0);
    amounts[asset] -= amount;
    assert!(amounts[asset] >= 0);
    amounts[liab] += amount;
    totals[asset] -= amount as u64;
    // liabs weren't counted in totals!
}

fn move_amount(
    amounts: &mut AccountTokenAmounts,
    from: usize,
    to: usize,
    amount: i64,
    totals: &mut [u64; 16],
) {
    assert!(from != to);
    assert!(amount >= 0);
    amounts[from] -= amount;
    assert!(amounts[from] >= 0);
    amounts[to] += amount;
    totals[from] -= amount as u64;
    totals[to] += amount as u64;
}

impl EquityFromSnapshot {
    fn run(args: EquityFromSnapshotArgs) -> anyhow::Result<()> {
        let constants = Constants::new(args.distribution_mode);
        let late_changes = late_deposits_withdrawals(&args.late_changes, &constants)?;
        let data = DataSource::new(args.snapshot.clone())?;

        let mut outwriter = OutWriter::new(&args.outfile, args.outtype);

        let group = data.load_group(args.group)?;

        let (cache, mngo_perp_price) = {
            let mut cache = data.load_cache(group.mango_cache)?;
            let mngo_cache_price = cache.price_cache[0].price;
            // Fix the MNGO snapshot price to be the same as the reimbursement price.
            // This does two things:
            // - the MNGO-based equity will be converted back to MNGO tokens at the same price,
            //   allowing the token count to stay unchanged
            // - if MNGO tokens must be used as assets, they're valued with the less favorable price
            cache.price_cache[0].price =
                constants.token_info_by_name("MNGO").unwrap().reimbursement_price;
            (cache, mngo_cache_price)
        };

        let token_names = constants.token_names();

        let ctx = EquityFromSnapshot {
            args,
            data,
            late_changes,
            group,
            cache,
            constants,
            mngo_perp_price,
        };

        println!("table,account,owner,{}", token_names.join(","));

        let debug_print = |table: &str, data: &[AccountData]| {
            for account in data.iter() {
                println!(
                    "{table},{},{},{}",
                    account.mango_account,
                    account.owner,
                    account.amounts.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
                );
            }
        };

        let account_equities = {
            let mut equities = ctx.snapshot_account_equities()?;

            debug_print("snapshot", &equities);

            ctx.apply_late_deposits_withdrawals(&mut equities)?;

            debug_print("after dep/with", &equities);

            ctx.skip_negative_equity_accounts(&mut equities)?;

            equities
        };

        // USD amounts in each token that can be used for reimbursement
        let available_amounts: [u64; 15] = ctx.constants.token_infos[0..15]
            .iter()
            .map(|ti| (I80F48::from(ti.available_native) * ti.reimbursement_price).to_num())
            .collect::<Vec<u64>>()
            .try_into()
            .unwrap();

        // Amounts each user should be reimbursed
        let mut reimburse_amounts = account_equities.clone();

        // Verify that equity for inactive tokens is zero
        for account in reimburse_amounts.iter_mut() {
            for ti in ctx.constants.token_infos.iter() {
                if !ti.is_active() {
                    assert_eq!(account.amounts[ti.index], 0);
                }
            }
        }

        // basic total amount of all positive equities per token (liabs handled later)
        let mut reimburse_totals = [0u64; 16];
        for account in reimburse_amounts.iter() {
            for (i, value) in account.amounts.iter().enumerate() {
                if *value >= 0 {
                    reimburse_totals[i] += *value as u64;
                }
            }
        }

        println!("sum of positive token equities: {:?}", reimburse_totals);
        println!("sum of available token equities: {:?}", available_amounts);

        // resolve user's liabilities with their assets in a way that aims to bring the
        // needed token amounts <= what's available
        for AccountData { amounts, .. } in reimburse_amounts.iter_mut() {
            for i in 0..16 {
                let mut value = amounts[i];
                // positive amounts get reimbursed
                if value >= 0 {
                    continue;
                }

                if ctx.args.enable_greedy {
                    // Negative amounts must be settled against other token balances
                    // This is using a greedy strategy, reducing the most requested token first
                    let mut weighted_indexes = amounts[0..15]
                        .iter()
                        .enumerate()
                        .skip(1) // skip MNGO
                        .filter_map(|(i, v)| (*v > 0).then_some(i))
                        .filter_map(|i| {
                            (available_amounts[i] < reimburse_totals[i])
                                .then(|| (i, reimburse_totals[i] - available_amounts[i]))
                        })
                        .collect::<Vec<(usize, u64)>>();

                    weighted_indexes.sort_by(|a, b| a.1.cmp(&b.1));
                    for &(j, _) in weighted_indexes.iter() {
                        let start = amounts[j];
                        let amount = if start + value >= 0 { -value } else { start };
                        pay_liab(amounts, i, j, amount, &mut reimburse_totals);
                        value += amount;
                        if value >= 0 {
                            break;
                        }
                    }
                }

                // Otherwise settle against some other token with positive balance.
                //
                // mSOL is third to last because it looks like we will have a lot of it and want
                // to prefer giving it out to users.
                // USDC is after tokens, because settling tokens first leads to better results
                // (consider delta-neutral positions)
                // MNGO is last, meaning that Mango tokens are only used as an asset to offset a
                // liability as last resort, because we force it to a bad price.
                for j in [14, 13, 12, 11, 9, 8, 7, 6, 5, 4, 3, 2, 1, 10, 15, 0] {
                    if amounts[j] <= 0 {
                        continue;
                    }
                    let start = amounts[j];
                    let amount = if start + value >= 0 { -value } else { start };
                    pay_liab(amounts, i, j, amount, &mut reimburse_totals);
                    value += amount;
                    if value >= 0 {
                        break;
                    }
                }

                assert!(value == 0);
                assert!(amounts[i] == 0);
            }
        }

        // now all reimburse_amounts are >= 0

        // Do a pass where we scale down user reimbursement token amounts and instead
        // reimburse with USDC if there's not enough tokens to give out
        let scale_down_iter = if !ctx.args.enable_scale_down {
            0..0
        } else if ctx.args.distribution_mode == DistributionMode::UsdcOnly {
            0..15
        } else {
            1..15 // keep MNGO intact, DAO can provide extra from treasury
        };
        for i in scale_down_iter {
            if reimburse_totals[i] == 0 || reimburse_totals[i] == available_amounts[i] {
                continue;
            }
            let fraction = I80F48::from(available_amounts[i]) / I80F48::from(reimburse_totals[i]);
            if fraction >= 1 {
                continue;
            }

            // Scale down token reimbursements and replace them with USDC reimbursements
            for AccountData { amounts, .. } in reimburse_amounts.iter_mut() {
                let start_amount = amounts[i];
                assert!(start_amount >= 0);
                if start_amount == 0 {
                    continue;
                }

                let new_amount: i64 = (I80F48::from(start_amount) * fraction).to_num();
                let amount = start_amount - new_amount;
                let target = if i == 3 {
                    10 // SOL -> mSOL
                } else {
                    QUOTE_INDEX
                };
                move_amount(amounts, i, target, amount, &mut reimburse_totals);
            }
        }

        // Double check that total user equity is unchanged
        let mut accounts_with_mngo = 0;
        let mut accounts_with_mngo_unchanged = 0;
        for (a_equity, a_reimburse) in account_equities.iter().zip(reimburse_amounts.iter()) {
            let eqsum = a_equity.amounts.iter().sum::<i64>();
            let resum = a_reimburse.amounts.iter().sum::<i64>();
            assert_eq!(eqsum, resum);

            if ctx.args.distribution_mode == DistributionMode::Tokens {
                let mngo_equity = a_equity.amounts[0];
                let mngo_reimburse = a_reimburse.amounts[0];
                if mngo_equity > 0 {
                    // MNGO amount can only go down
                    assert!(mngo_reimburse <= mngo_equity);
                    accounts_with_mngo += 1;
                    if mngo_reimburse == mngo_equity {
                        accounts_with_mngo_unchanged += 1;
                    }
                }
            }

            assert_eq!(a_equity.owner, a_reimburse.owner);
        }

        println!("account w mango: {accounts_with_mngo}, unchanged {accounts_with_mngo_unchanged}");

        println!("token,available in usd,used in usd,remaining in usd,buy/sell in token ui");
        for i in 0..15 {
            println!(
                "{},{},{},{},{}",
                token_names[i],
                available_amounts[i] / 1000000,
                reimburse_totals[i] / 1000000,
                (available_amounts[i] as i64 - reimburse_totals[i] as i64) / 1000000,
                -ctx.constants
                    .usd_to_tokens_ui(i, available_amounts[i] as i64 - reimburse_totals[i] as i64),
            );
        }
        println!("USDC: used {}", reimburse_totals[15] / 1000000);
        println!("reimburse total {}", reimburse_totals.iter().sum::<u64>() / 1000000);

        debug_print("usd final", &reimburse_amounts);

        let mut reimburse_native = reimburse_amounts.clone();
        for a in reimburse_native.iter_mut() {
            for (index, v) in a.amounts.iter_mut().enumerate() {
                *v = (I80F48::from(*v) / ctx.constants.token_infos[index].reimbursement_price)
                    .floor()
                    .to_num();
            }
        }

        // drop any full-zero rows
        reimburse_native =
            reimburse_native.into_iter().filter(|a| a.amounts.iter().sum::<i64>() != 0).collect();

        outwriter.write_header(&ctx.constants);
        for a in reimburse_native.iter() {
            outwriter.write(a)
        }

        Ok(())
    }

    fn snapshot_price(&self, index: usize) -> I80F48 {
        if index == QUOTE_INDEX {
            I80F48::ONE
        } else {
            self.cache.price_cache[index].price
        }
    }

    fn snapshot_price_perp(&self, index: usize) -> I80F48 {
        if index == 0 {
            self.mngo_perp_price
        } else {
            self.snapshot_price(index)
        }
    }

    fn snapshot_account_equities(&self) -> anyhow::Result<Vec<AccountData>> {
        let account_addresses =
            self.data.mango_account_list(self.args.program, DataType::MangoAccount)?;

        let mut account_equities: Vec<AccountData> = Vec::with_capacity(account_addresses.len()); // get the snapshot account equities
        for mango_account in account_addresses {
            let equity_opt = self
                .account_equity(mango_account)
                .context(format!("on account {}", mango_account))?;
            if equity_opt.is_none() {
                continue;
            }
            let (owner, amounts) = equity_opt.unwrap();
            account_equities.push(AccountData { mango_account, owner, amounts });
        }

        Ok(account_equities)
    }

    fn apply_late_deposits_withdrawals(
        &self,
        account_equities: &mut Vec<AccountData>,
    ) -> anyhow::Result<()> {
        // apply the late deposits/withdrawals
        for &(mango_account, owner, token_index, change_native) in self.late_changes.iter() {
            let change_usd =
                (I80F48::from(change_native) * self.snapshot_price(token_index)).to_num();
            // slow, but just ran a handful times
            let account_opt =
                account_equities.iter_mut().find(|a| a.mango_account == mango_account);
            if let Some(account) = account_opt {
                account.amounts[token_index] += change_usd;
            } else {
                assert!(change_usd > 0);
                let mut amounts = AccountTokenAmounts::default();
                amounts[token_index] = change_usd;
                account_equities.push(AccountData { mango_account, owner, amounts });
            }
        }

        Ok(())
    }

    fn skip_negative_equity_accounts(
        &self,
        account_equities: &mut Vec<AccountData>,
    ) -> anyhow::Result<()> {
        // Some accounts have negative equity because they already cashed out on a MNGO PERP position
        // that started to be valuable after the snapshot was taken, skip them
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
                let account =
                    account_equities.iter_mut().find(|a| a.mango_account == address).unwrap();
                assert!(self.late_changes.iter().any(|(a, _, _, c)| a == &address && *c < 0));
                let total = account.amounts.iter().sum::<i64>();
                assert!(total < 0);
                assert!(total > -10_000_000_000); // none of these was bigger than 10000 USD
                account.amounts = AccountTokenAmounts::default();
            }
        }

        // Negative equity accounts can happen due to us using a post-snapshot MNGO price
        for account in account_equities.iter_mut() {
            let equity = &mut account.amounts;
            let total = equity.iter().sum::<i64>();
            if total >= 0 {
                continue;
            }

            let mngo_equity = I80F48::from(equity[0]);
            let old_mngo_equity = mngo_equity / self.constants.token_infos[0].reimbursement_price
                * I80F48::from_num(0.0387250);
            let old_equity = old_mngo_equity.to_num::<i64>() + equity[1..].iter().sum::<i64>();

            if old_equity >= 0 {
                // negative equity due to changed MNGO price, is ok
                *equity = AccountTokenAmounts::default();
            }
        }

        // Some accounts withdrew everything after the snapshot was taken. When doing that they
        // probably withdrew a tiny bit more than their snapshot equity due to interest.
        // These accounts have already cashed out, no need to reimburse.
        for account in account_equities.iter_mut() {
            let equity = &mut account.amounts;
            let total = equity.iter().sum::<i64>();
            if total >= 0 {
                continue;
            }
            let had_withdrawal =
                self.late_changes.iter().any(|(a, _, _, c)| a == &account.mango_account && *c < 0);
            assert!(had_withdrawal);

            // only up to -10 USD is expected, otherwise investigate manually!
            assert!(total > -10_000_000);
            *equity = AccountTokenAmounts::default();
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
            let price = self.snapshot_price(oracle_index);
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
            let price = self.snapshot_price(oracle_index);
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
            if self.group.perp_markets[oracle_index].is_empty() {
                continue;
            }

            let mngo_index = 0;
            let mngo = mango_account.perp_accounts[oracle_index].mngo_accrued;
            equity[mngo_index] = equity[mngo_index]
                .checked_add(
                    I80F48::from(mngo).checked_mul(self.snapshot_price(mngo_index)).unwrap(),
                )
                .unwrap();

            if !mango_account.perp_accounts[oracle_index].is_active() {
                continue;
            }
            let price = self.snapshot_price_perp(oracle_index);
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
