use std::cell::RefMut;
use std::convert::TryFrom;
use std::mem::size_of;

use bytemuck::{cast, cast_mut, cast_ref};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{Deserialize, Serialize};
use solana_program::account_info::AccountInfo;
use solana_program::clock::Clock;
use solana_program::msg;
use solana_program::pubkey::Pubkey;
use solana_program::sysvar::rent::Rent;
use solana_program::sysvar::Sysvar;
use static_assertions::const_assert_eq;

use mango_common::Loadable;
use mango_macro::{Loadable, Pod};

use crate::error::{check_assert, MangoError, MangoErrorCode, MangoResult, SourceFileId};
use crate::queue::{EventQueue, FillEvent, OutEvent};
use crate::state::{
    DataType, MangoAccount, MetaData, PerpMarket, PerpMarketInfo, MAX_PERP_OPEN_ORDERS,
};

declare_check_assert_macros!(SourceFileId::Matching);
pub type NodeHandle = u32;

const NODE_SIZE: usize = 88;

#[derive(IntoPrimitive, TryFromPrimitive)]
#[repr(u32)]
pub enum NodeTag {
    Uninitialized = 0,
    InnerNode = 1,
    LeafNode = 2,
    FreeNode = 3,
    LastFreeNode = 4,
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct InnerNode {
    pub tag: u32,
    pub prefix_len: u32,
    pub key: i128,
    pub children: [u32; 2],
    pub padding: [u8; NODE_SIZE - 32],
}

impl InnerNode {
    fn new(prefix_len: u32, key: i128) -> Self {
        Self {
            tag: NodeTag::InnerNode.into(),
            prefix_len,
            key,
            children: [0; 2],
            padding: [0; NODE_SIZE - 32],
        }
    }
    fn walk_down(&self, search_key: i128) -> (NodeHandle, bool) {
        let crit_bit_mask = 1i128 << (127 - self.prefix_len);
        let crit_bit = (search_key & crit_bit_mask) != 0;
        (self.children[crit_bit as usize], crit_bit)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Pod)]
#[repr(C)]
pub struct LeafNode {
    pub tag: u32,
    pub owner_slot: u8,
    pub order_type: OrderType, // *** this was added for TradingView move order
    pub padding: [u8; 2],
    pub key: i128,
    pub owner: Pubkey,
    pub quantity: i64,
    pub client_order_id: u64,

    // Liquidity incentive related parameters
    // Either the best bid or best ask at the time the order was placed
    pub best_initial: i64,

    // The time the order was place
    pub timestamp: u64,
}

impl LeafNode {
    pub fn price(&self) -> i64 {
        (self.key >> 64) as i64
    }

    pub fn new(
        owner_slot: u8,
        key: i128,
        owner: Pubkey,
        quantity: i64,
        client_order_id: u64,
        timestamp: u64,
        best_initial: i64,
        order_type: OrderType,
    ) -> Self {
        Self {
            tag: NodeTag::LeafNode.into(),
            owner_slot,
            order_type,
            padding: [0; 2],
            key,
            owner,
            quantity,
            client_order_id,
            best_initial,
            timestamp,
        }
    }
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
struct FreeNode {
    tag: u32,
    next: u32,
    padding: [u8; NODE_SIZE - 8],
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct AnyNode {
    pub tag: u32,
    pub data: [u8; NODE_SIZE - 4],
}

const_assert_eq!(size_of::<AnyNode>(), size_of::<InnerNode>());
const_assert_eq!(size_of::<AnyNode>(), size_of::<LeafNode>());
const_assert_eq!(size_of::<AnyNode>(), size_of::<FreeNode>());

enum NodeRef<'a> {
    Inner(&'a InnerNode),
    Leaf(&'a LeafNode),
}

enum NodeRefMut<'a> {
    Inner(&'a mut InnerNode),
    Leaf(&'a mut LeafNode),
}

impl AnyNode {
    fn key(&self) -> Option<i128> {
        match self.case()? {
            NodeRef::Inner(inner) => Some(inner.key),
            NodeRef::Leaf(leaf) => Some(leaf.key),
        }
    }

    fn children(&self) -> Option<[u32; 2]> {
        match self.case().unwrap() {
            NodeRef::Inner(&InnerNode { children, .. }) => Some(children),
            NodeRef::Leaf(_) => None,
        }
    }

    fn case(&self) -> Option<NodeRef> {
        match NodeTag::try_from(self.tag) {
            Ok(NodeTag::InnerNode) => Some(NodeRef::Inner(cast_ref(self))),
            Ok(NodeTag::LeafNode) => Some(NodeRef::Leaf(cast_ref(self))),
            _ => None,
        }
    }

    fn case_mut(&mut self) -> Option<NodeRefMut> {
        match NodeTag::try_from(self.tag) {
            Ok(NodeTag::InnerNode) => Some(NodeRefMut::Inner(cast_mut(self))),
            Ok(NodeTag::LeafNode) => Some(NodeRefMut::Leaf(cast_mut(self))),
            _ => None,
        }
    }

    #[inline]
    pub fn as_leaf(&self) -> Option<&LeafNode> {
        match self.case() {
            Some(NodeRef::Leaf(leaf_ref)) => Some(leaf_ref),
            _ => None,
        }
    }

    #[inline]
    pub fn as_leaf_mut(&mut self) -> Option<&mut LeafNode> {
        match self.case_mut() {
            Some(NodeRefMut::Leaf(leaf_ref)) => Some(leaf_ref),
            _ => None,
        }
    }
}

impl AsRef<AnyNode> for InnerNode {
    fn as_ref(&self) -> &AnyNode {
        cast_ref(self)
    }
}

impl AsRef<AnyNode> for LeafNode {
    #[inline]
    fn as_ref(&self) -> &AnyNode {
        cast_ref(self)
    }
}

#[derive(
    Eq, PartialEq, Copy, Clone, TryFromPrimitive, IntoPrimitive, Debug, Serialize, Deserialize,
)]
#[repr(u8)]
#[serde(into = "u8", try_from = "u8")]
pub enum OrderType {
    Limit = 0,
    ImmediateOrCancel = 1,
    PostOnly = 2,
    Market = 3,
    PostOnlySlide = 4, // ***
}

#[derive(
    Eq, PartialEq, Copy, Clone, TryFromPrimitive, IntoPrimitive, Debug, Serialize, Deserialize,
)]
#[repr(u8)]
#[serde(into = "u8", try_from = "u8")]
pub enum Side {
    Bid = 0,
    Ask = 1,
}

pub const MAX_BOOK_NODES: usize = 1024; // NOTE: this cannot be larger than u32::MAX

#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct BookSide {
    pub meta_data: MetaData,

    pub bump_index: usize,
    pub free_list_len: usize,
    pub free_list_head: u32,
    pub root_node: u32,
    pub leaf_count: usize,
    pub nodes: [AnyNode; MAX_BOOK_NODES], // TODO make this variable length
}

impl BookSide {
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        perp_market: &PerpMarket,
    ) -> MangoResult<RefMut<'a, Self>> {
        check!(account.owner == program_id, MangoErrorCode::InvalidOwner)?;
        let state = Self::load_mut(account)?;
        check!(state.meta_data.is_initialized, MangoErrorCode::Default)?;

        match DataType::try_from(state.meta_data.data_type).unwrap() {
            DataType::Bids => check!(account.key == &perp_market.bids, MangoErrorCode::Default)?,
            DataType::Asks => check!(account.key == &perp_market.asks, MangoErrorCode::Default)?,
            _ => return Err(throw!()),
        }

        Ok(state)
    }

    pub fn load_and_init<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        data_type: DataType,
        rent: &Rent,
    ) -> MangoResult<RefMut<'a, Self>> {
        // NOTE: check this first so we can borrow account later
        check!(
            rent.is_exempt(account.lamports(), account.data_len()),
            MangoErrorCode::AccountNotRentExempt
        )?;

        let mut state = Self::load_mut(account)?;
        check!(account.owner == program_id, MangoErrorCode::InvalidOwner)?;
        check!(!state.meta_data.is_initialized, MangoErrorCode::Default)?;
        state.meta_data = MetaData::new(data_type, 0, true);
        Ok(state)
    }

    fn get_mut(&mut self, key: u32) -> Option<&mut AnyNode> {
        let node = &mut self.nodes[key as usize];
        let tag = NodeTag::try_from(node.tag);
        match tag {
            Ok(NodeTag::InnerNode) | Ok(NodeTag::LeafNode) => Some(node),
            _ => None,
        }
    }
    fn get(&self, key: u32) -> Option<&AnyNode> {
        let node = &self.nodes[key as usize];
        let tag = NodeTag::try_from(node.tag);
        match tag {
            Ok(NodeTag::InnerNode) | Ok(NodeTag::LeafNode) => Some(node),
            _ => None,
        }
    }
    pub fn find_max(&self) -> Option<NodeHandle> {
        self.find_min_max(true)
    }
    fn root(&self) -> Option<NodeHandle> {
        if self.leaf_count == 0 {
            None
        } else {
            Some(self.root_node)
        }
    }

    pub fn find_min(&self) -> Option<NodeHandle> {
        self.find_min_max(false)
    }

    #[cfg(test)]
    fn to_price_quantity_vec(&self, reverse: bool) -> Vec<(i64, i64)> {
        let mut pqs = vec![];
        let mut current: NodeHandle = match self.root() {
            None => return pqs,
            Some(node_handle) => node_handle,
        };

        let left = reverse as usize;
        let right = !reverse as usize;
        let mut stack = vec![];
        loop {
            let root_contents = self.get(current).unwrap(); // should never fail unless book is already fucked
            match root_contents.case().unwrap() {
                NodeRef::Inner(inner) => {
                    stack.push(inner);
                    current = inner.children[left];
                }
                NodeRef::Leaf(leaf) => {
                    // if you hit leaf then pop stack and go right
                    // all inner nodes on stack have already been visited to the left
                    pqs.push((leaf.price(), leaf.quantity));
                    match stack.pop() {
                        None => return pqs,
                        Some(inner) => {
                            current = inner.children[right];
                        }
                    }
                }
            }
        }
    }

    fn find_min_max(&self, find_max: bool) -> Option<NodeHandle> {
        let mut root: NodeHandle = self.root()?;

        let i = if find_max { 1 } else { 0 };
        loop {
            let root_contents = self.get(root).unwrap();
            match root_contents.case().unwrap() {
                NodeRef::Inner(&InnerNode { children, .. }) => {
                    root = children[i];
                }
                _ => return Some(root),
            }
        }
    }

    pub fn get_min(&self) -> Option<&LeafNode> {
        self.get_min_max(false)
    }

    pub fn get_max(&self) -> Option<&LeafNode> {
        self.get_min_max(true)
    }
    fn get_min_max(&self, find_max: bool) -> Option<&LeafNode> {
        let mut root: NodeHandle = self.root()?;

        let i = if find_max { 1 } else { 0 };
        loop {
            let root_contents = self.get(root)?;
            match root_contents.case()? {
                NodeRef::Inner(inner) => {
                    root = inner.children[i];
                }
                NodeRef::Leaf(leaf) => {
                    return Some(leaf);
                }
            }
        }
    }

    pub fn remove_min(&mut self) -> Option<LeafNode> {
        self.remove_by_key(self.get(self.find_min()?)?.key()?)
    }

    pub fn remove_max(&mut self) -> Option<LeafNode> {
        self.remove_by_key(self.get(self.find_max()?)?.key()?)
    }

    fn remove_by_key(&mut self, search_key: i128) -> Option<LeafNode> {
        let mut parent_h = self.root()?;
        let mut child_h;
        let mut crit_bit;
        match self.get(parent_h).unwrap().case().unwrap() {
            NodeRef::Leaf(&leaf) if leaf.key == search_key => {
                assert_eq!(self.leaf_count, 1);
                self.root_node = 0;
                self.leaf_count = 0;
                let _old_root = self.remove(parent_h).unwrap();
                return Some(leaf);
            }
            NodeRef::Leaf(_) => return None,
            NodeRef::Inner(inner) => {
                let (ch, cb) = inner.walk_down(search_key);
                child_h = ch;
                crit_bit = cb;
            }
        }
        loop {
            match self.get(child_h).unwrap().case().unwrap() {
                NodeRef::Inner(inner) => {
                    let (grandchild_h, grandchild_crit_bit) = inner.walk_down(search_key);
                    parent_h = child_h;
                    child_h = grandchild_h;
                    crit_bit = grandchild_crit_bit;
                    continue;
                }
                NodeRef::Leaf(&leaf) => {
                    if leaf.key != search_key {
                        return None;
                    }

                    break;
                }
            }
        }
        // replace parent with its remaining child node
        // free child_h, replace *parent_h with *other_child_h, free other_child_h
        let other_child_h = self.get(parent_h).unwrap().children().unwrap()[!crit_bit as usize];
        let other_child_node_contents = self.remove(other_child_h).unwrap();
        *self.get_mut(parent_h).unwrap() = other_child_node_contents;
        self.leaf_count -= 1;
        Some(cast(self.remove(child_h).unwrap()))
    }

    fn remove(&mut self, key: u32) -> Option<AnyNode> {
        let val = *self.get(key)?;

        self.nodes[key as usize] = cast(FreeNode {
            tag: if self.free_list_len == 0 {
                NodeTag::LastFreeNode.into()
            } else {
                NodeTag::FreeNode.into()
            },
            next: self.free_list_head,
            padding: [0; 80],
        });

        self.free_list_len += 1;
        self.free_list_head = key;
        Some(val)
    }

    fn insert(&mut self, val: &AnyNode) -> MangoResult<u32> {
        match NodeTag::try_from(val.tag) {
            Ok(NodeTag::InnerNode) | Ok(NodeTag::LeafNode) => (),
            _ => unreachable!(),
        };

        if self.free_list_len == 0 {
            check!(
                self.bump_index < self.nodes.len() && self.bump_index < (u32::MAX as usize),
                MangoErrorCode::OutOfSpace
            )?;

            self.nodes[self.bump_index] = *val;
            let key = self.bump_index as u32;
            self.bump_index += 1;
            return Ok(key);
        }

        let key = self.free_list_head;
        let node = &mut self.nodes[key as usize];

        // TODO OPT possibly unnecessary check here - remove if we need compute
        match NodeTag::try_from(node.tag) {
            Ok(NodeTag::FreeNode) => assert!(self.free_list_len > 1),
            Ok(NodeTag::LastFreeNode) => assert_eq!(self.free_list_len, 1),
            _ => unreachable!(),
        };

        let next_free_list_head: u32;
        {
            let free_list_item: &FreeNode = cast_ref(node);
            next_free_list_head = free_list_item.next;
        }
        self.free_list_head = next_free_list_head;
        self.free_list_len -= 1;
        *node = *val;
        Ok(key)
    }
    pub fn insert_leaf(
        &mut self,
        new_leaf: &LeafNode,
    ) -> MangoResult<(NodeHandle, Option<LeafNode>)> {
        let mut root: NodeHandle = match self.root() {
            Some(h) => h,
            None => {
                // create a new root if none exists
                let handle = self.insert(new_leaf.as_ref())?;
                self.root_node = handle;
                self.leaf_count = 1;
                return Ok((handle, None));
            }
        };
        loop {
            // check if the new node will be a child of the root
            let root_contents = *self.get(root).unwrap();
            let root_key = root_contents.key().unwrap();
            if root_key == new_leaf.key {
                // This should never happen because key should never match
                if let Some(NodeRef::Leaf(&old_root_as_leaf)) = root_contents.case() {
                    // clobber the existing leaf
                    *self.get_mut(root).unwrap() = *new_leaf.as_ref();
                    return Ok((root, Some(old_root_as_leaf)));
                }
            }
            let shared_prefix_len: u32 = (root_key ^ new_leaf.key).leading_zeros();
            match root_contents.case() {
                None => unreachable!(),
                Some(NodeRef::Inner(inner)) => {
                    let keep_old_root = shared_prefix_len >= inner.prefix_len;
                    if keep_old_root {
                        root = inner.walk_down(new_leaf.key).0;
                        continue;
                    };
                }
                _ => (),
            };
            // implies root is a Leaf or Inner where shared_prefix_len < prefix_len

            // change the root in place to represent the LCA of [new_leaf] and [root]
            let crit_bit_mask: i128 = 1i128 << (127 - shared_prefix_len);
            let new_leaf_crit_bit = (crit_bit_mask & new_leaf.key) != 0;
            let old_root_crit_bit = !new_leaf_crit_bit;

            let new_leaf_handle = self.insert(new_leaf.as_ref())?;
            let moved_root_handle = match self.insert(&root_contents) {
                Ok(h) => h,
                Err(e) => {
                    self.remove(new_leaf_handle).unwrap();
                    return Err(e);
                }
            };

            let new_root: &mut InnerNode = cast_mut(self.get_mut(root).unwrap());
            *new_root = InnerNode::new(shared_prefix_len, new_leaf.key);

            new_root.children[new_leaf_crit_bit as usize] = new_leaf_handle;
            new_root.children[old_root_crit_bit as usize] = moved_root_handle;
            self.leaf_count += 1;
            return Ok((new_leaf_handle, None));
        }
    }

    pub fn is_full(&self) -> bool {
        self.free_list_len == 0 && self.bump_index == self.nodes.len()
    }
}

pub struct Book<'a> {
    bids: RefMut<'a, BookSide>,
    asks: RefMut<'a, BookSide>,
}

impl<'a> Book<'a> {
    pub fn load_checked(
        program_id: &Pubkey,
        bids_ai: &'a AccountInfo,
        asks_ai: &'a AccountInfo,
        perp_market: &PerpMarket,
    ) -> MangoResult<Self> {
        Ok(Self {
            bids: BookSide::load_mut_checked(bids_ai, program_id, perp_market)?,
            asks: BookSide::load_mut_checked(asks_ai, program_id, perp_market)?,
        })
    }

    fn get_best_bid_handle(&self) -> Option<NodeHandle> {
        self.bids.find_max()
    }

    pub fn get_best_bid_price(&self) -> Option<i64> {
        Some(self.bids.get_max()?.price())
    }

    fn get_best_ask_handle(&self) -> Option<NodeHandle> {
        self.asks.find_min()
    }

    pub fn get_best_ask_price(&self) -> Option<i64> {
        Some(self.asks.get_min()?.price())
    }

    #[inline(never)]
    pub fn new_order(
        &mut self,
        event_queue: &mut EventQueue,
        market: &mut PerpMarket,
        info: &PerpMarketInfo,
        mango_account: &mut MangoAccount,
        mango_account_pk: &Pubkey,
        market_index: usize,
        side: Side,
        price: i64,
        quantity: i64, // quantity is guaranteed to be greater than zero due to initial check --
        order_type: OrderType,
        client_order_id: u64,
        now_ts: u64,
    ) -> MangoResult<()> {
        match side {
            Side::Bid => self.new_bid(
                event_queue,
                market,
                info,
                mango_account,
                mango_account_pk,
                market_index,
                price,
                quantity,
                order_type,
                client_order_id,
                now_ts,
            ),
            Side::Ask => self.new_ask(
                event_queue,
                market,
                info,
                mango_account,
                mango_account_pk,
                market_index,
                price,
                quantity,
                order_type,
                client_order_id,
                now_ts,
            ),
        }
    }

    /// Iterate over the book and return
    /// return changes to (taker_base, taker_quote, bids_quantity, asks_quantity)
    pub fn sim_new_bid(
        &self,
        price: i64,
        quantity: i64, // quantity is guaranteed to be greater than zero due to initial check --
        order_type: OrderType,
    ) -> MangoResult<(i64, i64, i64, i64)> {
        let (mut taker_base, mut taker_quote, mut bids_quantity, asks_quantity) = (0, 0, 0, 0);

        let (post_only, post_allowed, price) = match order_type {
            OrderType::Limit => (false, true, price),
            OrderType::ImmediateOrCancel => (false, false, price),
            OrderType::PostOnly => (true, true, price),
            OrderType::Market => (false, false, i64::MAX),
            OrderType::PostOnlySlide => {
                let price = if let Some(best_ask_price) = self.get_best_ask_price() {
                    price.min(best_ask_price.checked_sub(1).ok_or(math_err!())?)
                } else {
                    price
                };
                (true, true, price)
            }
        };

        let mut rem_quantity = quantity; // base lots (aka contracts)
        let mut stack = vec![];
        let mut current = match self.asks.root() {
            None => {
                if rem_quantity > 0 && post_allowed {
                    bids_quantity += rem_quantity;
                }
                return Ok((taker_base, taker_quote, bids_quantity, asks_quantity));
            }
            Some(node_handle) => node_handle,
        };
        while rem_quantity > 0 {
            match self.asks.get(current).ok_or(throw!())?.case().ok_or(throw!())? {
                NodeRef::Inner(inner) => {
                    stack.push(inner);
                    current = inner.children[0];
                }
                NodeRef::Leaf(best_ask) => {
                    let best_ask_price = best_ask.price();
                    if price < best_ask_price {
                        break;
                    } else if post_only {
                        return Ok((taker_base, taker_quote, bids_quantity, asks_quantity));
                    }

                    let match_quantity = rem_quantity.min(best_ask.quantity);
                    rem_quantity -= match_quantity;

                    taker_base += match_quantity;
                    taker_quote -= match_quantity * best_ask_price;

                    match stack.pop() {
                        // if no more inner nodes on stack, we've processed whole book
                        None => {
                            break;
                        }
                        Some(inner) => {
                            current = inner.children[1];
                        }
                    }
                }
            }
        }

        if rem_quantity > 0 && post_allowed {
            bids_quantity += rem_quantity;
        }
        Ok((taker_base, taker_quote, bids_quantity, asks_quantity))
    }

    pub fn sim_new_ask(
        &self,
        price: i64,
        quantity: i64, // quantity is guaranteed to be greater than zero due to initial check --
        order_type: OrderType,
    ) -> MangoResult<(i64, i64, i64, i64)> {
        let (mut taker_base, mut taker_quote, bids_quantity, mut asks_quantity) = (0, 0, 0, 0);

        let (post_only, post_allowed, price) = match order_type {
            OrderType::Limit => (false, true, price),
            OrderType::ImmediateOrCancel => (false, false, price),
            OrderType::PostOnly => (true, true, price),
            OrderType::Market => (false, false, 0),
            OrderType::PostOnlySlide => {
                let price = if let Some(best_bid_price) = self.get_best_bid_price() {
                    price.max(best_bid_price.checked_add(1).ok_or(math_err!())?)
                } else {
                    price
                };
                (true, true, price)
            }
        };

        let mut rem_quantity = quantity; // base lots (aka contracts)
        let mut stack = vec![];
        let mut current = match self.bids.root() {
            None => {
                if rem_quantity > 0 && post_allowed {
                    asks_quantity += rem_quantity;
                }
                return Ok((taker_base, taker_quote, bids_quantity, asks_quantity));
            }
            Some(node_handle) => node_handle,
        };
        while rem_quantity > 0 {
            match self.bids.get(current).ok_or(throw!())?.case().ok_or(throw!())? {
                NodeRef::Inner(inner) => {
                    stack.push(inner);
                    current = inner.children[1];
                }
                NodeRef::Leaf(best_bid) => {
                    let best_bid_price = best_bid.price();
                    if price > best_bid_price {
                        break;
                    } else if post_only {
                        return Ok((taker_base, taker_quote, bids_quantity, asks_quantity));
                    }

                    let match_quantity = rem_quantity.min(best_bid.quantity);
                    rem_quantity -= match_quantity;

                    taker_base -= match_quantity;
                    taker_quote += match_quantity * best_bid_price;

                    match stack.pop() {
                        // if no more inner nodes on stack, we've processed whole book
                        None => {
                            break;
                        }
                        Some(inner) => {
                            current = inner.children[0];
                        }
                    }
                }
            }
        }

        if rem_quantity > 0 && post_allowed {
            asks_quantity += rem_quantity;
        }
        Ok((taker_base, taker_quote, bids_quantity, asks_quantity))
    }

    #[inline(never)]
    fn new_bid(
        &mut self,
        event_queue: &mut EventQueue,
        market: &mut PerpMarket,
        info: &PerpMarketInfo,
        mango_account: &mut MangoAccount,
        mango_account_pk: &Pubkey,
        market_index: usize,
        price: i64,
        quantity: i64, // quantity is guaranteed to be greater than zero due to initial check --
        order_type: OrderType,
        client_order_id: u64,
        now_ts: u64,
    ) -> MangoResult<()> {
        // TODO proper error handling
        // TODO handle the case where we run out of compute (right now just fails)
        let (post_only, post_allowed, price) = match order_type {
            OrderType::Limit => (false, true, price),
            OrderType::ImmediateOrCancel => (false, false, price),
            OrderType::PostOnly => (true, true, price),
            OrderType::Market => (false, false, i64::MAX),
            OrderType::PostOnlySlide => {
                let price = if let Some(best_ask_price) = self.get_best_ask_price() {
                    price.min(best_ask_price.checked_sub(1).ok_or(math_err!())?)
                } else {
                    price
                };
                (true, true, price)
            }
        };

        let order_id = market.gen_order_id(Side::Bid, price);

        // if post only and price >= best_ask, return
        // Iterate through book and match against this new bid
        let mut rem_quantity = quantity; // base lots (aka contracts)
        while rem_quantity > 0 {
            let best_ask_h = match self.get_best_ask_handle() {
                None => break,
                Some(h) => h,
            };

            let best_ask = self.asks.get_mut(best_ask_h).unwrap().as_leaf_mut().unwrap();
            let best_ask_price = best_ask.price();

            if price < best_ask_price {
                break;
            } else if post_only {
                msg!("Order could not be placed due to PostOnly");
                return Ok(()); // return silently to not fail other instructions in tx
                               // return Err(throw_err!(MangoErrorCode::PostOnly));
            }

            let match_quantity = rem_quantity.min(best_ask.quantity);
            rem_quantity -= match_quantity;
            best_ask.quantity -= match_quantity;

            mango_account.perp_accounts[market_index]
                .add_taker_trade(match_quantity, -match_quantity * best_ask_price);
            let maker_out = best_ask.quantity == 0;
            let fill = FillEvent::new(
                Side::Bid,
                best_ask.owner_slot,
                maker_out,
                now_ts,
                event_queue.header.seq_num,
                best_ask.owner,
                best_ask.key,
                best_ask.client_order_id,
                info.maker_fee,
                best_ask.best_initial,
                best_ask.timestamp,
                *mango_account_pk,
                order_id,
                client_order_id,
                info.taker_fee,
                best_ask_price,
                match_quantity,
            );
            event_queue.push_back(cast(fill)).unwrap();

            // now either best_ask.quantity == 0 or rem_quantity == 0 or both
            if best_ask.quantity == 0 {
                // Remove the order from the book
                let key = best_ask.key;
                let _removed_node = self.asks.remove_by_key(key).unwrap();
            }
        }

        // If there are still quantity unmatched, place on the book
        if rem_quantity > 0 && post_allowed {
            if self.bids.is_full() {
                // If this bid is higher than lowest bid, boot that bid and insert this one
                let min_bid = self.bids.remove_min().unwrap();
                check!(price > min_bid.price(), MangoErrorCode::OutOfSpace)?;
                let event = OutEvent::new(
                    Side::Bid,
                    min_bid.owner_slot,
                    now_ts,
                    event_queue.header.seq_num,
                    min_bid.owner,
                    min_bid.quantity,
                );
                event_queue.push_back(cast(event)).unwrap();
            }

            let best_initial = match self.get_best_bid_price() {
                None => price,
                Some(p) => p,
            };

            let owner_slot = mango_account
                .next_order_slot()
                .ok_or(throw_err!(MangoErrorCode::TooManyOpenOrders))?;
            let new_bid = LeafNode::new(
                owner_slot as u8,
                order_id,
                *mango_account_pk,
                rem_quantity,
                client_order_id,
                now_ts,
                best_initial,
                order_type,
            );
            let _result = self.bids.insert_leaf(&new_bid)?;

            // TODO OPT remove if PlacePerpOrder needs more compute
            msg!(
                "bid on book client_id={} quantity={} price={}",
                client_order_id,
                rem_quantity,
                price
            );

            mango_account.add_order(market_index, Side::Bid, &new_bid)?;
        }

        Ok(())
    }

    #[inline(never)]
    pub fn new_ask(
        &mut self,
        event_queue: &mut EventQueue,
        market: &mut PerpMarket,
        info: &PerpMarketInfo,
        mango_account: &mut MangoAccount,
        mango_account_pk: &Pubkey,
        market_index: usize,
        price: i64,
        quantity: i64, // quantity is guaranteed to be greater than zero due to initial check --
        order_type: OrderType,
        client_order_id: u64,
        now_ts: u64,
    ) -> MangoResult<()> {
        // TODO proper error handling
        let (post_only, post_allowed, price) = match order_type {
            OrderType::Limit => (false, true, price),
            OrderType::ImmediateOrCancel => (false, false, price),
            OrderType::PostOnly => (true, true, price),
            OrderType::Market => (false, false, 0),
            OrderType::PostOnlySlide => {
                let price = if let Some(best_bid_price) = self.get_best_bid_price() {
                    price.max(best_bid_price.checked_add(1).ok_or(math_err!())?)
                } else {
                    price
                };
                (true, true, price)
            }
        };
        let order_id = market.gen_order_id(Side::Ask, price);

        // if post only and price >= best_ask, return
        // Iterate through book and match against this new bid
        let mut rem_quantity = quantity; // base lots (aka contracts)
        while rem_quantity > 0 {
            let best_bid_h = match self.get_best_bid_handle() {
                None => break,
                Some(h) => h,
            };

            let best_bid = self.bids.get_mut(best_bid_h).unwrap().as_leaf_mut().unwrap();
            let best_bid_price = best_bid.price();

            if price > best_bid_price {
                break;
            } else if post_only {
                msg!("Order could not be placed due to PostOnly");
                return Ok(()); // return silently to not fail other instructions in tx
            }

            let match_quantity = rem_quantity.min(best_bid.quantity);
            rem_quantity -= match_quantity;
            best_bid.quantity -= match_quantity;
            mango_account.perp_accounts[market_index]
                .add_taker_trade(-match_quantity, match_quantity * best_bid_price);
            let maker_out = best_bid.quantity == 0;

            let fill = FillEvent::new(
                Side::Ask,
                best_bid.owner_slot,
                maker_out,
                now_ts,
                event_queue.header.seq_num,
                best_bid.owner,
                best_bid.key,
                best_bid.client_order_id,
                info.maker_fee,
                best_bid.best_initial,
                best_bid.timestamp,
                *mango_account_pk,
                order_id,
                client_order_id,
                info.taker_fee,
                best_bid_price,
                match_quantity,
            );

            event_queue.push_back(cast(fill)).unwrap();

            // now either best_bid.quantity == 0 or rem_quantity == 0 or both
            if best_bid.quantity == 0 {
                // Remove the order from the book
                let key = best_bid.key;
                let _removed_node = self.bids.remove_by_key(key).unwrap();
            }
        }

        // If there are still quantity unmatched, place on the book
        if rem_quantity > 0 && post_allowed {
            if self.asks.is_full() {
                // If this asks is lower than highest ask, boot that ask and insert this one
                let max_ask = self.asks.remove_max().unwrap();
                check!(price < max_ask.price(), MangoErrorCode::OutOfSpace)?;
                let event = OutEvent::new(
                    Side::Ask,
                    max_ask.owner_slot,
                    now_ts,
                    event_queue.header.seq_num,
                    max_ask.owner,
                    max_ask.quantity,
                );
                event_queue.push_back(cast(event)).unwrap();
            }
            let best_initial = match self.get_best_ask_price() {
                None => price,
                Some(p) => p,
            };

            let owner_slot = mango_account
                .next_order_slot()
                .ok_or(throw_err!(MangoErrorCode::TooManyOpenOrders))?;
            let new_ask = LeafNode::new(
                owner_slot as u8,
                order_id,
                *mango_account_pk,
                rem_quantity,
                client_order_id,
                now_ts,
                best_initial,
                order_type,
            );

            // TODO OPT remove if PlacePerpOrder needs more compute
            msg!(
                "ask on book client_id={} quantity={} price={}",
                client_order_id,
                rem_quantity,
                price
            );

            let _result = self.asks.insert_leaf(&new_ask)?;
            mango_account.add_order(market_index, Side::Ask, &new_ask)?;
        }

        Ok(())
    }

    pub fn cancel_order(&mut self, order_id: i128, side: Side) -> MangoResult<LeafNode> {
        match side {
            Side::Bid => {
                self.bids.remove_by_key(order_id).ok_or(throw_err!(MangoErrorCode::InvalidOrderId))
            }
            Side::Ask => {
                self.asks.remove_by_key(order_id).ok_or(throw_err!(MangoErrorCode::InvalidOrderId))
            }
        }
    }

    /// Used by force cancel so does not need to give liquidity incentives
    pub fn cancel_all(
        &mut self,
        mango_account: &mut MangoAccount,
        market_index: usize,
        mut limit: u8,
    ) -> MangoResult<()> {
        let market_index = market_index as u8;
        for i in 0..MAX_PERP_OPEN_ORDERS {
            if mango_account.order_market[i] != market_index {
                // means slot is free or belongs to different perp market
                continue;
            }
            let order_id = mango_account.orders[i];
            match self.cancel_order(order_id, mango_account.order_side[i]) {
                Ok(order) => {
                    mango_account.remove_order(order.owner_slot as usize, order.quantity)?;
                }
                Err(_) => {
                    // If it's not on the book, then it has been matched and only Keeper can remove
                }
            };

            limit -= 1;
            if limit == 0 {
                break;
            }
        }
        Ok(())
    }

    pub fn cancel_all_with_incentives(
        &mut self,
        mango_account: &mut MangoAccount,
        perp_market: &mut PerpMarket,
        market_index: usize,
        mut limit: u8,
    ) -> MangoResult<()> {
        let now_ts = Clock::get()?.unix_timestamp as u64;

        for i in 0..MAX_PERP_OPEN_ORDERS {
            if mango_account.order_market[i] != market_index as u8 {
                // means slot is free or belongs to different perp market
                continue;
            }
            let order_id = mango_account.orders[i];
            let order_side = mango_account.order_side[i];

            let best_final = match order_side {
                Side::Bid => self.get_best_bid_price().unwrap(),
                Side::Ask => self.get_best_ask_price().unwrap(),
            };

            match self.cancel_order(order_id, order_side) {
                Ok(order) => {
                    // technically these should be the same. Can enable this check to be extra sure
                    // check!(i == order.owner_slot as usize, MathError)?;
                    mango_account.remove_order(order.owner_slot as usize, order.quantity)?;
                    mango_account.perp_accounts[market_index].apply_incentives(
                        perp_market,
                        order_side,
                        order.price(),
                        order.best_initial,
                        best_final,
                        order.timestamp,
                        now_ts,
                        order.quantity,
                    )?;
                }
                Err(_) => {
                    // If it's not on the book, then it has been matched and only Keeper can remove
                }
            };

            limit -= 1;
            if limit == 0 {
                break;
            }
        }
        Ok(())
    }
}
