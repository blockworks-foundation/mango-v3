use crate::critbit::{LeafNode, NodeHandle, NodeTag, Slab, SlabView};
use crate::error::MerpsResult;
use crate::queue::{EventQueue, EventType, FillEvent, OutEvent};
use crate::state::{MerpsAccount, PerpMarket};
use bytemuck::cast;
use fixed::types::I80F48;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{Deserialize, Serialize};
use solana_program::pubkey::Pubkey;

declare_check_assert_macros!(SourceFileId::Matching);

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

pub struct OrderBook<'a> {
    pub bids: &'a mut Slab,
    pub asks: &'a mut Slab,
}

pub struct BookSide {}

impl<'a> OrderBook<'a> {
    pub fn get_best_bid_price(&self) -> Option<NodeHandle> {
        match self.get_best_bid() {
            None => {}
            Some(h) =>
        }
    }
    pub fn get_best_bid(&self) -> Option<NodeHandle> {
        self.bids.find_max()
    }
    pub fn get_best_ask(&self) -> Option<NodeHandle> {
        self.asks.find_min()
    }
    pub fn new_bid(
        &mut self,
        event_queue: &mut EventQueue,
        market: &mut PerpMarket,
        merps_account: &mut MerpsAccount,
        merps_account_pk: &Pubkey,
        price: u64,
        quantity: i64, // quantity is guaranteed to be greater than zero due to initial check
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
            quote_used += match_quantity * (best_ask_price as i64);
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
            self.bids.insert_leaf(&new_bid).unwrap();
            // TODO adjust merps_account to account for new order and locked funds
        }

        // Edit merps_account if some contracts were matched
        if rem_quantity < quantity {
            /*
                How to adjust the funding settled
                FS_t = (FS_t-1 - FE) * C_t-1 / C_t + FE
            */

            let market_index = 0; // TODO

            let base_position = merps_account.base_positions[market_index];

            merps_account.base_positions[market_index] += quantity - rem_quantity; // TODO make these checked
            merps_account.quote_positions[market_index] -= quote_used;

            merps_account.funding_settled[market_index] =
                ((merps_account.funding_settled[market_index] - market.total_funding)
                    * I80F48::from_num(base_position)
                    / I80F48::from_num(merps_account.base_positions[market_index]))
                    + market.total_funding;

            market.open_interest += I80F48::from_num(base_position);

        }

        Ok(())
    }
}
