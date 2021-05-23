use crate::error::{check_assert, MerpsErrorCode, MerpsResult, SourceFileId};
use bytemuck::{Pod, Zeroable};
use safe_transmute::{self, trivial::TriviallyTransmutable};
use std::cell::RefMut;

declare_check_assert_macros!(SourceFileId::Queue);

// Don't want event queue to become single threaded if it's logging liquidations
// Most common scenario will be liqors depositing USDC and withdrawing some other token
// So tying it to token deposited is not wise
// also can't tie it to token withdrawn because during bull market, liqs will be depositing all base tokens and withdrawing quote
//

pub trait QueueHeader: Pod {
    type Item: Pod + Copy;

    fn head(&self) -> u64;
    fn set_head(&mut self, value: u64);
    fn count(&self) -> u64;
    fn set_count(&mut self, value: u64);

    fn incr_event_id(&mut self);
    fn decr_event_id(&mut self, n: u64);
}

pub struct Queue<'a, H: QueueHeader> {
    header: RefMut<'a, H>,
    buf: RefMut<'a, [H::Item]>,
}

impl<'a, H: QueueHeader> Queue<'a, H> {
    pub fn new(header: RefMut<'a, H>, buf: RefMut<'a, [H::Item]>) -> Self {
        Self { header, buf }
    }

    pub fn len(&self) -> u64 {
        self.header.count()
    }

    pub fn full(&self) -> bool {
        self.header.count() as usize == self.buf.len()
    }

    pub fn empty(&self) -> bool {
        self.header.count() == 0
    }

    pub fn push_back(&mut self, value: H::Item) -> Result<(), H::Item> {
        if self.full() {
            return Err(value);
        }
        let slot = ((self.header.head() + self.header.count()) as usize) % self.buf.len();
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
        Some(&self.buf[self.header.head() as usize])
    }

    pub fn peek_front_mut(&mut self) -> Option<&mut H::Item> {
        if self.empty() {
            return None;
        }
        Some(&mut self.buf[self.header.head() as usize])
    }

    pub fn pop_front(&mut self) -> Result<H::Item, ()> {
        if self.empty() {
            return Err(());
        }
        let value = self.buf[self.header.head() as usize];

        let count = self.header.count();
        self.header.set_count(count - 1);

        let head = self.header.head();
        self.header.set_head((head + 1) % self.buf.len() as u64);

        Ok(value)
    }

    pub fn revert_pushes(&mut self, desired_len: u64) -> MerpsResult<()> {
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
    index: u64,
}

impl<'a, 'b, H: QueueHeader> Iterator for QueueIterator<'a, 'b, H> {
    type Item = &'b H::Item;
    fn next(&mut self) -> Option<Self::Item> {
        if self.index == self.queue.len() {
            None
        } else {
            let item = &self.queue.buf
                [(self.queue.header.head() + self.index) as usize % self.queue.buf.len()];
            self.index += 1;
            Some(item)
        }
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct EventQueueHeader {
    account_flags: u64, // Initialized, EventQueue
    head: u64,
    count: u64,
    seq_num: u64,
}
unsafe impl Zeroable for EventQueueHeader {}
unsafe impl Pod for EventQueueHeader {}

unsafe impl TriviallyTransmutable for EventQueueHeader {}

impl QueueHeader for EventQueueHeader {
    type Item = AnyEvent;

    fn head(&self) -> u64 {
        self.head
    }
    fn set_head(&mut self, value: u64) {
        self.head = value;
    }
    fn count(&self) -> u64 {
        self.count
    }
    fn set_count(&mut self, value: u64) {
        self.count = value;
    }
    fn incr_event_id(&mut self) {
        self.seq_num += 1;
    }
    fn decr_event_id(&mut self, n: u64) {
        self.seq_num -= n;
    }
}

pub type EventQueue<'a> = Queue<'a, EventQueueHeader>;

#[derive(Copy, Clone)]
#[repr(u8)]
pub enum EventType {
    Fill,
    Out,
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct AnyEvent {
    pub event_type: u8,
    pub padding: [u8; 7],
}
unsafe impl Zeroable for AnyEvent {}
unsafe impl Pod for AnyEvent {}
unsafe impl TriviallyTransmutable for AnyEvent {}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct FillEvent {
    pub event_type: u8,
    pub padding: [u8; 7],
}
unsafe impl Zeroable for FillEvent {}
unsafe impl Pod for FillEvent {}
unsafe impl TriviallyTransmutable for FillEvent {}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct OutEvent {
    pub event_type: u8,
    pub padding: [u8; 7],
}
unsafe impl Zeroable for OutEvent {}
unsafe impl Pod for OutEvent {}
unsafe impl TriviallyTransmutable for OutEvent {}
