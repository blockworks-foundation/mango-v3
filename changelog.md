# Mango Program Change Log

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
