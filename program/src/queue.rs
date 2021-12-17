use crate::error::{check_assert, MangoErrorCode, MangoResult, SourceFileId};
use crate::matching::Side;
use crate::state::{DataType, MetaData, PerpMarket};
use crate::utils::strip_header_mut;

use fixed::types::I80F48;
use mango_logs::FillLog;
use mango_macro::Pod;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use safe_transmute::{self, trivial::TriviallyTransmutable};
use solana_program::account_info::AccountInfo;
use solana_program::pubkey::Pubkey;
use solana_program::sysvar::rent::Rent;
use static_assertions::const_assert_eq;
use std::cell::RefMut;
use std::mem::size_of;

declare_check_assert_macros!(SourceFileId::Queue);

// Don't want event queue to become single threaded if it's logging liquidations
// Most common scenario will be liqors depositing USDC and withdrawing some other token
// So tying it to token deposited is not wise
// also can't tie it to token withdrawn because during bull market, liqs will be depositing all base tokens and withdrawing quote
//

pub trait QueueHeader: bytemuck::Pod {
    type Item: bytemuck::Pod + Copy;

    fn head(&self) -> usize;
    fn set_head(&mut self, value: usize);
    fn count(&self) -> usize;
    fn set_count(&mut self, value: usize);

    fn incr_event_id(&mut self);
    fn decr_event_id(&mut self, n: usize);
}

pub struct Queue<'a, H: QueueHeader> {
    pub header: RefMut<'a, H>,
    pub buf: RefMut<'a, [H::Item]>,
}

impl<'a, H: QueueHeader> Queue<'a, H> {
    pub fn new(header: RefMut<'a, H>, buf: RefMut<'a, [H::Item]>) -> Self {
        Self { header, buf }
    }

    pub fn load_mut(account: &'a AccountInfo) -> MangoResult<Self> {
        let (header, buf) = strip_header_mut::<H, H::Item>(account)?;
        Ok(Self { header, buf })
    }

    pub fn len(&self) -> usize {
        self.header.count()
    }

    pub fn full(&self) -> bool {
        self.header.count() == self.buf.len()
    }

    pub fn empty(&self) -> bool {
        self.header.count() == 0
    }

    pub fn push_back(&mut self, value: H::Item) -> Result<(), H::Item> {
        if self.full() {
            return Err(value);
        }
        let slot = (self.header.head() + self.header.count()) % self.buf.len();
        self.buf[slot] = value;

        let count = self.header.count();
        self.header.set_count(count + 1);

        self.header.incr_event_id();
        Ok(())
    }

    pub fn peek_front(&self) -> Option<&H::Item> {
        if self.empty() {
            return None;
        }
        Some(&self.buf[self.header.head()])
    }

    pub fn peek_front_mut(&mut self) -> Option<&mut H::Item> {
        if self.empty() {
            return None;
        }
        Some(&mut self.buf[self.header.head()])
    }

    pub fn pop_front(&mut self) -> Result<H::Item, ()> {
        if self.empty() {
            return Err(());
        }
        let value = self.buf[self.header.head()];

        let count = self.header.count();
        self.header.set_count(count - 1);

        let head = self.header.head();
        self.header.set_head((head + 1) % self.buf.len());

        Ok(value)
    }

    pub fn revert_pushes(&mut self, desired_len: usize) -> MangoResult<()> {
        check!(desired_len <= self.header.count(), MangoErrorCode::Default)?;
        let len_diff = self.header.count() - desired_len;
        self.header.set_count(desired_len);
        self.header.decr_event_id(len_diff);
        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = &H::Item> {
        QueueIterator { queue: self, index: 0 }
    }
}

struct QueueIterator<'a, 'b, H: QueueHeader> {
    queue: &'b Queue<'a, H>,
    index: usize,
}

impl<'a, 'b, H: QueueHeader> Iterator for QueueIterator<'a, 'b, H> {
    type Item = &'b H::Item;
    fn next(&mut self) -> Option<Self::Item> {
        if self.index == self.queue.len() {
            None
        } else {
            let item =
                &self.queue.buf[(self.queue.header.head() + self.index) % self.queue.buf.len()];
            self.index += 1;
            Some(item)
        }
    }
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct EventQueueHeader {
    pub meta_data: MetaData,
    head: usize,
    count: usize,
    pub seq_num: usize,
}
unsafe impl TriviallyTransmutable for EventQueueHeader {}

impl QueueHeader for EventQueueHeader {
    type Item = AnyEvent;

    fn head(&self) -> usize {
        self.head
    }
    fn set_head(&mut self, value: usize) {
        self.head = value;
    }
    fn count(&self) -> usize {
        self.count
    }
    fn set_count(&mut self, value: usize) {
        self.count = value;
    }
    fn incr_event_id(&mut self) {
        self.seq_num += 1;
    }
    fn decr_event_id(&mut self, n: usize) {
        self.seq_num -= n;
    }
}

pub type EventQueue<'a> = Queue<'a, EventQueueHeader>;

impl<'a> EventQueue<'a> {
    pub fn load_mut_checked(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        perp_market: &PerpMarket,
    ) -> MangoResult<Self> {
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;
        check_eq!(&perp_market.event_queue, account.key, MangoErrorCode::InvalidAccount)?;
        Self::load_mut(account)
    }

    pub fn load_and_init(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        rent: &Rent,
    ) -> MangoResult<Self> {
        // NOTE: check this first so we can borrow account later
        check!(
            rent.is_exempt(account.lamports(), account.data_len()),
            MangoErrorCode::AccountNotRentExempt
        )?;

        let mut state = Self::load_mut(account)?;
        check!(account.owner == program_id, MangoErrorCode::InvalidOwner)?;

        check!(!state.header.meta_data.is_initialized, MangoErrorCode::Default)?;
        state.header.meta_data = MetaData::new(DataType::EventQueue, 0, true);

        Ok(state)
    }
}

#[derive(Copy, Clone, IntoPrimitive, TryFromPrimitive, Eq, PartialEq)]
#[repr(u8)]
pub enum EventType {
    Fill,
    Out,
    Liquidate,
}

const EVENT_SIZE: usize = 200;
#[derive(Copy, Clone, Debug, Pod)]
#[repr(C)]
pub struct AnyEvent {
    pub event_type: u8,
    pub padding: [u8; EVENT_SIZE - 1],
}
unsafe impl TriviallyTransmutable for AnyEvent {}

#[derive(Copy, Clone, Debug, Pod)]
#[repr(C)]
pub struct FillEvent {
    pub event_type: u8,
    pub taker_side: Side, // side from the taker's POV
    pub maker_slot: u8,
    pub maker_out: bool, // true if maker order quantity == 0
    pub version: u8,
    pub padding: [u8; 3],
    pub timestamp: u64,
    pub seq_num: usize, // note: usize same as u64

    pub maker: Pubkey,
    pub maker_order_id: i128,
    pub maker_client_order_id: u64,
    pub maker_fee: I80F48,

    // The best bid/ask at the time the maker order was placed. Used for liquidity incentives
    pub best_initial: i64,

    // Timestamp of when the maker order was placed; copied over from the LeafNode
    pub maker_timestamp: u64,

    pub taker: Pubkey,
    pub taker_order_id: i128,
    pub taker_client_order_id: u64,
    pub taker_fee: I80F48,

    pub price: i64,
    pub quantity: i64, // number of quote lots
}
unsafe impl TriviallyTransmutable for FillEvent {}

impl FillEvent {
    pub fn new(
        taker_side: Side,
        maker_slot: u8,
        maker_out: bool,
        timestamp: u64,
        seq_num: usize,
        maker: Pubkey,
        maker_order_id: i128,
        maker_client_order_id: u64,
        maker_fee: I80F48,
        best_initial: i64,
        maker_timestamp: u64,

        taker: Pubkey,
        taker_order_id: i128,
        taker_client_order_id: u64,
        taker_fee: I80F48,
        price: i64,
        quantity: i64,
        version: u8,
    ) -> FillEvent {
        Self {
            event_type: EventType::Fill as u8,
            taker_side,
            maker_slot,
            maker_out,
            version,
            padding: [0u8; 3],
            timestamp,
            seq_num,
            maker,
            maker_order_id,
            maker_client_order_id,
            maker_fee,
            best_initial,
            maker_timestamp,
            taker,
            taker_order_id,
            taker_client_order_id,
            taker_fee,
            price,
            quantity,
        }
    }

    pub fn base_quote_change(&self, side: Side) -> (i64, i64) {
        match side {
            Side::Bid => (self.quantity, -self.price.checked_mul(self.quantity).unwrap()),
            Side::Ask => (-self.quantity, self.price.checked_mul(self.quantity).unwrap()),
        }
    }

    pub fn to_fill_log(&self, mango_group: Pubkey, market_index: usize) -> FillLog {
        FillLog {
            mango_group,
            market_index: market_index as u64,
            taker_side: self.taker_side as u8,
            maker_slot: self.maker_slot,
            maker_out: self.maker_out,
            timestamp: self.timestamp,
            seq_num: self.seq_num as u64,
            maker: self.maker,
            maker_order_id: self.maker_order_id,
            maker_client_order_id: self.maker_client_order_id,
            maker_fee: self.maker_fee.to_bits(),
            best_initial: self.best_initial,
            maker_timestamp: self.maker_timestamp,
            taker: self.taker,
            taker_order_id: self.taker_order_id,
            taker_client_order_id: self.taker_client_order_id,
            taker_fee: self.taker_fee.to_bits(),
            price: self.price,
            quantity: self.quantity,
        }
    }
}

#[derive(Copy, Clone, Debug, Pod)]
#[repr(C)]
pub struct OutEvent {
    pub event_type: u8,
    pub side: Side,
    pub slot: u8,
    padding0: [u8; 5],
    pub timestamp: u64,
    pub seq_num: usize,
    pub owner: Pubkey,
    pub quantity: i64,
    padding1: [u8; EVENT_SIZE - 64],
}
unsafe impl TriviallyTransmutable for OutEvent {}
impl OutEvent {
    pub fn new(
        side: Side,
        slot: u8,
        timestamp: u64,
        seq_num: usize,
        owner: Pubkey,
        quantity: i64,
    ) -> Self {
        Self {
            event_type: EventType::Out.into(),
            side,
            slot,
            padding0: [0; 5],
            timestamp,
            seq_num,
            owner,
            quantity,
            padding1: [0; EVENT_SIZE - 64],
        }
    }
}

#[derive(Copy, Clone, Debug, Pod)]
#[repr(C)]
/// Liquidation for the PerpMarket this EventQueue is for
pub struct LiquidateEvent {
    pub event_type: u8,
    padding0: [u8; 7],
    pub timestamp: u64,
    pub seq_num: usize,
    pub liqee: Pubkey,
    pub liqor: Pubkey,
    pub price: I80F48,           // oracle price at the time of liquidation
    pub quantity: i64,           // number of contracts that were moved from liqee to liqor
    pub liquidation_fee: I80F48, // liq fee for this earned for this market
    padding1: [u8; EVENT_SIZE - 128],
}
unsafe impl TriviallyTransmutable for LiquidateEvent {}
impl LiquidateEvent {
    pub fn new(
        timestamp: u64,
        seq_num: usize,
        liqee: Pubkey,
        liqor: Pubkey,
        price: I80F48,
        quantity: i64,
        liquidation_fee: I80F48,
    ) -> Self {
        Self {
            event_type: EventType::Liquidate.into(),
            padding0: [0u8; 7],
            timestamp,
            seq_num,
            liqee,
            liqor,
            price,
            quantity,
            liquidation_fee,
            padding1: [0u8; EVENT_SIZE - 128],
        }
    }
}
const_assert_eq!(size_of::<AnyEvent>(), size_of::<FillEvent>());
const_assert_eq!(size_of::<AnyEvent>(), size_of::<OutEvent>());
const_assert_eq!(size_of::<AnyEvent>(), size_of::<LiquidateEvent>());
