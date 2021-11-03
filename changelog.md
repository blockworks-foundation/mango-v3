# Mango Program Change Log

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
