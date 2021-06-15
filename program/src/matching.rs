use crate::error::{check_assert, MerpsError, MerpsErrorCode, MerpsResult, SourceFileId};
use crate::queue::{EventQueue, FillEvent, OutEvent};
use crate::state::{DataType, MerpsAccount, MetaData, PerpMarket, PerpOpenOrders};
use bytemuck::{cast, cast_mut, cast_ref, Zeroable};
use fixed::types::I80F48;
use mango_common::Loadable;
use mango_macro::{Loadable, Pod};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{Deserialize, Serialize};
use solana_program::account_info::AccountInfo;
use solana_program::msg;
use solana_program::pubkey::Pubkey;
use solana_program::sysvar::rent::Rent;
use std::cell::RefMut;
use std::convert::TryFrom;
use std::ops::DerefMut;

declare_check_assert_macros!(SourceFileId::Matching);
pub type NodeHandle = u32;
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
    pub padding: [u8; 40],
}

impl InnerNode {
    fn walk_down(&self, search_key: i128) -> (NodeHandle, bool) {
        let crit_bit_mask = (1i128 << 127) >> self.prefix_len;
        let crit_bit = (search_key & crit_bit_mask) != 0;
        (self.children[crit_bit as usize], crit_bit)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Pod)]
#[repr(C)]
pub struct LeafNode {
    pub tag: u32,
    pub owner_slot: u8,
    pub padding: [u8; 3],
    pub key: i128,
    pub owner: Pubkey,
    pub quantity: i64,
    pub client_order_id: u64,
}

impl LeafNode {
    pub fn price(&self) -> i64 {
        (self.key >> 64) as i64
    }
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
struct FreeNode {
    tag: u32,
    next: u32,
    padding: [u8; 64],
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
pub struct AnyNode {
    pub tag: u32,
    pub data: [u8; 68],
}

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
pub enum OrderType {
    Limit = 0,
    ImmediateOrCancel = 1,
    PostOnly = 2,
}

#[derive(
    Eq, PartialEq, Copy, Clone, TryFromPrimitive, IntoPrimitive, Debug, Serialize, Deserialize,
)]
#[repr(u8)]
pub enum Side {
    Bid = 0,
    Ask = 1,
}

pub const MAX_BOOK_NODES: usize = 1024; // NOTE: this cannot be larger than u32::MAX

#[derive(Copy, Clone, Pod, Loadable)]
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
    ) -> MerpsResult<RefMut<'a, Self>> {
        check!(account.owner == program_id, MerpsErrorCode::InvalidOwner)?;
        let state = Self::load_mut(account)?;
        check!(state.meta_data.is_initialized, MerpsErrorCode::Default)?;

        match DataType::try_from(state.meta_data.data_type).unwrap() {
            DataType::Bids => check!(account.key == &perp_market.bids, MerpsErrorCode::Default)?,
            DataType::Asks => check!(account.key == &perp_market.asks, MerpsErrorCode::Default)?,
            _ => return Err(throw!()),
        }

        Ok(state)
    }

    pub fn load_and_init<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        data_type: DataType,
        rent: &Rent,
    ) -> MerpsResult<RefMut<'a, Self>> {
        // NOTE: check this first so we can borrow account later
        check!(
            rent.is_exempt(account.lamports(), account.data_len()),
            MerpsErrorCode::AccountNotRentExempt
        )?;

        let mut state = Self::load_mut(account)?;
        check!(account.owner == program_id, MerpsErrorCode::InvalidOwner)?;
        check!(!state.meta_data.is_initialized, MerpsErrorCode::Default)?;
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
            padding: Zeroable::zeroed(),
        });

        self.free_list_len += 1;
        self.free_list_head = key;
        Some(val)
    }

    fn insert(&mut self, val: &AnyNode) -> MerpsResult<u32> {
        match NodeTag::try_from(val.tag) {
            Ok(NodeTag::InnerNode) | Ok(NodeTag::LeafNode) => (),
            _ => unreachable!(),
        };

        if self.free_list_len == 0 {
            check!(
                self.bump_index < self.nodes.len() && self.bump_index < (u32::MAX as usize),
                MerpsErrorCode::OutOfSpace
            )?;

            self.nodes[self.bump_index] = *val;
            let key = self.bump_index as u32;
            self.bump_index += 1;
            return Ok(key);
        }

        let key = self.free_list_head;
        let node = &mut self.nodes[key as usize];

        // TODO: possibly unnecessary check here - remove if we need compute
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
    ) -> MerpsResult<(NodeHandle, Option<LeafNode>)> {
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
            let crit_bit_mask: i128 = (1i128 << 127) >> shared_prefix_len;
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
            *new_root = InnerNode {
                tag: NodeTag::InnerNode.into(),
                prefix_len: shared_prefix_len,
                key: new_leaf.key,
                children: [0; 2],
                padding: [0u8; 40],
            };

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
    ) -> MerpsResult<Self> {
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

    pub fn new_order(
        &mut self,
        event_queue: &mut EventQueue,
        market: &mut PerpMarket,
        merps_account: &mut MerpsAccount,
        merps_account_pk: &Pubkey,
        market_index: usize,
        side: Side,
        price: i64,
        quantity: i64, // quantity is guaranteed to be greater than zero due to initial check --
        order_type: OrderType,
        client_order_id: u64,
    ) -> MerpsResult<()> {
        match side {
            Side::Bid => self.new_bid(
                event_queue,
                market,
                merps_account,
                merps_account_pk,
                market_index,
                price,
                quantity,
                order_type,
                client_order_id,
            ),
            Side::Ask => self.new_ask(
                event_queue,
                market,
                merps_account,
                merps_account_pk,
                market_index,
                price,
                quantity,
                order_type,
                client_order_id,
            ),
        }
    }

    fn new_bid(
        &mut self,
        event_queue: &mut EventQueue,
        market: &mut PerpMarket,
        merps_account: &mut MerpsAccount,
        merps_account_pk: &Pubkey,
        market_index: usize,
        price: i64,
        quantity: i64, // quantity is guaranteed to be greater than zero due to initial check --
        order_type: OrderType,
        client_order_id: u64,
    ) -> MerpsResult<()> {
        // TODO make use of the order options
        // TODO proper error handling
        // TODO handle the case where we run out of compute
        #[allow(unused_variables)]
        let (post_only, post_allowed) = match order_type {
            OrderType::Limit => (false, true),
            OrderType::ImmediateOrCancel => unimplemented!(),
            OrderType::PostOnly => unimplemented!(),
        };
        let order_id = market.gen_order_id(Side::Bid, price);

        // if post only and price >= best_ask, return
        // Iterate through book and match against this new bid
        let mut rem_quantity = quantity; // base lots (aka contracts)
        let mut quote_used = 0; // quote lots
        while rem_quantity > 0 {
            let best_ask_h = match self.get_best_ask_handle() {
                None => {
                    break;
                }
                Some(h) => h,
            };

            let best_ask = self.asks.get_mut(best_ask_h).unwrap().as_leaf_mut().unwrap();
            let best_ask_price = best_ask.price();

            msg!("new_ask p={} bap={}", price, best_ask_price);

            if price < best_ask_price {
                break;
            }

            let match_quantity = rem_quantity.min(best_ask.quantity);
            rem_quantity -= match_quantity;
            let quote_change = match_quantity * best_ask_price;
            quote_used += quote_change;
            best_ask.quantity -= match_quantity;

            let maker_fill = FillEvent::new(
                true,
                best_ask.owner,
                -match_quantity,
                quote_change,
                market.long_funding,
                market.short_funding,
            );
            event_queue.push_back(cast(maker_fill)).unwrap();

            // This fill is not necessary, purely for stats purposes
            let taker_fill = FillEvent::new(
                false,
                *merps_account_pk,
                match_quantity,
                -quote_change,
                market.long_funding,
                market.short_funding,
            );
            event_queue.push_back(cast(taker_fill)).unwrap();

            // now either best_ask.quantity == 0 or rem_quantity == 0 or both
            if best_ask.quantity == 0 {
                // Create an Out event
                let event = OutEvent::new(Side::Ask, best_ask.owner_slot, 0, best_ask.owner);
                event_queue.push_back(cast(event)).unwrap();
                // Remove the order from the book
                let key = best_ask.key;
                let _removed_node = self.asks.remove_by_key(key).unwrap();
            }
        }

        // If there are still quantity unmatched, place on the book
        if rem_quantity > 0 {
            if self.bids.is_full() {
                // If this bid is higher than lowest bid, boot that bid and insert this one
                let min_bid_handle = self.bids.find_min().unwrap();
                let min_bid = self.bids.get(min_bid_handle).unwrap().as_leaf().unwrap();
                check!(price > min_bid.price(), MerpsErrorCode::OutOfSpace)?;
                let event =
                    OutEvent::new(Side::Bid, min_bid.owner_slot, min_bid.quantity, min_bid.owner);
                event_queue.push_back(cast(event)).unwrap();

                let _removed_node = self.bids.remove(min_bid_handle).unwrap();
            }

            let oo = &mut merps_account.perp_accounts[market_index].open_orders;

            let new_bid = LeafNode {
                tag: NodeTag::LeafNode as u32,
                owner_slot: oo.next_order_slot(),
                padding: [0; 3],
                key: order_id,
                owner: *merps_account_pk,
                quantity: rem_quantity,
                client_order_id,
            };

            let _result = self.bids.insert_leaf(&new_bid)?;

            msg!(
                "bid on book client_id={} quantity={} price={}",
                client_order_id,
                rem_quantity,
                price
            );
            oo.add_order(Side::Bid, &new_bid)?;
        }

        // Edit merps_account if some contracts were matched
        if rem_quantity < quantity {
            let base_change = quantity - rem_quantity;
            merps_account.perp_accounts[market_index].change_position(
                base_change,
                I80F48::from_num(-quote_used * market.quote_lot_size),
                market.long_funding,
                market.short_funding,
            )?;

            msg!(
                "matched base={} quote={:?}",
                base_change,
                I80F48::from_num(-quote_used * market.quote_lot_size)
            );
        }

        Ok(())
    }

    // TODO implement self trade behavior
    pub fn new_ask(
        &mut self,
        event_queue: &mut EventQueue,
        market: &mut PerpMarket,
        merps_account: &mut MerpsAccount,
        merps_account_pk: &Pubkey,
        market_index: usize,
        price: i64,
        quantity: i64, // quantity is guaranteed to be greater than zero due to initial check --
        order_type: OrderType,
        client_order_id: u64,
    ) -> MerpsResult<()> {
        // TODO make use of the order options
        // TODO proper error handling
        #[allow(unused_variables)]
        let (post_only, post_allowed) = match order_type {
            OrderType::Limit => (false, true),
            OrderType::ImmediateOrCancel => unimplemented!(),
            OrderType::PostOnly => unimplemented!(),
        };
        let order_id = market.gen_order_id(Side::Ask, price);

        // if post only and price >= best_ask, return
        // Iterate through book and match against this new bid
        let mut rem_quantity = quantity; // base lots (aka contracts)
        let mut quote_used = 0; // quote lots
        while rem_quantity > 0 {
            let best_bid_h = match self.get_best_bid_handle() {
                None => {
                    break;
                }
                Some(h) => h,
            };

            let best_bid = self.bids.get_mut(best_bid_h).unwrap().as_leaf_mut().unwrap();
            let best_bid_price = best_bid.price();

            msg!("new_ask p={} bbp={}", price, best_bid_price);
            if price > best_bid_price {
                break;
            }

            let match_quantity = rem_quantity.min(best_bid.quantity);
            let quote_change = match_quantity * best_bid_price;
            rem_quantity -= match_quantity;
            quote_used += quote_change;
            best_bid.quantity -= match_quantity;

            let maker_fill = FillEvent::new(
                true,
                best_bid.owner,
                match_quantity,
                -quote_change,
                market.long_funding,
                market.short_funding,
            );
            event_queue.push_back(cast(maker_fill)).unwrap();

            // This fill is not necessary, purely for stats purposes
            let taker_fill = FillEvent::new(
                false,
                *merps_account_pk,
                -match_quantity,
                quote_change,
                market.long_funding,
                market.short_funding,
            );
            event_queue.push_back(cast(taker_fill)).unwrap();

            // now either best_bid.quantity == 0 or rem_quantity == 0 or both
            if best_bid.quantity == 0 {
                // Create an Out event
                let event = OutEvent::new(Side::Bid, best_bid.owner_slot, 0, best_bid.owner);
                event_queue.push_back(cast(event)).unwrap();
                // Remove the order from the book
                let key = best_bid.key;
                let _removed_node = self.bids.remove_by_key(key).unwrap();
            }
        }

        // If there are still quantity unmatched, place on the book
        if rem_quantity > 0 {
            if self.bids.is_full() {
                // If this asks is lower than highest ask, boot that ask and insert this one
                let max_ask_handle = self.asks.find_min().unwrap();
                let max_ask = self.asks.get(max_ask_handle).unwrap().as_leaf().unwrap();
                check!(price < max_ask.price(), MerpsErrorCode::OutOfSpace)?;
                let event =
                    OutEvent::new(Side::Ask, max_ask.owner_slot, max_ask.quantity, max_ask.owner);
                event_queue.push_back(cast(event)).unwrap();
                let _removed_node = self.asks.remove(max_ask_handle).unwrap();
            }

            let oo = &mut merps_account.perp_accounts[market_index].open_orders;

            let new_ask = LeafNode {
                tag: NodeTag::LeafNode as u32,
                owner_slot: oo.next_order_slot(),
                padding: [0; 3],
                key: order_id,
                owner: *merps_account_pk,
                quantity: rem_quantity,
                client_order_id,
            };

            msg!(
                "ask on book client_id={} quantity={} price={}",
                client_order_id,
                rem_quantity,
                price
            );

            let _result = self.asks.insert_leaf(&new_ask)?;
            oo.add_order(Side::Ask, &new_ask)?;
        }

        // Edit merps_account if some contracts were matched
        if rem_quantity < quantity {
            let base_change = -(quantity - rem_quantity); // negative because short
            merps_account.perp_accounts[market_index].change_position(
                base_change,
                I80F48::from_num(quote_used * market.quote_lot_size),
                market.long_funding,
                market.short_funding,
            )?;

            msg!(
                "matched base={} quote={:?}",
                base_change,
                I80F48::from_num(-quote_used * market.quote_lot_size)
            );
        }

        Ok(())
    }

    pub fn cancel_order(
        &mut self,
        _event_queue: &mut EventQueue, // TODO remove
        oo: &mut PerpOpenOrders,
        merps_account_pk: &Pubkey,
        _market_index: usize, // TODO remove
        order_id: i128,
        side: Side,
    ) -> MerpsResult<()> {
        let book_side = match side {
            Side::Bid => self.bids.deref_mut(),
            Side::Ask => self.asks.deref_mut(),
        };

        let order = book_side.remove_by_key(order_id).ok_or(throw_err!(MerpsErrorCode::Default))?;
        check_eq!(order.owner, *merps_account_pk, MerpsErrorCode::Default)?;

        oo.cancel_order(&order, order_id, side)?;

        // TODO: write to event queue

        Ok(())
    }
}
