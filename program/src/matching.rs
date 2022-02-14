use std::cell::RefMut;
use std::convert::TryFrom;
use std::mem::size_of;

use bytemuck::{cast, cast_mut, cast_ref};
use fixed::types::I80F48;
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
use mango_logs::{mango_emit_stack, ReferralFeeAccrualLog};
use mango_macro::{Loadable, Pod};

use crate::error::{check_assert, MangoError, MangoErrorCode, MangoResult, SourceFileId};
use crate::ids::mngo_token;
use crate::queue::{EventQueue, FillEvent, OutEvent};
use crate::state::{
    DataType, MangoAccount, MangoCache, MangoGroup, MetaData, PerpMarket, PerpMarketCache,
    PerpMarketInfo, CENTIBPS_PER_UNIT, MAX_PERP_OPEN_ORDERS, ZERO_I80F48,
};
use crate::utils::emit_perp_balances;

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
    /// number of highest `key` bits that all children share
    /// e.g. if it's 2, the two highest bits of `key` will be the same on all children
    pub prefix_len: u32,
    /// only the top `prefix_len` bits of `key` are relevant
    pub key: i128,
    /// indexes into `BookSide::nodes`
    pub children: [NodeHandle; 2],
    pub child_expiry: [u64; 2],
    pub padding: [u8; NODE_SIZE - 48],
}

impl InnerNode {
    fn new(prefix_len: u32, key: i128) -> Self {
        Self {
            tag: NodeTag::InnerNode.into(),
            prefix_len,
            key,
            children: [0; 2],
            child_expiry: [u64::MAX; 2],
            padding: [0; NODE_SIZE - 48],
        }
    }

    /// Returns the handle of the child that may contain the search key
    /// and 0 or 1 depending on which child it was.
    fn walk_down(&self, search_key: i128) -> (NodeHandle, bool) {
        let crit_bit_mask = 1i128 << (127 - self.prefix_len);
        let crit_bit = (search_key & crit_bit_mask) != 0;
        (self.children[crit_bit as usize], crit_bit)
    }

    #[inline(always)]
    pub fn expiry(&self) -> u64 {
        std::cmp::min(self.child_expiry[0], self.child_expiry[1])
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Pod)]
#[repr(C)]
pub struct LeafNode {
    pub tag: u32,
    pub owner_slot: u8,
    pub order_type: OrderType, // this was added for TradingView move order
    pub version: u8,
    pub time_in_force: u8,
    pub key: i128,
    pub owner: Pubkey,
    pub quantity: i64,
    pub client_order_id: u64,

    // Liquidity incentive related parameters
    // Either the best bid or best ask at the time the order was placed
    pub best_initial: i64,

    // The time the order was placed
    pub timestamp: u64,
}

#[inline(always)]
fn key_to_price(key: i128) -> i64 {
    (key >> 64) as i64
}
impl LeafNode {
    pub fn new(
        version: u8,
        owner_slot: u8,
        key: i128,
        owner: Pubkey,
        quantity: i64,
        client_order_id: u64,
        timestamp: u64,
        best_initial: i64,
        order_type: OrderType,
        time_in_force: u8,
    ) -> Self {
        Self {
            tag: NodeTag::LeafNode.into(),
            owner_slot,
            order_type,
            version,
            time_in_force,
            key,
            owner,
            quantity,
            client_order_id,
            best_initial,
            timestamp,
        }
    }

    #[inline(always)]
    pub fn price(&self) -> i64 {
        key_to_price(self.key)
    }

    #[inline(always)]
    pub fn expiry(&self) -> u64 {
        if self.time_in_force == 0 {
            u64::MAX
        } else {
            self.timestamp + self.time_in_force as u64
        }
    }

    #[inline(always)]
    pub fn is_valid(&self, now_ts: u64) -> bool {
        self.time_in_force == 0 || now_ts <= self.timestamp + self.time_in_force as u64
    }
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
struct FreeNode {
    tag: u32,
    next: NodeHandle,
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

    fn children(&self) -> Option<[NodeHandle; 2]> {
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

    #[inline]
    pub fn as_inner_mut(&mut self) -> Option<&mut InnerNode> {
        match self.case_mut() {
            Some(NodeRefMut::Inner(inner_ref)) => Some(inner_ref),
            _ => None,
        }
    }

    #[inline]
    pub fn expiry(&self) -> u64 {
        match self.case().unwrap() {
            NodeRef::Inner(inner) => inner.expiry(),
            NodeRef::Leaf(leaf) => leaf.expiry(),
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
    PostOnlySlide = 4,
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

/// A binary tree on AnyNode::key()
///
/// The key encodes the price in the top 64 bits.
#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct BookSide {
    pub meta_data: MetaData,

    pub bump_index: usize,
    pub free_list_len: usize,
    pub free_list_head: NodeHandle,
    pub root_node: NodeHandle,
    pub leaf_count: usize,
    pub nodes: [AnyNode; MAX_BOOK_NODES],
}

pub struct BookSideIter<'a> {
    book_side: &'a BookSide,
    stack: Vec<&'a InnerNode>,
    next_leaf: Option<&'a LeafNode>,

    /// either 0, 1 to iterate low-to-high, or 1, 0 to iterate high-to-low
    left: usize,
    right: usize,

    now_ts: u64,
}

impl<'a> BookSideIter<'a> {
    pub fn new(book_side: &'a BookSide, now_ts: u64) -> Self {
        let (left, right) =
            if book_side.meta_data.data_type == DataType::Bids as u8 { (1, 0) } else { (0, 1) };
        let mut stack = vec![];
        let mut current = book_side.root_node;
        if book_side.leaf_count == 0 {
            Self { book_side, stack, next_leaf: None, left, right, now_ts }
        } else {
            loop {
                match book_side.get(current).unwrap().case().unwrap() {
                    NodeRef::Inner(inner) => {
                        stack.push(inner);
                        current = inner.children[left];
                    }
                    NodeRef::Leaf(leaf) => {
                        if leaf.is_valid(now_ts) {
                            return Self {
                                book_side,
                                stack,
                                next_leaf: Some(leaf),
                                left,
                                right,
                                now_ts,
                            };
                        } else {
                            match stack.pop() {
                                None => {
                                    return Self {
                                        book_side,
                                        stack,
                                        next_leaf: None,
                                        left,
                                        right,
                                        now_ts,
                                    }
                                }
                                Some(inner) => current = inner.children[right],
                            }
                        }
                    }
                }
            }
        }
    }
}

impl<'a> Iterator for BookSideIter<'a> {
    type Item = &'a LeafNode;

    fn next(&mut self) -> Option<Self::Item> {
        // if next leaf is None just return it
        if self.next_leaf.is_none() {
            return None;
        }

        // start popping from stack and get the other child
        let current_leaf = self.next_leaf;
        let mut current = match self.stack.pop() {
            None => {
                self.next_leaf = None;
                return current_leaf;
            }
            Some(inner) => inner.children[self.right],
        };

        // go down the left branch as much as possible until reaching a valid leaf
        loop {
            match self.book_side.get(current).unwrap().case().unwrap() {
                NodeRef::Inner(inner) => {
                    self.stack.push(inner);
                    current = inner.children[self.left];
                }
                NodeRef::Leaf(leaf) => {
                    if leaf.is_valid(self.now_ts) {
                        self.next_leaf = Some(leaf);
                        return current_leaf;
                    } else {
                        match self.stack.pop() {
                            None => {
                                self.next_leaf = None;
                                return current_leaf;
                            }
                            Some(inner) => {
                                current = inner.children[self.right];
                            }
                        }
                    }
                }
            }
        }
    }
}

impl BookSide {
    /// Iterate over all entries in the book filtering out invalid orders
    ///
    /// smallest to highest for asks
    /// highest to smallest for bids
    pub fn iter(&self, now_ts: u64) -> BookSideIter {
        BookSideIter::new(self, now_ts)
    }

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

    fn get_mut(&mut self, key: NodeHandle) -> Option<&mut AnyNode> {
        let node = &mut self.nodes[key as usize];
        let tag = NodeTag::try_from(node.tag);
        match tag {
            Ok(NodeTag::InnerNode) | Ok(NodeTag::LeafNode) => Some(node),
            _ => None,
        }
    }
    fn get(&self, key: NodeHandle) -> Option<&AnyNode> {
        let node = &self.nodes[key as usize];
        let tag = NodeTag::try_from(node.tag);
        match tag {
            Ok(NodeTag::InnerNode) | Ok(NodeTag::LeafNode) => Some(node),
            _ => None,
        }
    }

    pub fn remove_min(&mut self) -> Option<LeafNode> {
        self.remove_by_key(self.get(self.find_min()?)?.key()?)
    }

    pub fn remove_max(&mut self) -> Option<LeafNode> {
        self.remove_by_key(self.get(self.find_max()?)?.key()?)
    }

    pub fn remove_one_expired(&mut self, now_ts: u64) -> Option<LeafNode> {
        let (expired_h, expires_at) = self.soonest_expiry()?;
        if expires_at < now_ts {
            self.remove_by_key(self.get(expired_h)?.key()?)
        } else {
            None
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

    fn remove_by_key(&mut self, search_key: i128) -> Option<LeafNode> {
        // path of InnerNode handles that lead to the removed leaf
        let mut stack: Vec<(NodeHandle, bool)> = vec![];

        // special case potentially removing the root
        let mut parent_h = self.root()?;
        let (mut child_h, mut crit_bit) = match self.get(parent_h).unwrap().case().unwrap() {
            NodeRef::Leaf(&leaf) if leaf.key == search_key => {
                assert_eq!(self.leaf_count, 1);
                self.root_node = 0;
                self.leaf_count = 0;
                let _old_root = self.remove(parent_h).unwrap();
                return Some(leaf);
            }
            NodeRef::Leaf(_) => return None,
            NodeRef::Inner(inner) => inner.walk_down(search_key),
        };
        stack.push((parent_h, crit_bit));

        // walk down the tree until finding the key
        loop {
            match self.get(child_h).unwrap().case().unwrap() {
                NodeRef::Inner(inner) => {
                    parent_h = child_h;
                    let (new_child_h, new_crit_bit) = inner.walk_down(search_key);
                    child_h = new_child_h;
                    crit_bit = new_crit_bit;
                    stack.push((parent_h, crit_bit));
                }
                NodeRef::Leaf(leaf) => {
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
        let new_expiry = other_child_node_contents.expiry();
        *self.get_mut(parent_h).unwrap() = other_child_node_contents;
        self.leaf_count -= 1;
        let removed_leaf: LeafNode = cast(self.remove(child_h).unwrap());

        // update child min expiry back up to the root
        let outdated_expiry = removed_leaf.expiry();
        stack.pop(); // the final parent has been replaced by the remaining leaf
        self.update_expiry(&stack, outdated_expiry, new_expiry);

        Some(removed_leaf)
    }

    fn remove(&mut self, key: NodeHandle) -> Option<AnyNode> {
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

    fn insert(&mut self, val: &AnyNode) -> MangoResult<NodeHandle> {
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

        // TODO - test borrow checker
        self.free_list_head = cast_ref::<AnyNode, FreeNode>(node).next;
        self.free_list_len -= 1;
        *node = *val;
        Ok(key)
    }
    pub fn insert_leaf(
        &mut self,
        new_leaf: &LeafNode,
    ) -> MangoResult<(NodeHandle, Option<LeafNode>)> {
        // path of InnerNode handles that lead to the removed leaf
        let mut stack: Vec<(NodeHandle, bool)> = vec![];

        // deal with inserts into an empty tree
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
                    self.update_expiry(&stack, old_root_as_leaf.expiry(), new_leaf.expiry());
                    return Ok((root, Some(old_root_as_leaf)));
                }
                // InnerNodes have a random child's key, so matching can happen and is fine
            }
            let shared_prefix_len: u32 = (root_key ^ new_leaf.key).leading_zeros();
            match root_contents.case() {
                None => unreachable!(),
                Some(NodeRef::Inner(inner)) => {
                    let keep_old_root = shared_prefix_len >= inner.prefix_len;
                    if keep_old_root {
                        let (child, crit_bit) = inner.walk_down(new_leaf.key);
                        stack.push((root, crit_bit));
                        root = child;
                        continue;
                    };
                }
                _ => (),
            };
            // implies root is a Leaf or Inner where shared_prefix_len < prefix_len
            // we'll replace root with a new InnerNode that has new_leaf and root as children

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

            let new_leaf_expiry = new_leaf.expiry();
            let old_root_expiry = root_contents.expiry();
            new_root.child_expiry[new_leaf_crit_bit as usize] = new_leaf_expiry;
            new_root.child_expiry[old_root_crit_bit as usize] = old_root_expiry;

            // walk up the stack and fix up the new min if needed
            if new_leaf_expiry < old_root_expiry {
                self.update_expiry(&stack, old_root_expiry, new_leaf_expiry);
            }

            self.leaf_count += 1;
            return Ok((new_leaf_handle, None));
        }
    }

    pub fn is_full(&self) -> bool {
        self.free_list_len <= 1 && self.bump_index >= self.nodes.len() - 1
    }

    fn update_expiry(
        &mut self,
        stack: &[(NodeHandle, bool)],
        mut outdated_expiry: u64,
        mut new_expiry: u64,
    ) {
        for (parent_h, crit_bit) in stack.iter().rev() {
            let parent = self.get_mut(*parent_h).unwrap().as_inner_mut().unwrap();
            if parent.child_expiry[*crit_bit as usize] != outdated_expiry {
                break;
            }
            outdated_expiry = parent.expiry();
            parent.child_expiry[*crit_bit as usize] = new_expiry;
            new_expiry = std::cmp::min(new_expiry, parent.child_expiry[!*crit_bit as usize]);
        }
    }

    pub fn soonest_expiry(&self) -> Option<(NodeHandle, u64)> {
        let mut current: NodeHandle = match self.root() {
            Some(h) => h,
            None => return None,
        };

        loop {
            let contents = *self.get(current).unwrap();
            match contents.case() {
                None => unreachable!(),
                Some(NodeRef::Inner(inner)) => {
                    current =
                        inner.children[(inner.child_expiry[0] > inner.child_expiry[1]) as usize];
                }
                _ => {
                    return Some((current, contents.expiry()));
                }
            };
        }
    }
}

pub struct Book<'a> {
    pub bids: RefMut<'a, BookSide>,
    pub asks: RefMut<'a, BookSide>,
}

impl<'a> Book<'a> {
    pub fn load_checked(
        program_id: &Pubkey,
        bids_ai: &'a AccountInfo,
        asks_ai: &'a AccountInfo,
        perp_market: &PerpMarket,
    ) -> MangoResult<Self> {
        check!(bids_ai.key == &perp_market.bids, MangoErrorCode::InvalidAccount)?;
        check!(asks_ai.key == &perp_market.asks, MangoErrorCode::InvalidAccount)?;
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

    /// Get the quantity of bids above and including the price
    pub fn get_bids_size_above(&self, price: i64, max_depth: i64, now_ts: u64) -> i64 {
        let mut s = 0;
        for bid in self.bids.iter(now_ts) {
            if price > bid.price() || s >= max_depth {
                break;
            }
            s += bid.quantity;
        }
        s.min(max_depth)
    }

    /// Walk up the book `quantity` units and return the price at that level. If `quantity` units
    /// not on book, return None
    pub fn get_impact_price(&self, side: Side, quantity: i64, now_ts: u64) -> Option<i64> {
        let mut s = 0;
        let book_side = match side {
            Side::Bid => self.bids.iter(now_ts),
            Side::Ask => self.asks.iter(now_ts),
        };
        for order in book_side {
            s += order.quantity;
            if s >= quantity {
                return Some(order.price());
            }
        }
        None
    }

    /// Get the quantity of asks below and including the price
    pub fn get_asks_size_below(&self, price: i64, max_depth: i64, now_ts: u64) -> i64 {
        let mut s = 0;
        for ask in self.asks.iter(now_ts) {
            if price < ask.price() || s >= max_depth {
                break;
            }
            s += ask.quantity;
        }
        s.min(max_depth)
    }
    /// Get the quantity of bids above this order id. Will return full size of book if order id not found
    pub fn get_bids_size_above_order(&self, order_id: i128, max_depth: i64, now_ts: u64) -> i64 {
        let mut s = 0;
        for bid in self.bids.iter(now_ts) {
            if bid.key == order_id || s >= max_depth {
                break;
            }
            s += bid.quantity;
        }
        s.min(max_depth)
    }

    /// Get the quantity of bids above this order id. Will return full size of book if order id not found
    pub fn get_asks_size_below_order(&self, order_id: i128, max_depth: i64, now_ts: u64) -> i64 {
        let mut s = 0;
        for ask in self.asks.iter(now_ts) {
            if ask.key == order_id || s >= max_depth {
                break;
            }
            s += ask.quantity;
        }
        s.min(max_depth)
    }
    #[inline(never)]
    pub fn new_order(
        &mut self,
        program_id: &Pubkey,
        mango_group: &MangoGroup,
        mango_group_pk: &Pubkey,
        mango_cache: &MangoCache,
        event_queue: &mut EventQueue,
        market: &mut PerpMarket,
        oracle_price: I80F48,
        mango_account: &mut MangoAccount,
        mango_account_pk: &Pubkey,
        market_index: usize,
        side: Side,
        price: i64,
        quantity: i64, // quantity is guaranteed to be greater than zero due to initial check --
        order_type: OrderType,
        time_in_force: u8,
        client_order_id: u64,
        now_ts: u64,
        referrer_mango_account_ai: Option<&AccountInfo>,
    ) -> MangoResult {
        match side {
            Side::Bid => self.new_bid(
                program_id,
                mango_group,
                mango_group_pk,
                mango_cache,
                event_queue,
                market,
                oracle_price,
                mango_account,
                mango_account_pk,
                market_index,
                price,
                quantity,
                order_type,
                time_in_force,
                client_order_id,
                now_ts,
                referrer_mango_account_ai,
            ),
            Side::Ask => self.new_ask(
                program_id,
                mango_group,
                mango_group_pk,
                mango_cache,
                event_queue,
                market,
                oracle_price,
                mango_account,
                mango_account_pk,
                market_index,
                price,
                quantity,
                order_type,
                time_in_force,
                client_order_id,
                now_ts,
                referrer_mango_account_ai,
            ),
        }
    }

    /// Iterate over the book and return
    /// return changes to (taker_base, taker_quote, bids_quantity, asks_quantity)
    pub fn sim_new_bid(
        &self,
        market: &PerpMarket,
        info: &PerpMarketInfo,
        oracle_price: I80F48,
        price: i64,
        quantity: i64, // quantity is guaranteed to be greater than zero due to initial check --
        order_type: OrderType,
        now_ts: u64,
    ) -> MangoResult<(i64, i64, i64, i64)> {
        let (mut taker_base, mut taker_quote, mut bids_quantity, asks_quantity) = (0, 0, 0i64, 0);

        let (post_only, mut post_allowed, price) = match order_type {
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
        if post_allowed {
            // price limit check computed lazily to save CU on average
            let native_price = market.lot_to_native_price(price);
            if native_price.checked_div(oracle_price).unwrap() > info.maint_liab_weight {
                msg!("Posting on book disallowed due to price limits");
                post_allowed = false;
            }
        }

        let mut rem_quantity = quantity; // base lots (aka contracts)

        for best_ask in self.asks.iter(now_ts) {
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
            if rem_quantity == 0 {
                break;
            }
        }

        if rem_quantity > 0 && post_allowed {
            bids_quantity = bids_quantity.checked_add(rem_quantity).unwrap();
        }
        Ok((taker_base, taker_quote, bids_quantity, asks_quantity))
    }

    pub fn sim_new_ask(
        &self,
        market: &PerpMarket,
        info: &PerpMarketInfo,
        oracle_price: I80F48,
        price: i64,
        quantity: i64, // quantity is guaranteed to be greater than zero due to initial check --
        order_type: OrderType,
        now_ts: u64,
    ) -> MangoResult<(i64, i64, i64, i64)> {
        let (mut taker_base, mut taker_quote, bids_quantity, mut asks_quantity) = (0, 0, 0, 0i64);

        let (post_only, mut post_allowed, price) = match order_type {
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
        if post_allowed {
            // price limit check computed lazily to save CU on average
            let native_price = market.lot_to_native_price(price);
            if native_price.checked_div(oracle_price).unwrap() < info.maint_asset_weight {
                msg!("Posting on book disallowed due to price limits");
                post_allowed = false;
            }
        }

        let mut rem_quantity = quantity; // base lots (aka contracts)

        for best_bid in self.bids.iter(now_ts) {
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
            if rem_quantity == 0 {
                break;
            }
        }

        if rem_quantity > 0 && post_allowed {
            asks_quantity = asks_quantity.checked_add(rem_quantity).unwrap();
        }
        Ok((taker_base, taker_quote, bids_quantity, asks_quantity))
    }

    #[inline(never)]
    fn new_bid(
        &mut self,
        program_id: &Pubkey,
        mango_group: &MangoGroup,
        mango_group_pk: &Pubkey,
        mango_cache: &MangoCache,
        event_queue: &mut EventQueue,
        market: &mut PerpMarket,
        oracle_price: I80F48,
        mango_account: &mut MangoAccount,
        mango_account_pk: &Pubkey,
        market_index: usize,
        price: i64,
        quantity: i64, // quantity is guaranteed to be greater than zero due to initial check
        order_type: OrderType,
        time_in_force: u8,
        client_order_id: u64,
        now_ts: u64,
        referrer_mango_account_ai: Option<&AccountInfo>,
    ) -> MangoResult {
        // TODO proper error handling
        // TODO handle the case where we run out of compute (right now just fails)
        let (post_only, mut post_allowed, price) = match order_type {
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
        let info = &mango_group.perp_markets[market_index];
        if post_allowed {
            // price limit check computed lazily to save CU on average
            let native_price = market.lot_to_native_price(price);
            if native_price.checked_div(oracle_price).unwrap() > info.maint_liab_weight {
                msg!("Posting on book disallowed due to price limits");
                post_allowed = false;
            }
        }

        // referral fee related variables
        let mut ref_fee_rate = None;
        let mut referrer_mango_account_opt = None;
        let mut total_quote_taken = 0;

        // generate new order id
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

            if !best_ask.is_valid(now_ts) {
                // Remove the order from the book
                let event = OutEvent::new(
                    Side::Ask,
                    best_ask.owner_slot,
                    now_ts,
                    event_queue.header.seq_num,
                    best_ask.owner,
                    best_ask.quantity,
                );
                event_queue.push_back(cast(event)).unwrap();
                let key = best_ask.key;
                let _removed_node = self.asks.remove_by_key(key).unwrap();
                continue;
            }

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
            let match_quote = match_quantity * best_ask_price;
            total_quote_taken += match_quote;

            mango_account.perp_accounts[market_index].add_taker_trade(match_quantity, -match_quote);
            let maker_out = best_ask.quantity == 0;

            // if ref_fee_rate is none, determine it
            // if ref_valid, then pay into referrer, else pay to perp market
            if ref_fee_rate.is_none() {
                let (a, b) = determine_ref_vars(
                    program_id,
                    mango_group,
                    mango_group_pk,
                    mango_cache,
                    mango_account,
                    referrer_mango_account_ai,
                    now_ts,
                )?;
                ref_fee_rate = Some(a);
                referrer_mango_account_opt = b;
            }

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
                info.taker_fee + ref_fee_rate.unwrap(),
                best_ask_price,
                match_quantity,
                best_ask.version,
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
                // Drop an expired if possible
                if let Some(expired_bid) = self.bids.remove_one_expired(now_ts) {
                    let event = OutEvent::new(
                        Side::Bid,
                        expired_bid.owner_slot,
                        now_ts,
                        event_queue.header.seq_num,
                        expired_bid.owner,
                        expired_bid.quantity,
                    );
                    event_queue.push_back(cast(event)).unwrap();
                } else {
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
            }

            // iterate through book on the bid side
            let best_initial = if market.meta_data.version == 0 {
                match self.get_best_bid_price() {
                    None => price,
                    Some(p) => p,
                }
            } else {
                let max_depth: i64 = market.liquidity_mining_info.max_depth_bps.to_num();
                self.get_bids_size_above(price, max_depth, now_ts)
            };

            let owner_slot = mango_account
                .next_order_slot()
                .ok_or(throw_err!(MangoErrorCode::TooManyOpenOrders))?;
            let new_bid = LeafNode::new(
                market.meta_data.version,
                owner_slot as u8,
                order_id,
                *mango_account_pk,
                rem_quantity,
                client_order_id,
                now_ts,
                best_initial,
                order_type,
                time_in_force,
            );
            let _result = self.bids.insert_leaf(&new_bid)?;

            // TODO OPT remove if PlacePerpOrder needs more compute
            msg!("bid on book order_id={} quantity={} price={}", order_id, rem_quantity, price);
            mango_account.add_order(market_index, Side::Bid, &new_bid)?;
        }

        // if there were matched taker quote apply ref fees
        // we know ref_fee_rate is not None if total_quote_taken > 0
        if total_quote_taken > 0 {
            apply_fees(
                market,
                info,
                mango_account,
                mango_account_pk,
                market_index,
                referrer_mango_account_opt,
                referrer_mango_account_ai,
                total_quote_taken,
                ref_fee_rate.unwrap(),
                &mango_cache.perp_market_cache[market_index],
            );
        }

        Ok(())
    }

    #[inline(never)]
    pub fn new_ask(
        &mut self,
        program_id: &Pubkey,
        mango_group: &MangoGroup,
        mango_group_pk: &Pubkey,
        mango_cache: &MangoCache,
        event_queue: &mut EventQueue,
        market: &mut PerpMarket,
        oracle_price: I80F48,
        mango_account: &mut MangoAccount,
        mango_account_pk: &Pubkey,
        market_index: usize,
        price: i64,
        quantity: i64, // quantity is guaranteed to be greater than zero due to initial check
        order_type: OrderType,
        time_in_force: u8,
        client_order_id: u64,
        now_ts: u64,
        referrer_mango_account_ai: Option<&AccountInfo>,
    ) -> MangoResult {
        let (post_only, mut post_allowed, price) = match order_type {
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
        let info = &mango_group.perp_markets[market_index];
        if post_allowed {
            // price limit check computed lazily to save CU on average
            let native_price = market.lot_to_native_price(price);
            if native_price.checked_div(oracle_price).unwrap() < info.maint_asset_weight {
                msg!("Posting on book disallowed due to price limits");
                post_allowed = false;
            }
        }

        // referral fee related variables
        let mut ref_fee_rate = None;
        let mut referrer_mango_account_opt = None;
        let mut total_quote_taken = 0;

        // generate new order id
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

            if !best_bid.is_valid(now_ts) {
                // Remove the order from the book
                let event = OutEvent::new(
                    Side::Bid,
                    best_bid.owner_slot,
                    now_ts,
                    event_queue.header.seq_num,
                    best_bid.owner,
                    best_bid.quantity,
                );
                event_queue.push_back(cast(event)).unwrap();
                let key = best_bid.key;
                let _removed_node = self.bids.remove_by_key(key).unwrap();
                continue;
            }

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

            let match_quote = match_quantity * best_bid_price;
            total_quote_taken += match_quote;
            mango_account.perp_accounts[market_index].add_taker_trade(-match_quantity, match_quote);
            let maker_out = best_bid.quantity == 0;

            // if ref_fee_rate is none, determine it
            // if ref_valid, then pay into referrer, else pay to perp market
            if ref_fee_rate.is_none() {
                let (a, b) = determine_ref_vars(
                    program_id,
                    mango_group,
                    mango_group_pk,
                    mango_cache,
                    mango_account,
                    referrer_mango_account_ai,
                    now_ts,
                )?;
                ref_fee_rate = Some(a);
                referrer_mango_account_opt = b;
            }

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
                info.taker_fee + ref_fee_rate.unwrap(),
                best_bid_price,
                match_quantity,
                best_bid.version,
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
                // Drop an expired if possible
                if let Some(expired_ask) = self.asks.remove_one_expired(now_ts) {
                    let event = OutEvent::new(
                        Side::Ask,
                        expired_ask.owner_slot,
                        now_ts,
                        event_queue.header.seq_num,
                        expired_ask.owner,
                        expired_ask.quantity,
                    );
                    event_queue.push_back(cast(event)).unwrap();
                } else {
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
            }

            let best_initial = if market.meta_data.version == 0 {
                match self.get_best_ask_price() {
                    None => price,
                    Some(p) => p,
                }
            } else {
                let max_depth: i64 = market.liquidity_mining_info.max_depth_bps.to_num();
                self.get_asks_size_below(price, max_depth, now_ts)
            };

            let owner_slot = mango_account
                .next_order_slot()
                .ok_or(throw_err!(MangoErrorCode::TooManyOpenOrders))?;
            let new_ask = LeafNode::new(
                market.meta_data.version,
                owner_slot as u8,
                order_id,
                *mango_account_pk,
                rem_quantity,
                client_order_id,
                now_ts,
                best_initial,
                order_type,
                time_in_force,
            );

            // TODO OPT remove if PlacePerpOrder needs more compute
            msg!("ask on book order_id={} quantity={} price={}", order_id, rem_quantity, price);

            let _result = self.asks.insert_leaf(&new_ask)?;
            mango_account.add_order(market_index, Side::Ask, &new_ask)?;
        }

        // if there were matched taker quote apply ref fees
        // we know ref_fee_rate is not None if total_quote_taken > 0
        if total_quote_taken > 0 {
            apply_fees(
                market,
                info,
                mango_account,
                mango_account_pk,
                market_index,
                referrer_mango_account_opt,
                referrer_mango_account_ai,
                total_quote_taken,
                ref_fee_rate.unwrap(),
                &mango_cache.perp_market_cache[market_index],
            );
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
    ) -> MangoResult {
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

    pub fn cancel_all_side_with_size_incentives(
        &mut self,
        mango_account: &mut MangoAccount,
        perp_market: &mut PerpMarket,
        market_index: usize,
        side: Side,
        mut limit: u8,
    ) -> MangoResult<(Vec<i128>, Vec<i128>)> {
        // TODO - test different limits
        let now_ts = Clock::get()?.unix_timestamp as u64;
        let max_depth: i64 = perp_market.liquidity_mining_info.max_depth_bps.to_num();

        let mut all_order_ids = vec![];
        let mut canceled_order_ids = vec![];
        let mut keys = vec![];
        let market_index_u8 = market_index as u8;
        for i in 0..MAX_PERP_OPEN_ORDERS {
            if mango_account.order_market[i] == market_index_u8
                && mango_account.order_side[i] == side
            {
                all_order_ids.push(mango_account.orders[i]);
                keys.push(mango_account.orders[i])
            }
        }
        match side {
            Side::Bid => self.cancel_all_bids_with_size_incentives(
                mango_account,
                perp_market,
                market_index,
                max_depth,
                now_ts,
                &mut limit,
                keys,
                &mut canceled_order_ids,
            )?,
            Side::Ask => self.cancel_all_asks_with_size_incentives(
                mango_account,
                perp_market,
                market_index,
                max_depth,
                now_ts,
                &mut limit,
                keys,
                &mut canceled_order_ids,
            )?,
        };
        Ok((all_order_ids, canceled_order_ids))
    }
    pub fn cancel_all_with_size_incentives(
        &mut self,
        mango_account: &mut MangoAccount,
        perp_market: &mut PerpMarket,
        market_index: usize,
        mut limit: u8,
    ) -> MangoResult<(Vec<i128>, Vec<i128>)> {
        // TODO - test different limits
        let now_ts = Clock::get()?.unix_timestamp as u64;
        let max_depth: i64 = perp_market.liquidity_mining_info.max_depth_bps.to_num();

        let mut all_order_ids = vec![];
        let mut canceled_order_ids = vec![];

        let market_index_u8 = market_index as u8;
        let mut bids_keys = vec![];
        let mut asks_keys = vec![];
        for i in 0..MAX_PERP_OPEN_ORDERS {
            if mango_account.order_market[i] != market_index_u8 {
                continue;
            }
            all_order_ids.push(mango_account.orders[i]);
            match mango_account.order_side[i] {
                Side::Bid => bids_keys.push(mango_account.orders[i]),
                Side::Ask => asks_keys.push(mango_account.orders[i]),
            }
        }
        self.cancel_all_bids_with_size_incentives(
            mango_account,
            perp_market,
            market_index,
            max_depth,
            now_ts,
            &mut limit,
            bids_keys,
            &mut canceled_order_ids,
        )?;
        self.cancel_all_asks_with_size_incentives(
            mango_account,
            perp_market,
            market_index,
            max_depth,
            now_ts,
            &mut limit,
            asks_keys,
            &mut canceled_order_ids,
        )?;
        Ok((all_order_ids, canceled_order_ids))
    }

    /// Internal
    fn cancel_all_bids_with_size_incentives(
        &mut self,
        mango_account: &mut MangoAccount,
        perp_market: &mut PerpMarket,
        market_index: usize,
        max_depth: i64,
        now_ts: u64,
        limit: &mut u8,
        mut my_bids: Vec<i128>,
        canceled_order_ids: &mut Vec<i128>,
    ) -> MangoResult {
        my_bids.sort_unstable();
        let mut bids_and_sizes = vec![];
        let mut cuml_bids = 0;

        let mut iter = self.bids.iter(0); // pass in 0 so all orders are considered valid
        let mut curr = iter.next();
        while let Some(bid) = curr {
            match my_bids.last() {
                None => break,
                Some(&my_highest_bid) => {
                    if bid.key > my_highest_bid {
                        if bid.is_valid(now_ts) {
                            // if bid is not valid, it doesn't count towards book liquidity
                            cuml_bids += bid.quantity;
                        }
                        curr = iter.next();
                    } else if bid.key == my_highest_bid {
                        bids_and_sizes.push((bid.key, cuml_bids));
                        my_bids.pop();
                        curr = iter.next();
                    } else {
                        // my_highest_bid is not on the book; it must be on EventQueue waiting to be processed
                        // check the next my_highest_bid against bid
                        my_bids.pop();
                    }

                    if cuml_bids >= max_depth {
                        for bid_key in my_bids {
                            bids_and_sizes.push((bid_key, max_depth));
                        }
                        break;
                    }
                }
            }
        }

        for (key, cuml_size) in bids_and_sizes {
            if *limit == 0 {
                return Ok(());
            } else {
                *limit -= 1;
            }

            match self.cancel_order(key, Side::Bid) {
                Ok(order) => {
                    mango_account.remove_order(order.owner_slot as usize, order.quantity)?;
                    canceled_order_ids.push(key);
                    if order.version == perp_market.meta_data.version
                        && order.version != 0
                        && order.is_valid(now_ts)
                    {
                        mango_account.perp_accounts[market_index].apply_size_incentives(
                            perp_market,
                            order.best_initial,
                            cuml_size,
                            order.timestamp,
                            now_ts,
                            order.quantity,
                        )?;
                    }
                }
                Err(_) => {
                    msg!("Failed to cancel bid oid: {}; Either error state or bid is on EventQueue unprocessed", key)
                }
            }
        }
        Ok(())
    }

    /// Internal
    fn cancel_all_asks_with_size_incentives(
        &mut self,
        mango_account: &mut MangoAccount,
        perp_market: &mut PerpMarket,
        market_index: usize,
        max_depth: i64,
        now_ts: u64,
        limit: &mut u8,
        mut my_asks: Vec<i128>,
        canceled_order_ids: &mut Vec<i128>,
    ) -> MangoResult {
        my_asks.sort_unstable_by(|a, b| b.cmp(a));
        let mut asks_and_sizes = vec![];
        let mut cuml_asks = 0;

        let mut iter = self.asks.iter(0); // pass in 0 so all orders are considered valid
        let mut curr = iter.next();
        while let Some(ask) = curr {
            match my_asks.last() {
                None => break,
                Some(&my_lowest_ask) => {
                    if ask.key < my_lowest_ask {
                        if ask.is_valid(now_ts) {
                            // if ask is not valid, it doesn't count towards book liquidity
                            cuml_asks += ask.quantity;
                        }
                        curr = iter.next();
                    } else if ask.key == my_lowest_ask {
                        asks_and_sizes.push((ask.key, cuml_asks));
                        my_asks.pop();
                        curr = iter.next();
                    } else {
                        // my_lowest_ask is not on the book; it must be on EventQueue waiting to be processed
                        // check the next my_lowest_ask against ask
                        my_asks.pop();
                    }
                    if cuml_asks >= max_depth {
                        for key in my_asks {
                            asks_and_sizes.push((key, max_depth))
                        }
                        break;
                    }
                }
            }
        }

        for (key, cuml_size) in asks_and_sizes {
            if *limit == 0 {
                return Ok(());
            } else {
                *limit -= 1;
            }
            match self.cancel_order(key, Side::Ask) {
                Ok(order) => {
                    mango_account.remove_order(order.owner_slot as usize, order.quantity)?;
                    canceled_order_ids.push(key);
                    if order.version == perp_market.meta_data.version
                        && order.version != 0
                        && order.is_valid(now_ts)
                    {
                        mango_account.perp_accounts[market_index].apply_size_incentives(
                            perp_market,
                            order.best_initial,
                            cuml_size,
                            order.timestamp,
                            now_ts,
                            order.quantity,
                        )?;
                    }
                }
                Err(_) => {
                    msg!("Failed to cancel ask oid: {}; Either error state or ask is on EventQueue unprocessed", key);
                }
            }
        }

        Ok(())
    }
    /// Cancel all the orders for MangoAccount for this PerpMarket up to `limit`
    /// Only used when PerpMarket version == 0
    pub fn cancel_all_with_price_incentives(
        &mut self,
        mango_account: &mut MangoAccount,
        perp_market: &mut PerpMarket,
        market_index: usize,
        mut limit: u8,
    ) -> MangoResult {
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
                    if order.version != perp_market.meta_data.version {
                        continue;
                    }
                    mango_account.perp_accounts[market_index].apply_price_incentives(
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

fn determine_ref_vars<'a>(
    program_id: &Pubkey,
    mango_group: &MangoGroup,
    mango_group_pk: &Pubkey,
    mango_cache: &MangoCache,
    mango_account: &MangoAccount,
    referrer_mango_account_ai: Option<&'a AccountInfo>,
    now_ts: u64,
) -> MangoResult<(I80F48, Option<RefMut<'a, MangoAccount>>)> {
    let mngo_index = match mango_group.find_token_index(&mngo_token::id()) {
        None => return Ok((ZERO_I80F48, None)),
        Some(i) => i,
    };

    let mngo_cache = &mango_cache.root_bank_cache[mngo_index];

    // If the user's MNGO deposit is non-zero then the rootbank cache will be checked already in `place_perp_order`.
    // If it's zero then cache may be out of date, but it doesn't matter because 0 * index = 0
    let mngo_deposits = mango_account.get_native_deposit(mngo_cache, mngo_index)?;
    let ref_mngo_req = I80F48::from_num(mango_group.ref_mngo_required);
    if mngo_deposits >= ref_mngo_req {
        return Ok((ZERO_I80F48, None));
    } else if let Some(referrer_mango_account_ai) = referrer_mango_account_ai {
        // If referrer_mango_account is invalid, just treat it as if it doesn't exist
        if let Ok(referrer_mango_account) =
            MangoAccount::load_mut_checked(referrer_mango_account_ai, program_id, mango_group_pk)
        {
            // Need to check if it's valid because user may not have mngo in active assets
            mngo_cache.check_valid(mango_group, now_ts)?;
            let ref_mngo_deposits =
                referrer_mango_account.get_native_deposit(mngo_cache, mngo_index)?;

            if !referrer_mango_account.is_bankrupt
                && !referrer_mango_account.being_liquidated
                && ref_mngo_deposits >= ref_mngo_req
            {
                return Ok((
                    I80F48::from_num(mango_group.ref_share_centibps) / CENTIBPS_PER_UNIT,
                    Some(referrer_mango_account),
                ));
            }
        }
    }
    Ok((I80F48::from_num(mango_group.ref_surcharge_centibps) / CENTIBPS_PER_UNIT, None))
}

/// Apply taker fees to the taker account and update the markets' fees_accrued for
/// both the maker and taker fees.
fn apply_fees(
    market: &mut PerpMarket,
    info: &PerpMarketInfo,
    mango_account: &mut MangoAccount,
    mango_account_pk: &Pubkey,
    market_index: usize,
    referrer_mango_account_opt: Option<RefMut<MangoAccount>>,
    referrer_mango_account_ai: Option<&AccountInfo>,
    total_quote_taken: i64,
    ref_fee_rate: I80F48,
    perp_market_cache: &PerpMarketCache,
) {
    let taker_quote_native =
        I80F48::from_num(market.quote_lot_size.checked_mul(total_quote_taken).unwrap());

    if ref_fee_rate > ZERO_I80F48 {
        let ref_fees = taker_quote_native * ref_fee_rate;

        // if ref mango account is some, then we send some fees over
        if let Some(mut referrer_mango_account) = referrer_mango_account_opt {
            mango_account.perp_accounts[market_index].transfer_quote_position(
                &mut referrer_mango_account.perp_accounts[market_index],
                ref_fees,
            );
            emit_perp_balances(
                referrer_mango_account.mango_group,
                *referrer_mango_account_ai.unwrap().key,
                market_index as u64,
                &referrer_mango_account.perp_accounts[market_index],
                perp_market_cache,
            );
            mango_emit_stack::<_, 200>(ReferralFeeAccrualLog {
                mango_group: referrer_mango_account.mango_group,
                referrer_mango_account: *referrer_mango_account_ai.unwrap().key,
                referree_mango_account: *mango_account_pk,
                market_index: market_index as u64,
                referral_fee_accrual: ref_fees.to_bits(),
            });
        } else {
            // else user didn't have valid amount of MNGO and no valid referrer
            mango_account.perp_accounts[market_index].quote_position -= ref_fees;
            market.fees_accrued += ref_fees;
        }
    }

    // Track maker fees immediately: they can be negative and applying them later
    // risks that fees_accrued is settled to 0 before they apply. It going negative
    // breaks assumptions.
    // The maker fees apply to the maker's account only when the fill event is consumed.
    let maker_fees = taker_quote_native * info.maker_fee;

    let taker_fees = taker_quote_native * info.taker_fee;
    mango_account.perp_accounts[market_index].quote_position -= taker_fees;
    market.fees_accrued += taker_fees + maker_fees;

    emit_perp_balances(
        mango_account.mango_group,
        *mango_account_pk,
        market_index as u64,
        &mango_account.perp_accounts[market_index],
        perp_market_cache,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_bookside(data_type: DataType) -> BookSide {
        BookSide {
            meta_data: MetaData::new(data_type, 0, true),
            bump_index: 0,
            free_list_len: 0,
            free_list_head: 0,
            root_node: 0,
            leaf_count: 0,
            nodes: [AnyNode { tag: 0, data: [0u8; NODE_SIZE - 4] }; MAX_BOOK_NODES],
        }
    }

    fn verify_bookside_expiry(bookside: &BookSide) {
        let r = match bookside.root() {
            Some(h) => h,
            None => return,
        };

        fn recursive_check(bookside: &BookSide, h: NodeHandle) {
            match bookside.get(h).unwrap().case().unwrap() {
                NodeRef::Inner(&inner) => {
                    let left = bookside.get(inner.children[0]).unwrap().expiry();
                    let right = bookside.get(inner.children[1]).unwrap().expiry();
                    assert_eq!(inner.child_expiry[0], left);
                    assert_eq!(inner.child_expiry[1], right);
                    recursive_check(bookside, inner.children[0]);
                    recursive_check(bookside, inner.children[1]);
                }
                _ => {}
            }
        };
        recursive_check(bookside, r);
    }

    #[test]
    fn bookside_expiry_manual() {
        let mut bids = new_bookside(DataType::Bids);
        let new_expiring_leaf = |key: i128, expiry: u64| {
            LeafNode::new(0, 0, key, Pubkey::default(), 0, 0, expiry - 1, 0, OrderType::Limit, 1)
        };

        assert!(bids.soonest_expiry().is_none());

        bids.insert_leaf(&new_expiring_leaf(0, 5000)).unwrap();
        assert_eq!(bids.soonest_expiry().unwrap(), (bids.root_node, 5000));
        verify_bookside_expiry(&bids);

        let (new4000_h, _) = bids.insert_leaf(&new_expiring_leaf(1, 4000)).unwrap();
        assert_eq!(bids.soonest_expiry().unwrap(), (new4000_h, 4000));
        verify_bookside_expiry(&bids);

        let (new4500_h, _) = bids.insert_leaf(&new_expiring_leaf(2, 4500)).unwrap();
        assert_eq!(bids.soonest_expiry().unwrap(), (new4000_h, 4000));
        verify_bookside_expiry(&bids);

        let (new3500_h, _) = bids.insert_leaf(&new_expiring_leaf(3, 3500)).unwrap();
        assert_eq!(bids.soonest_expiry().unwrap(), (new3500_h, 3500));
        verify_bookside_expiry(&bids);
        // the first two levels of the tree are innernodes, with 0;1 on one side and 2;3 on the other
        assert_eq!(
            bids.get_mut(bids.root_node).unwrap().as_inner_mut().unwrap().child_expiry,
            [4000, 3500]
        );

        bids.remove_by_key(3).unwrap();
        verify_bookside_expiry(&bids);
        assert_eq!(
            bids.get_mut(bids.root_node).unwrap().as_inner_mut().unwrap().child_expiry,
            [4000, 4500]
        );
        assert_eq!(bids.soonest_expiry().unwrap().1, 4000);

        bids.remove_by_key(0).unwrap();
        verify_bookside_expiry(&bids);
        assert_eq!(
            bids.get_mut(bids.root_node).unwrap().as_inner_mut().unwrap().child_expiry,
            [4000, 4500]
        );
        assert_eq!(bids.soonest_expiry().unwrap().1, 4000);

        bids.remove_by_key(1).unwrap();
        verify_bookside_expiry(&bids);
        assert_eq!(bids.soonest_expiry().unwrap().1, 4500);

        bids.remove_by_key(2).unwrap();
        verify_bookside_expiry(&bids);
        assert!(bids.soonest_expiry().is_none());
    }

    #[test]
    fn bookside_expiry_random() {
        use rand::Rng;
        let mut rng = rand::thread_rng();

        let mut bids = new_bookside(DataType::Bids);
        let new_expiring_leaf = |key: i128, expiry: u64| {
            LeafNode::new(0, 0, key, Pubkey::default(), 0, 0, expiry - 1, 0, OrderType::Limit, 1)
        };

        // add 200 random leaves
        let mut keys = vec![];
        for _ in 0..200 {
            let key: i128 = rng.gen_range(0..10000); // overlap in key bits
            if keys.contains(&key) {
                continue;
            }
            keys.push(key);
            bids.insert_leaf(&new_expiring_leaf(key, rng.gen_range(1..200))); // give good chance of duplicate expiry times
            verify_bookside_expiry(&bids);
        }

        // remove 50 at random
        for _ in 0..50 {
            if keys.len() == 0 {
                break;
            }
            let k = keys[rng.gen_range(0..keys.len())];
            bids.remove_by_key(k);
            keys.retain(|v| *v != k);
            verify_bookside_expiry(&bids);
        }
    }
}
