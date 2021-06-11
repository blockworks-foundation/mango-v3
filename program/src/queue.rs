use crate::error::{check_assert, MerpsErrorCode, MerpsResult, SourceFileId};
use crate::matching::Side;
use crate::state::{DataType, MetaData, PerpMarket};
use crate::utils::strip_header_mut;
use bytemuck::Pod;
use fixed::types::I80F48;
use mango_macro::Pod;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use safe_transmute::{self, trivial::TriviallyTransmutable};
use solana_program::account_info::AccountInfo;
use solana_program::pubkey::Pubkey;
use solana_program::sysvar::rent::Rent;
use std::cell::RefMut;

declare_check_assert_macros!(SourceFileId::Queue);

// Don't want event queue to become single threaded if it's logging liquidations
// Most common scenario will be liqors depositing USDC and withdrawing some other token
// So tying it to token deposited is not wise
// also can't tie it to token withdrawn because during bull market, liqs will be depositing all base tokens and withdrawing quote
//

pub trait QueueHeader: Pod {
    type Item: Pod + Copy;

    fn head(&self) -> usize;
    fn set_head(&mut self, value: usize);
    fn count(&self) -> usize;
    fn set_count(&mut self, value: usize);

    fn incr_event_id(&mut self);
    fn decr_event_id(&mut self, n: usize);
}

pub struct Queue<'a, H: QueueHeader> {
    pub header: RefMut<'a, H>,
    buf: RefMut<'a, [H::Item]>,
}

impl<'a, H: QueueHeader> Queue<'a, H> {
    pub fn new(header: RefMut<'a, H>, buf: RefMut<'a, [H::Item]>) -> Self {
        Self { header, buf }
    }

    pub fn load_mut(account: &'a AccountInfo) -> MerpsResult<Self> {
        // TODO
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

    pub fn revert_pushes(&mut self, desired_len: usize) -> MerpsResult<()> {
        check!(desired_len <= self.header.count(), MerpsErrorCode::Default)?;
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
    seq_num: usize,
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
    ) -> MerpsResult<Self> {
        check_eq!(account.owner, program_id, MerpsErrorCode::InvalidOwner)?;
        check_eq!(&perp_market.event_queue, account.key, MerpsErrorCode::Default)?;
        Self::load_mut(account)
    }

    pub fn load_and_init(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        rent: &Rent,
    ) -> MerpsResult<Self> {
        // TODO: check for size

        // NOTE: check this first so we can borrow account later
        check!(
            rent.is_exempt(account.lamports(), account.data_len()),
            MerpsErrorCode::AccountNotRentExempt
        )?;

        let mut state = Self::load_mut(account)?;
        check!(account.owner == program_id, MerpsErrorCode::InvalidOwner)?;

        check!(!state.header.meta_data.is_initialized, MerpsErrorCode::Default)?;
        state.header.meta_data = MetaData::new(DataType::EventQueue, 0, true);
        Ok(state)
    }
}

#[derive(Copy, Clone, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
pub enum EventType {
    Fill,
    Out,
}

const EVENT_SIZE: usize = 96;
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
    pub maker: bool,
    padding: [u8; 6],
    pub owner: Pubkey,
    pub base_change: i64,
    pub quote_change: i64, // number of quote lots
    pub long_funding: I80F48,
    pub short_funding: I80F48,
}
unsafe impl TriviallyTransmutable for FillEvent {}

impl FillEvent {
    pub fn new(
        maker: bool,
        owner: Pubkey,
        base_change: i64,
        quote_change: i64,
        long_funding: I80F48,
        short_funding: I80F48,
    ) -> Self {
        Self {
            event_type: EventType::Fill.into(),
            maker,
            padding: [0; 6],
            owner,
            base_change,
            quote_change,
            long_funding,
            short_funding,
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
    pub owner: Pubkey,
    pub quantity: i64,
    padding1: [u8; EVENT_SIZE - 16],
}
unsafe impl TriviallyTransmutable for OutEvent {}
impl OutEvent {
    pub fn new(side: Side, slot: u8, quantity: i64, owner: Pubkey) -> Self {
        Self {
            event_type: EventType::Out.into(),
            side,
            slot,
            padding0: [0; 5],
            owner,
            quantity,
            padding1: [0; EVENT_SIZE - 16],
        }
    }
}
