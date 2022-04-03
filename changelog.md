# Mango Program Change Log

## v3.4.3
Deployed: Apr 3, 2022 at 19:00:13 UTC | Slot: 128,066,047
1. SettleFees will return Ok and fail silently if not pnl negative and fees accrued positive
2. Use pyth_client crate instead of copy pasta
3. Remove pyth status check and just rely on confidence intervals

## v3.4.2
Deployed: Mar 19, 2022 at 16:22:04 UTC | Slot: 125,699,230
1. Increase PriceCache and PerpMarketCache validity by 2x
2. Upgrade anchor to 0.22.1 and upgrade all other packages
3. Move interest rate calculation outside of RootBank

## v3.4.1
Deployed: Feb 28, 2022 at 15:43:29 UTC | Slot: 122,878,778
1. Fix div by zero bug in market order sells

## v3.4.0
Deployed: Feb 28, 2022 at 13:57:00 UTC | Slot: 122,868,568
1. Breaking change: Orders on the perp orderbook can now expire.
   Either use the iterator that returns only valid orders or manually filter out invalid orders.
2. New instruction: PlacePerpOrder2
   - can set an expiry timestamp
   - can have a quote quantity limit
   - limits the depth of orderbook iteration
3. Reduce heap memory use of event logging: ConsumeEvents limit raised back to 8

## v3.3.5
Deployed: Feb 11, 2022 at 17:36:15 UTC | Slot: 120,380,891 
1. reduce consume_events limit to 4 to prevent memory issues
2. record maker fees on PerpMarket at time of trade to prevent it later going negative
3. fix typo in emit_perp_balances to print correct short_funding

## v3.3.4
Deployed: Feb 11, 2022 at 01:55:57 UTC | Slot: 120,283,217
1. Added three instructions (ChangeReferralFeeParams, SetReferrerMemory, RegisterReferrerId) to help with referral program
2. Assess taker fees at the time of the taker trade
3. Add back Pyth status checks

## v3.3.3
Deployed: Feb 4, 2022 at 01:47:33 UTC | Slot: 119,226,876
1. Pyth status check temporarily removed to let people use accounts with COPE

## v3.3.2
Deployed: Jan 28, 2022 at 20:38:57 UTC | Slot: 118,276,295
1. Fix the bug in cancel_all where some orders weren't canceled
2. Add optional payer account to CreateMangoAccount and CreateSpotOpenOrders for better composability
3. Clean up iteration code and add better documentation

## v3.3.1
Deployed: Jan 18, 2022 at 21:06:57 UTC | Slot: 116,847,318
1. Check quote token vault inside resolve_token_bankruptcy
2. Add checked to num for general safety

## v3.3.0
Deployed: Jan 17, 2022 at 00:45:05 UTC | Slot: 116,585,405
1. CancelAllPerpOrdersSide - cancels all order on one side
2. CloseMangoAccount - close account and retrieve lamports
3. ResolveDust - settle anything below 1 native SPL against the dust account
4. CreateDustAccount - create a PDA tied to the MangoGroup useful for settling dust against
5. SetDelegate - delegate authority to operate MangoAccount to another account
6. upgrade packages
7. impose price limits for placing limit orders
8. ChangeSpotMarketParams
9. CreateSpotOpenOrders using PDA for better UX

## v3.2.16
Deployed: Jan 11, 2022 at 01:59:05 UTC | Slot: 115,691,635
1. Checked math in all areas touched by place_perp_order

## v3.2.15
Deployed: Jan 10, 2022 at 22:00:54 UTC | Slot: 115,666,186
1. Impose price limits on spot orders

## v3.2.14
Deployed: Jan 2, 2022 at 20:48:01 UTC | Slot: 114,518,931
1. Check bids and asks when loading perp market book

## v3.2.13
Deployed: Dec 16, 2021 at 21:16:50 UTC | Slot: 111,865,268
1. Fixed FillLog maker_fee and taker_fee
2. Now logging order id in new_bid and new_ask

## v3.2.12
Deployed: Dec 16, 2021 at 16:15:19 UTC | Slot: 111,832,202
1. Add CancelAllPerpOrdersLog to mango_logs and start logging cancel_all_with_size_incentives
2. For reduce_only on perp orders, now checking base position that's sitting on EventQueue unprocessed
2. Fix bug in check_exit_bankruptcy; now checking all borrows

## v3.2.11
Deployed: Dec 9, 2021 at 18:59:28 UTC | Slot: 110,796,491
1. Fixed bug where perp limit orders past price limit would fail due to simulation
2. Remove unnecessary Rent account in InitMangoAccount

## v3.2.10
Deployed: Dec 9, 2021 at 01:49:38 UTC | Slot: 110,691,491
1. Limit placing bids to oracle + maint margin req and asks to oracle - maint margin req
2. Add more checked math in FillEvent struct method and execute_maker()

## v3.2.9
Deployed: Dec 8, 2021 at 22:29:47 UTC | Slot: 110,669,751
1. Add ChangeMaxMangoAccounts
2. Add some checked math in MangoAccount and matching

## v3.2.8
Deployed: Dec 4, 2021 at 21:04:59 | Slot: 110,056,063
1. Add check to Pyth CachePrice so conf intervals larger than 10% result in no change to cache price

## v3.2.7
Deployed: Nov 30, 2021 at 03:23:08 UTC | Slot: 109,359,973
1. Update margin basket check in ForceCancelSpot
2. Update margin baskets in PlaceSpotOrder and PlaceSpotOrder2; intended to free up unused margin basket elements
3. Allow passing in base_decimals when CreatePerpMarket before AddSpotMarket
4. Make bids and asks pub in Book

## v3.2.6
Deployed: Nov 20, 2021 at 20:53:42 UTC | Slot: 107,876,588
1. Checking the owner of OpenOrders accounts now

## v3.2.5
Deployed: Nov 20, 2021 at 14:35:26 UTC | Slot: 107,833,583
1. Fixed init_spot_open_orders bug not checking signer_key
2. checking signer_key wherever it's passed it

## v3.2.4
Deployed: Nov 15, 2021 at 19:38:22 UTC | Slot: 107,052,828
1. Updated the update_margin_basket function to include Serum dex OpenOrders accounts with any open orders.
2. Add instruction UpdateMarginBasket to bring MangoAccount into compliance with new standard

## v3.2.3
Deployed: Deployed: Nov 15, 2021 at 15:25:19 UTC | Slot: 107,024,833
1. Comment out in_margin_basket check in ForceCancelSpot due to to it being wrong for an account 

## v3.2.2
Deployed: Deployed: Nov 7, 2021 at 14:20:04 UTC | Slot: 105,693,864
1. Get rid of destructuring assignment feature
2. Use impact bid/ask for calculating funding (100 contracts)

## v3.2.1
Deployed: Nov 1, 2021 at 18:09:05 UTC | Slot: 104,689,370
1. If perp market is added before spot market, fix decimals to 6
2. remove ChangePerpMarketParams

## v3.2.0
Deployed: Oct 28, 2021 at 23:53:49 UTC | Slot: 104,038,884
1. Added Size LM functionality
2. Added ChangePerpMarketParams2
3. Added CreatePerpMarket which uses PDAs for MNGO vault and PerpMarket
4. Updated to solana 1.8.1 and anchor 0.18.0

## v3.1.4
Deployed: Oct 26, 2021 at 17:04:50 UTC | Slot: 103,646,150
1. fixed bug when book is full 
2. Adjusted max rate adj back to 4 for LM

## v3.1.3
Deployed:
1. Change rate adjustment for liquidity mining to 10 so changes are fast

## v3.1.2
Deployed: Oct 18, 2021 at 22:12:08 UTC | Slot: 102,256,816
1. Allow for 0 max depth bps

## v3.1.1
Deployed: Oct 15, 2021 at 17:45:59 UTC

1. Fixed bug in liquidate_token_and_perp div by zero bug

## v3.1.0
Deployed: Oct 11, 2021 at 16:57:51 UTC
1. Add reduce only to PlacePerpOrder
2. Add Market OrderType to PlacePerpOrder
3. Reduce MAX_IN_MARGIN_BASKET to 9 from 10 to reflect tx size limits
4. Add PlaceSpotOrder2 which is optimized for smaller tx size
5. Add new way to pass in open orders pubkeys to reduce tx size
6. Add InitAdvancedOrders, AddPerpTriggerOrder, RemovePerpTriggerOrder, ExecutePerpTriggerOrder to allow stop loss, stop limit, take profit orders
7. Remove ForceSettleQuotePositions because it mixes in the risk from all perp markets into USDC lending pool
8. All cache valid checks are done independently of one another and have different valid_interval
9. Remove CacheRootBank instruction
10. Add new param for exponent in liquidity incentives
11. FillEvent logging is now done via FillLog borsh serialized and base64 encoded to save compute
12. Added mango-logs and replaced all logging with more efficient Anchor event
13. Added logging of OpenOrders balance to keep full track of acocunt value
14. Added PostOnlySlide for Perp orders (including trigger)
15. Added OrderType into LeafNode for ability to modify order on TradingView
16. Added MngoAccrualLog
17. added DepositLog, WithdrawLog, RedeemMngoLog
18. sending u64::MAX in withdraw function withdraws total amount in deposit
19. UpdateFunding now takes in MangoCache as writable and caches the result and UpdateFundingLog is emitted

## v3.0.6
Deployed: October 5, 2:00 UTC
1. Changed the being_liquidated threshold inside liquidate functions to -1 to account for dust issues.
2. Upgrade anchor version for verifiable build

## v3.0.5
Deployed: September 26, 16:40 UTC
1. Fixed bug in check_enter_bankruptcy
2. updated anchor version and packages
