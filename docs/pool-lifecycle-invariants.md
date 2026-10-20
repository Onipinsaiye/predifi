# Pool lifecycle invariants

This document describes the lifecycle enforced by `predifi-contract`. A pool is
stored in `MarketState::Active` when created. The contract does not create a
separate `Closed` state: the entry cutoff is derived from `end_time`, while
`close_staking` emits an explicit, idempotent signal for indexers and keepers.

## Normative transition matrix

| Transition | Caller | Time and preconditions | Storage effects | Event | Repeated/invalid behavior |
| --- | --- | --- | --- | --- | --- |
| Create → Active | Authenticated creator | Token/category and pool configuration are valid; `end_time` is strictly in the future and satisfies the configured minimum duration | Creates the pool and outcome-stake storage; optionally transfers initial liquidity; adds the pool to the active index | `PoolCreatedEvent`; `InitialLiquidityProvidedEvent` when applicable | Invalid configuration is rejected before pool storage is written |
| Active → Entry accepted | Authenticated user | `timestamp < end_time`; pool is active; outcome, token, stake, whitelist, and capacity checks pass | Transfers stake, updates total/outcome stake, prediction, participant, and index storage | `PredictionPlacedEvent` and applicable diagnostic events | At or after `end_time`, entry is rejected; failed validation does not represent a successful prediction |
| Active → Staking closed signal | Permissionless caller | `timestamp >= end_time`; pool remains active | Writes `StakingClosed(pool_id)` once | `StakingClosedEvent` once | Before the cutoff returns `StakingStillOpen`; later calls are idempotent |
| Active → Ready signal | Permissionless caller | Staking-closed sentinel exists and `timestamp >= end_time + resolution_delay` | Writes `PoolReady(pool_id)` once | `PoolReadyForResolutionEvent` once | Missing closure, early calls, or terminal pools are rejected; repeated calls are idempotent |
| Active → Resolved | Authenticated operator/oracle path | Pool is active; resolution delay has elapsed; outcome is valid; operator vote/price-oracle rules pass | Records votes; at threshold sets state, outcome, fee, resolution timestamp, and removes active index | Resolution vote/conflict events; `PoolResolvedEvent` on finalization | Duplicate votes, conflicts, invalid outcomes, early calls, and terminal pools are rejected |
| Active → Canceled | Authorized admin/operator/eligible creator or overdue failsafe caller | Pool is active and the caller satisfies the cancellation rule | Sets canceled state and removes active index | `PoolCanceledEvent` | Resolved/canceled pools cannot be canceled again; unauthorized calls do not mutate the pool |
| Resolved → Claimed | Authenticated claimant | Pool is resolved, claim is within the claim window, and the prediction has not been claimed | Writes the one-way claimed sentinel and transfers the calculated payout | Winnings/reward events | Replays return `AlreadyClaimed`; losing or absent predictions pay zero |
| Canceled → Refunded | Authenticated claimant | Pool is canceled, prediction is positive, and the prediction has not been claimed | Writes the one-way claimed sentinel and transfers the original stake | Refund/reward events | Replays and non-canceled pools are rejected |

## Invariants

1. **One-way pool state:** only `Active → Resolved` or `Active → Canceled` is
   legal. Terminal states cannot be reopened or changed to another terminal
   state.
2. **Cutoff:** prediction placement requires the ledger timestamp to be before
   `end_time`; the exact cutoff is therefore closed to new entries.
3. **Resolution delay:** resolution requires the inclusive boundary
   `timestamp >= end_time + resolution_delay`.
4. **Single claim:** `Claimed(user, pool_id)` is write-once for winnings and
   refunds, preventing payout replay.
5. **No value creation:** payout arithmetic rejects invalid values and asserts
   that a payout cannot exceed the pool total stake.
6. **Failure isolation:** authorization, state, time, and input checks occur
   before lifecycle state is finalized or an event for a successful transition
   is emitted. Soroban transaction failure also reverts the invocation; callers
   should still assert observable state when testing rejected operations.

The implementation and boundary suites are the source of truth for behavior;
this document does not introduce a new `Closed` state or make the
`PoolReady`/`StakingClosed` indexer signals additional preconditions for the
existing resolution entry points.
