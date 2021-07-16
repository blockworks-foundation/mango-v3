use fixed::types::I80F48;
use mango_common::Loadable;
use mango_macro::{Loadable, Pod};
use solana_program::{account_info::AccountInfo, pubkey::Pubkey, rent::Rent};
use std::{cell::RefMut, mem::size_of};

use crate::error::{check_assert, MangoErrorCode, MangoResult, SourceFileId};

declare_check_assert_macros!(SourceFileId::Oracle);

#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct StubOracle {
    pub magic: u32,    // Magic byte
    pub price: I80F48, // unit is interpreted as how many quote native tokens for 1 base native token
    pub last_update: u64,
}

// TODO move to separate program
impl StubOracle {
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MangoResult<RefMut<'a, Self>> {
        check_eq!(account.data_len(), size_of::<Self>(), MangoErrorCode::Default)?;
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;

        let oracle = Self::load_mut(account)?;

        Ok(oracle)
    }

    pub fn load_and_init<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        rent: &Rent,
    ) -> MangoResult<RefMut<'a, Self>> {
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;
        check!(
            rent.is_exempt(account.lamports(), account.data_len()),
            MangoErrorCode::AccountNotRentExempt
        )?;

        let oracle = Self::load_mut(account)?;

        Ok(oracle)
    }
}

// Start of pyth implementation

pub const MAGIC: u32 = 0xa1b2c3d4;
pub const VERSION_2: u32 = 2;
pub const VERSION: u32 = VERSION_2;
pub const MAP_TABLE_SIZE: usize = 640;
pub const PROD_ACCT_SIZE: usize = 512;
pub const PROD_HDR_SIZE: usize = 48;
pub const PROD_ATTR_SIZE: usize = PROD_ACCT_SIZE - PROD_HDR_SIZE;

// oracle can be of different types
#[derive(PartialEq)]
#[repr(C)]
pub enum OracleType {
    Stub,
    Pyth,
    Unknown,
}

// each account has its own type
#[repr(C)]
pub enum AccountType {
    Unknown,
    Mapping,
    Product,
    Price,
}

// aggregate and contributing prices are associated with a status
// only Trading status is valid
#[derive(Copy, Clone)]
#[repr(C)]
pub enum PriceStatus {
    Unknown,
    Trading,
    Halted,
    Auction,
}

// ongoing coporate action event - still undergoing dev
#[derive(Copy, Clone)]
#[repr(C)]
pub enum CorpAction {
    NoCorpAct,
}

// different types of prices associated with a product
#[derive(Copy, Clone)]
#[repr(C)]
pub enum PriceType {
    Unknown,
    Price,
}

// solana public key
#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct AccKey {
    pub val: [u8; 32],
}

// Mapping account structure
#[repr(C)]
pub struct Mapping {
    pub magic: u32, // pyth magic number
    pub ver: u32,   // program version
    pub atype: u32, // account type
    pub size: u32,  // account used size
    pub num: u32,   // number of product accounts
    pub unused: u32,
    pub next: AccKey, // next mapping account (if any)
    pub products: [AccKey; MAP_TABLE_SIZE],
}

// Product account structure
#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct Product {
    pub magic: u32,                 // pyth magic number
    pub ver: u32,                   // program version
    pub atype: u32,                 // account type
    pub size: u32,                  // price account size
    pub px_acc: AccKey,             // first price account in list
    pub attr: [u8; PROD_ATTR_SIZE], // key/value pairs of reference attr.
}

impl Product {
    pub fn get_product<'a>(account: &'a AccountInfo) -> MangoResult<Product> {
        let borrowed = &account.data.borrow();
        let product = cast::<Product>(&borrowed);
        assert_eq!(product.magic, MAGIC, "not a valid pyth account");
        assert_eq!(product.atype, AccountType::Product as u32, "not a valid pyth product account");
        assert_eq!(product.ver, VERSION_2, "unexpected pyth product account version");
        Ok(*product)
    }
}

// contributing or aggregate price component
#[derive(Copy, Clone)]
#[repr(C)]
pub struct PriceInfo {
    pub price: i64,           // product price
    pub conf: u64,            // confidence interval of product price
    pub status: PriceStatus,  // status of price (Trading is valid)
    pub corp_act: CorpAction, // notification of any corporate action
    pub pub_slot: u64,
}

// latest component price and price used in aggregate snapshot
#[derive(Copy, Clone)]
#[repr(C)]
pub struct PriceComp {
    publisher: AccKey, // key of contributing quoter
    agg: PriceInfo,    // contributing price to last aggregate
    latest: PriceInfo, // latest contributing price (not in agg.)
}

// Price account structure
#[derive(Copy, Clone)]
#[repr(C)]
pub struct Price {
    pub magic: u32,       // pyth magic number
    pub ver: u32,         // program version
    pub atype: u32,       // account type
    pub size: u32,        // price account size
    pub ptype: PriceType, // price or calculation type
    pub expo: i32,        // price exponent
    pub num: u32,         // number of component prices
    pub unused: u32,
    pub curr_slot: u64,        // currently accumulating price slot
    pub valid_slot: u64,       // valid slot-time of agg. price
    pub twap: i64,             // time-weighted average price
    pub avol: u64,             // annualized price volatility
    pub drv0: i64,             // space for future derived values
    pub drv1: i64,             // space for future derived values
    pub drv2: i64,             // space for future derived values
    pub drv3: i64,             // space for future derived values
    pub drv4: i64,             // space for future derived values
    pub drv5: i64,             // space for future derived values
    pub prod: AccKey,          // product account key
    pub next: AccKey,          // next Price account in linked list
    pub agg_pub: AccKey,       // quoter who computed last aggregate price
    pub agg: PriceInfo,        // aggregate price info
    pub comp: [PriceComp; 32], // price components one per quoter
}

impl Price {
    pub fn get_price<'a>(account: &'a AccountInfo) -> MangoResult<Price> {
        let borrowed = &account.data.borrow();
        let price = cast::<Price>(&borrowed);
        assert_eq!(price.magic, MAGIC, "not a valid pyth account");
        assert_eq!(price.atype, AccountType::Price as u32, "not a valid pyth price account");
        assert_eq!(price.ver, VERSION_2, "unexpected pyth price account version");
        Ok(*price)
    }
}

struct AccKeyU64 {
    pub val: [u64; 4],
}

pub fn cast<T>(d: &[u8]) -> &T {
    let (_, pxa, _) = unsafe { d.align_to::<T>() };
    &pxa[0]
}

impl AccKey {
    pub fn is_valid(&self) -> bool {
        let k8 = cast::<AccKeyU64>(&self.val);
        return k8.val[0] != 0 || k8.val[1] != 0 || k8.val[2] != 0 || k8.val[3] != 0;
    }
}

pub fn determine_oracle_type<'a>(account: &'a AccountInfo) -> OracleType {
    let borrowed = &account.data.borrow();
    if borrowed[0] == 212 && borrowed[1] == 195 && borrowed[2] == 178 && borrowed[3] == 161 {
        return OracleType::Pyth;
    } else if borrowed[0] == 77 && borrowed[1] == 110 && borrowed[2] == 103 && borrowed[3] == 111 {
        return OracleType::Stub;
    } else {
        return OracleType::Unknown;
    }
}
