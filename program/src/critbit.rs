use arrayref::{array_refs, mut_array_refs};
use bytemuck::{cast, cast_mut, cast_ref, cast_slice, cast_slice_mut, Pod, Zeroable};

use crate::error::{check_assert, MerpsErrorCode, MerpsResult, SourceFileId};
use crate::matching::{OrderType, Side};
use crate::queue::{EventQueue, EventType, FillEvent, OutEvent};
use crate::state::{MerpsAccount, MetaData, PerpMarket};
use fixed::types::I80F48;
use mango_common::Loadable;
use mango_macro::{Loadable, Pod};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use solana_program::account_info::AccountInfo;
use solana_program::pubkey::Pubkey;
use std::cell::RefMut;
use std::{
    convert::TryFrom,
    mem::{align_of, size_of},
};

declare_check_assert_macros!(SourceFileId::Critbit);

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

    #[cfg(test)]
    fn prefix_len(&self) -> u32 {
        match self.case().unwrap() {
            NodeRef::Inner(&InnerNode { prefix_len, .. }) => prefix_len,
            NodeRef::Leaf(_) => 128,
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

#[cfg(debug_assertions)]
unsafe fn invariant(check: bool) {
    if check {
        unreachable!();
    }
}

#[cfg(not(debug_assertions))]
#[inline(always)]
unsafe fn invariant(check: bool) {
    if check {
        std::hint::unreachable_unchecked();
    }
}

#[derive(Copy, Clone)]
#[repr(C)]
struct SlabHeader {
    pub bump_index: u64,
    pub free_list_len: u64,
    pub free_list_head: u32,

    pub root_node: u32,
    pub leaf_count: u64,
}
unsafe impl Zeroable for SlabHeader {}
unsafe impl Pod for SlabHeader {}

const SLAB_HEADER_LEN: usize = size_of::<SlabHeader>();

#[repr(transparent)]
pub struct Slab([u8]);

impl Slab {
    /// Creates a slab that holds and references the bytes
    #[inline]
    pub fn new(bytes: &mut [u8]) -> &mut Self {
        let len_without_header = bytes.len().checked_sub(SLAB_HEADER_LEN).unwrap();
        let slop = len_without_header % size_of::<AnyNode>();
        let truncated_len = bytes.len() - slop;
        let bytes = &mut bytes[..truncated_len];
        let slab: &mut Self = unsafe { &mut *(bytes as *mut [u8] as *mut Slab) };
        slab.check_size_align(); // check alignment
        slab
    }

    #[inline]
    pub fn assert_minimum_capacity(&self, capacity: u32) -> MerpsResult<()> {
        check!(self.nodes().len() <= (capacity as usize) * 2, MerpsErrorCode::Default)
    }

    fn check_size_align(&self) {
        let (header_bytes, nodes_bytes) = array_refs![&self.0, SLAB_HEADER_LEN; .. ;];
        let _header: &SlabHeader = cast_ref(header_bytes);
        let _nodes: &[AnyNode] = cast_slice(nodes_bytes);
    }

    fn parts(&self) -> (&SlabHeader, &[AnyNode]) {
        // TODO possibly remove this if it's safe
        unsafe {
            invariant(self.0.len() < size_of::<SlabHeader>());
            invariant((self.0.as_ptr() as usize) % align_of::<SlabHeader>() != 0);
            invariant(
                ((self.0.as_ptr() as usize) + size_of::<SlabHeader>()) % align_of::<AnyNode>() != 0,
            );
        }

        let (header_bytes, nodes_bytes) = array_refs![&self.0, SLAB_HEADER_LEN; .. ;];
        let header = cast_ref(header_bytes);
        let nodes = cast_slice(nodes_bytes);
        (header, nodes)
    }

    fn parts_mut(&mut self) -> (&mut SlabHeader, &mut [AnyNode]) {
        unsafe {
            invariant(self.0.len() < size_of::<SlabHeader>());
            invariant((self.0.as_ptr() as usize) % align_of::<SlabHeader>() != 0);
            invariant(
                ((self.0.as_ptr() as usize) + size_of::<SlabHeader>()) % align_of::<AnyNode>() != 0,
            );
        }

        let (header_bytes, nodes_bytes) = mut_array_refs![&mut self.0, SLAB_HEADER_LEN; .. ;];
        let header = cast_mut(header_bytes);
        let nodes = cast_slice_mut(nodes_bytes);
        (header, nodes)
    }

    fn header(&self) -> &SlabHeader {
        self.parts().0
    }

    fn header_mut(&mut self) -> &mut SlabHeader {
        self.parts_mut().0
    }

    fn nodes(&self) -> &[AnyNode] {
        self.parts().1
    }

    fn nodes_mut(&mut self) -> &mut [AnyNode] {
        self.parts_mut().1
    }
}

pub trait SlabView<T> {
    fn capacity(&self) -> u64;
    fn clear(&mut self);
    fn is_empty(&self) -> bool;
    fn get(&self, h: NodeHandle) -> Option<&T>;
    fn get_mut(&mut self, h: NodeHandle) -> Option<&mut T>;
    fn insert(&mut self, val: &T) -> Result<u32, ()>;
    fn remove(&mut self, h: NodeHandle) -> Option<T>;
    fn contains(&self, h: NodeHandle) -> bool;
}

impl SlabView<AnyNode> for Slab {
    fn capacity(&self) -> u64 {
        self.nodes().len() as u64
    }

    fn clear(&mut self) {
        let (header, _nodes) = self.parts_mut();
        *header = SlabHeader {
            bump_index: 0,
            free_list_len: 0,
            free_list_head: 0,

            root_node: 0,
            leaf_count: 0,
        }
    }

    fn is_empty(&self) -> bool {
        let SlabHeader { bump_index, free_list_len, .. } = *self.header();
        bump_index == free_list_len
    }

    fn get(&self, key: u32) -> Option<&AnyNode> {
        let node = self.nodes().get(key as usize)?;
        let tag = NodeTag::try_from(node.tag);
        match tag {
            Ok(NodeTag::InnerNode) | Ok(NodeTag::LeafNode) => Some(node),
            _ => None,
        }
    }

    fn get_mut(&mut self, key: u32) -> Option<&mut AnyNode> {
        let node = self.nodes_mut().get_mut(key as usize)?;
        let tag = NodeTag::try_from(node.tag);
        match tag {
            Ok(NodeTag::InnerNode) | Ok(NodeTag::LeafNode) => Some(node),
            _ => None,
        }
    }

    fn insert(&mut self, val: &AnyNode) -> Result<u32, ()> {
        match NodeTag::try_from(val.tag) {
            Ok(NodeTag::InnerNode) | Ok(NodeTag::LeafNode) => (),
            _ => unreachable!(),
        };

        let (header, nodes) = self.parts_mut();

        if header.free_list_len == 0 {
            if header.bump_index as usize == nodes.len() {
                return Err(());
            }

            if header.bump_index == u32::MAX as u64 {
                return Err(());
            }
            let key = header.bump_index as u32;
            header.bump_index += 1;

            nodes[key as usize] = *val;
            return Ok(key);
        }

        let key = header.free_list_head;
        let node = &mut nodes[key as usize];

        match NodeTag::try_from(node.tag) {
            Ok(NodeTag::FreeNode) => assert!(header.free_list_len > 1),
            Ok(NodeTag::LastFreeNode) => assert_eq!(header.free_list_len, 1),
            _ => unreachable!(),
        };

        let next_free_list_head: u32;
        {
            let free_list_item: &FreeNode = cast_ref(node);
            next_free_list_head = free_list_item.next;
        }
        header.free_list_head = next_free_list_head;
        header.free_list_len -= 1;
        *node = *val;
        Ok(key)
    }

    fn remove(&mut self, key: u32) -> Option<AnyNode> {
        let val = *self.get(key)?;
        let (header, nodes) = self.parts_mut();
        let any_node_ref = &mut nodes[key as usize];
        let free_node_ref: &mut FreeNode = cast_mut(any_node_ref);
        *free_node_ref = FreeNode {
            tag: if header.free_list_len == 0 {
                NodeTag::LastFreeNode.into()
            } else {
                NodeTag::FreeNode.into()
            },
            next: header.free_list_head,
            padding: [0u8; 64],
        };
        header.free_list_len += 1;
        header.free_list_head = key;
        Some(val)
    }

    fn contains(&self, key: u32) -> bool {
        self.get(key).is_some()
    }
}

#[derive(Debug)]
pub enum SlabTreeError {
    OutOfSpace,
}

impl Slab {
    fn root(&self) -> Option<NodeHandle> {
        if self.header().leaf_count == 0 {
            return None;
        }

        Some(self.header().root_node)
    }

    fn find_min_max(&self, find_max: bool) -> Option<NodeHandle> {
        let mut root: NodeHandle = self.root()?;

        loop {
            let root_contents = self.get(root).unwrap();
            match root_contents.case().unwrap() {
                NodeRef::Inner(&InnerNode { children, .. }) => {
                    root = children[if find_max { 1 } else { 0 }];
                    continue;
                }
                _ => return Some(root),
            }
        }
    }

    #[inline]
    pub fn find_min(&self) -> Option<NodeHandle> {
        self.find_min_max(false)
    }

    #[inline]
    pub fn find_max(&self) -> Option<NodeHandle> {
        self.find_min_max(true)
    }

    #[inline]
    pub fn insert_leaf(
        &mut self,
        new_leaf: &LeafNode,
    ) -> Result<(NodeHandle, Option<LeafNode>), SlabTreeError> {
        let mut root: NodeHandle = match self.root() {
            Some(h) => h,
            None => {
                // create a new root if none exists
                match self.insert(new_leaf.as_ref()) {
                    Ok(handle) => {
                        self.header_mut().root_node = handle;
                        self.header_mut().leaf_count = 1;
                        return Ok((handle, None));
                    }
                    Err(()) => return Err(SlabTreeError::OutOfSpace),
                }
            }
        };
        loop {
            // check if the new node will be a child of the root
            let root_contents = *self.get(root).unwrap();
            let root_key = root_contents.key().unwrap();
            if root_key == new_leaf.key {
                // This should never happen
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

            let new_leaf_handle =
                self.insert(new_leaf.as_ref()).map_err(|()| SlabTreeError::OutOfSpace)?;
            let moved_root_handle = match self.insert(&root_contents) {
                Ok(h) => h,
                Err(()) => {
                    self.remove(new_leaf_handle).unwrap();
                    return Err(SlabTreeError::OutOfSpace);
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
            self.header_mut().leaf_count += 1;
            return Ok((new_leaf_handle, None));
        }
    }

    #[cfg(test)]
    fn find_by_key(&self, search_key: i128) -> Option<NodeHandle> {
        let mut node_handle: NodeHandle = self.root()?;
        loop {
            let node_ref = self.get(node_handle).unwrap();
            let node_prefix_len = node_ref.prefix_len();
            let node_key = node_ref.key().unwrap();
            let common_prefix_len = (search_key ^ node_key).leading_zeros();
            if common_prefix_len < node_prefix_len {
                return None;
            }
            match node_ref.case().unwrap() {
                NodeRef::Leaf(_) => break Some(node_handle),
                NodeRef::Inner(inner) => {
                    let crit_bit_mask = (1i128 << 127) >> node_prefix_len;
                    let _search_key_crit_bit = (search_key & crit_bit_mask) != 0;
                    node_handle = inner.walk_down(search_key).0;
                    continue;
                }
            }
        }
    }

    #[inline]
    pub fn remove_by_key(&mut self, search_key: i128) -> Option<LeafNode> {
        let mut parent_h = self.root()?;
        let mut child_h;
        let mut crit_bit;
        match self.get(parent_h).unwrap().case().unwrap() {
            NodeRef::Leaf(&leaf) if leaf.key == search_key => {
                let header = self.header_mut();
                assert_eq!(header.leaf_count, 1);
                header.root_node = 0;
                header.leaf_count = 0;
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
        self.header_mut().leaf_count -= 1;
        Some(cast(self.remove(child_h).unwrap()))
    }

    #[inline]
    pub fn remove_min(&mut self) -> Option<LeafNode> {
        self.remove_by_key(self.get(self.find_min()?)?.key()?)
    }

    #[inline]
    pub fn remove_max(&mut self) -> Option<LeafNode> {
        self.remove_by_key(self.get(self.find_max()?)?.key()?)
    }
}

pub const MAX_BOOK_NODES: usize = 1024;

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
    #[allow(unused)]
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MerpsResult<RefMut<'a, Self>> {
        // TODO
        Ok(Self::load_mut(account)?)
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
    pub fn find_min_max(&self, find_max: bool) -> Option<NodeHandle> {
        let mut root: NodeHandle = self.root()?;

        let i = if find_max { 1 } else { 0 };
        loop {
            let root_contents = self.get(root).unwrap();
            match root_contents.case().unwrap() {
                NodeRef::Inner(&InnerNode { children, .. }) => {
                    root = children[i];
                    continue;
                }
                _ => return Some(root),
            }
        }
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
}

pub struct Book<'a> {
    pub bids: RefMut<'a, BookSide>,
    pub asks: RefMut<'a, BookSide>,
}

impl<'a> Book<'a> {
    pub fn get_best_ask(&self) -> Option<NodeHandle> {
        self.asks.find_max()
    }

    pub fn get_best_bid(&self) -> Option<NodeHandle> {
        self.bids.find_min()
    }
    pub fn new_bid(
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
            OrderType::ImmediateOrCancel => (false, false),
            OrderType::PostOnly => (true, true),
        };
        let order_id = market.gen_order_id(Side::Bid, price);

        // if post only and price >= best_ask, return
        // Iterate through book and match against this new bid
        let mut rem_quantity = quantity; // base lots (aka contracts)
        let mut quote_used = 0; // quote lots
        while rem_quantity > 0 {
            let best_ask_h = match self.get_best_ask() {
                None => {
                    break;
                }
                Some(h) => h,
            };

            let best_ask = self.asks.get_mut(best_ask_h).unwrap().as_leaf_mut().unwrap();
            let best_ask_price = best_ask.price();
            if price < best_ask_price {
                break;
            }

            let match_quantity = rem_quantity.min(best_ask.quantity);
            rem_quantity -= match_quantity;
            quote_used += match_quantity * best_ask_price;
            best_ask.quantity -= match_quantity;

            // TODO fill out FillEvent
            let maker_fill = FillEvent { event_type: EventType::Fill as u8, padding: [0; 7] };
            event_queue.push_back(cast(maker_fill)).unwrap();

            // This fill is not necessary, purely for stats purposes
            let taker_fill = FillEvent { event_type: EventType::Fill as u8, padding: [0; 7] };
            event_queue.push_back(cast(taker_fill)).unwrap();

            if best_ask.quantity == 0 {
                // Create an Out event
                let event = OutEvent { event_type: EventType::Out as u8, padding: [0; 7] };
                event_queue.push_back(cast(event)).unwrap();
                // Remove the order from the book
                // self.asks.remove_by_key(best_ask.key).unwrap();
            }
        }

        // If there are still quantity unmatched, place on the book
        if rem_quantity > 0 {
            let new_bid = LeafNode {
                tag: NodeTag::LeafNode as u32,
                owner_slot: 0, // TODO
                padding: [0; 3],
                key: order_id,
                owner: *merps_account_pk,
                quantity: rem_quantity,
                client_order_id,
            };

            merps_account.add_perp_bid(&new_bid)?;
            self.bids.insert_leaf(&new_bid).unwrap();
        }

        // Edit merps_account if some contracts were matched
        if rem_quantity < quantity {
            /*
                How to adjust the funding settled
                FS_t = (FS_t-1 - FE) * C_t-1 / C_t + FE
            */

            let base_position = merps_account.base_positions[market_index];

            merps_account.base_positions[market_index] += quantity - rem_quantity; // TODO make these checked
            merps_account.quote_positions[market_index] -= quote_used;

            merps_account.funding_settled[market_index] =
                ((merps_account.funding_settled[market_index] - market.total_funding)
                    * I80F48::from_num(base_position)
                    / I80F48::from_num(merps_account.base_positions[market_index]))
                    + market.total_funding;

            market.open_interest += quantity - rem_quantity;
        }

        Ok(())
    }
}
