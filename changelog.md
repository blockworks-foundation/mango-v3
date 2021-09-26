# Mango Program Change Log

## v3.1.0
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

## v3.0.5
Deployed: September 26, 16:40 UTC
1. Fixed bug in check_enter_bankruptcy
2. updated anchor version and packages
