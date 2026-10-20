# PrediFi Contract Event Schema Reference

**Schema version: `1`** (mirrored by the on-chain constant `EVENT_SCHEMA_VERSION` in
`contract/contracts/predifi-contract/src/constants.rs`)

This document is the **stable contract** between the `predifi-contract` Soroban smart
contract and every off-chain consumer: indexers, Horizon/Soroban RPC streamers, subgraphs,
analytics pipelines, notification services, and security monitors.

It describes, for the pool lifecycle, prediction, resolution, and payout event families:

1. The **topic** used to filter each event (the stable discriminator).
2. The **payload fields** and their exact Soroban types.
3. The **compatibility rules** used to evolve schemas without breaking consumers.
4. The **stable identifiers** that make indexing idempotent and replay-safe.

The authoritative Rust definitions live in
`contract/contracts/predifi-contract/src/lib.rs` (the emitting definitions) and
`contract/contracts/predifi-contract/src/events.rs` (documentation-oriented definitions).
The golden tests in
`contract/contracts/predifi-contract/src/event_schema_tests.rs` pin the topics and payload
shapes described here; a change that breaks this document must fail those tests.

---

## 1. Scope

| Category | Purpose | Topics in this reference |
| --- | --- | --- |
| Pool lifecycle | Creation, liquidity, staking windows, readiness, cancellation | `init`, `pool_created`, `initial_liquidity_provided`, `staking_closed`, `pool_ready`, `pool_canceled`, `pool_disputed`, `stake_limits_updated`, `pool_description_updated`, `max_stake_increased`, `outcome_stakes_updated` |
| Prediction | Stake placement and prediction-side signals | `prediction_placed`, `high_value_prediction`, `prediction_blocked_delisted`, `referrer_updated` |
| Resolution | Outcome determination (operator, oracle, and price-feed paths) | `resolution_vote_cast`, `resolution_conflict`, `pool_resolved`, `pool_resolved_diag`, `oracle_resolved`, `price_feed_updated`, `price_condition_set`, `price_resolved`, `oracle_init` |
| Payout | Claims, refunds, referrals, and treasury movements | `winnings_claimed`, `reward_claimed`, `refund_claimed`, `referral_paid`, `treasury_withdrawn`, `emergency_withdraw` |

Administrative/governance and monitoring/alert events (for example `pause`,
`fee_update`, `unauthorized_admin_op`, `double_claim_attempt`) are documented in the
`events.rs` module header and are intentionally out of scope here; they follow the same
versioning and idempotency rules.

---

## 2. Event envelope

Every Soroban event is emitted as:

```text
(contract_id: Address, topics: Vec<Val>, data: Val)
```

For PrediFi contract events:

- **`topics[0]`** is always the event **topic** — a `Symbol` such as `pool_created`.
  This is the stable discriminator that consumers filter on.
- **`data`** is a map from field-name `Symbol` to field value (`data_format = map`, the
  `#[contractevent]` default). Field **names** in the map are stable; field **order** is
  not guaranteed.
- The envelope also carries `ledger`, `ledger_close_time`, `transaction_hash`, and the
  per-transaction `event_index`, supplied by the network rather than by the contract.

---

## 3. Versioning & compatibility rules

### 3.1 What is versioned

`EVENT_SCHEMA_VERSION` identifies the shape of **all** PrediFi event payloads as a set.
The current value is **`1`**. It is bumped according to the rules below and is **not**
embedded in topic names — topic names stay stable so existing filters keep working.

### 3.2 Allowed (non-breaking) changes

These may ship without a schema-version bump:

- Adding a **new** event topic.
- Adding a **new** optional field to an existing event, provided it is appended to the
  data map and consumers ignore unknown keys.
- Clarifying documentation, comments, or descriptions.
- Adding a new value to an enumerated `Symbol`/`String` field when the field is
  documented as open-ended (e.g. `pool_canceled.reason`).

### 3.3 Breaking changes

The following require a **major** `EVENT_SCHEMA_VERSION` bump, a changelog entry, a
deprecation window, and a migration note for indexers:

- Renaming or removing a topic.
- Renaming, removing, or reordering the meaning of a payload field.
- Changing a field's **type** (e.g. `u32` → `u64`, `Address` → `String`) or its scale.
- Changing the `data_format` of an event.
- Changing the semantics of an existing event (e.g. what triggers it, or the meaning of
  a numeric field).

### 3.4 Deprecation policy

1. A deprecated topic keeps being emitted unchanged for at least one release and is
   marked `DEPRECATED` in the catalog below with a replacement topic.
2. The replacement topic is added first (additive change); consumers migrate at their own
   pace.
3. The deprecated topic is removed only in a release that bumps the major schema version.

### 3.5 Consumer guidance

- **Filter on `topics[0]`; read fields by name.** Never depend on map ordering.
- **Tolerate unknown fields.** Store raw payloads (or hashes) so you can reprocess.
- **Fail open on unknown topics.** A new topic is additive, not an error.
- **Pin the schema version you index.** Record `EVENT_SCHEMA_VERSION` alongside your
  cursor so a future major bump is detectable.

---

## 4. Stable identifiers & idempotent indexing

Indexers must be resilient to duplicate delivery and re-org replay. Each event carries a
**stable identifier** (business key) and is delivered inside an envelope with a
**transport identity**.

### 4.1 Transport identity (dedupe keys)

Build a globally unique, deterministic key from the network envelope, not from wall-clock
time or arrival order:

```text
<contract_id>:<ledger>:<transaction_hash>:<event_index>
```

This is the primary dedupe key for every event.

### 4.2 Business keys (ordering keys)

Within a contract the following business keys order and scope state changes. They are
stable across replays and are the identifiers to use when projecting state.

| Scope | Business key | Used by |
| --- | --- | --- |
| Protocol | `pool_id` | all pool-scoped events |
| Prediction | `(pool_id, user)` | `prediction_placed`, `winnings_claimed`, `reward_claimed`, `refund_claimed` |
| Outcome | `(pool_id, outcome)` | `pool_resolved`, `pool_resolved_diag`, `resolution_vote_cast` |
| Referral | `(pool_id, referrer, referred_user)` | `referral_paid` |
| Treasury | `(token, recipient)` | `treasury_withdrawn`, `emergency_withdraw` |
| Oracle feed | `feed_pair` | `price_feed_updated`, `price_condition_set`, `price_resolved` |
| Governance | singleton (no key field) | `init`, `oracle_init`, `treasury_update` |

### 4.3 Recommended dedupe algorithm

1. Drop the event if `<contract_id>:<ledger>:<transaction_hash>:<event_index>` was seen.
2. Upsert projections using the business key from §4.2 (`pool_id` for pools,
   `(pool_id, user)` for predictions/claims, `(pool_id, outcome)` for resolutions).
3. Treat `pool_resolved` + `pool_resolved_diag` as one **atomic fact** for a pool: they
   are published in the same transaction and must be applied together.
4. Treat `winnings_claimed` + `reward_claimed` as one atomic fact for `(pool_id, user)`.
5. Only advance the ingest cursor past a ledger once every event in that ledger has been
   applied, so partial failures can be retried safely.

### 4.4 Ordering guarantees within one transaction

PrediFi publishes related events in a deterministic order. Consumers may rely on:

| Flow | Emitted order |
| --- | --- |
| Pool creation with seed liquidity | `pool_created` → `initial_liquidity_provided` |
| Operator resolution reaching threshold | `resolution_vote_cast` (per vote) → `pool_resolved` → `pool_resolved_diag` |
| Winning claim with a referrer | `referral_paid` → `winnings_claimed` → `reward_claimed` |
| Cancelled-pool refund | `winnings_claimed` → `reward_claimed` (`claim_type = "winnings"`) → (refund path) `refund_claimed` → `reward_claimed` (`claim_type = "refund"`) |

---

## 5. Field type reference

All payload values use the standard Soroban scalar encodings:

| Type | Soroban encoding | Notes |
| --- | --- | --- |
| `Address` | `Address` | Account (`G...`) or contract (`C...`) strkey. |
| `Symbol` | `Symbol` | ≤ 32 chars, `a-zA-Z0-9_`. Used for topics and short labels. |
| `String` | `String` | UTF-8. |
| `u32` | `U32` | Unsigned 32-bit. |
| `u64` | `U64` | Unsigned 64-bit. Timestamps are Unix seconds. |
| `i128` | `I128` | Signed 128-bit. Token amounts are in the token's base units (stroops for XLM). |
| `BytesN<32>` | `Bytes` (32 bytes) | Fixed-length hash. |
| `Vec<String>` | `Vec` | Ordered list of UTF-8 strings. |
| `Option<Address>` | `Address` / `Void` | `Some` encodes as the inner value, `None` as `Void`. |

---

## 6. Event catalog

### 6.1 Pool lifecycle

#### `init` — `InitEvent`

Emitted exactly once by `init`. Marks protocol genesis and establishes initial governance
parameters.

| Field | Type | Description |
| --- | --- | --- |
| `access_control` | `Address` | Access-control contract managing roles. |
| `treasury` | `Address` | Initial treasury fee recipient. |
| `fee_bps` | `u32` | Default protocol fee in basis points. |
| `resolution_delay` | `u64` | Seconds after `end_time` before resolution is allowed. |
| `min_pool_duration` | `u64` | Minimum pool duration in seconds. |
| `max_predictions_per_user` | `u32` | Per-user prediction cap (`0` = unlimited). |

**Business key:** protocol singleton. **Idempotency:** emitted once per contract; ignore
any subsequent `init` attempt (it panics before emitting).

#### `pool_created` — `PoolCreatedEvent`

Emitted by `create_pool` for every new market.

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Monotonic pool identifier. |
| `creator` | `Address` | Pool creator. |
| `end_time` | `u64` | Unix time when staking closes. |
| `token` | `Address` | Staking token (SAC) address. |
| `options_count` | `u32` | Number of outcomes. |
| `metadata_url` | `String` | Off-chain metadata URI. |
| `initial_liquidity` | `i128` | Seed liquidity deposited by the creator. |
| `category` | `Symbol` | Market category tag. |
| `required_resolutions` | `u32` | Votes required to finalise resolution. |
| `max_total_stake` | `i128` | Total stake cap (`0` = unlimited). |
| `outcome_descriptions` | `Vec<String>` | Human-readable outcome labels. |

**Business key:** `pool_id`. **Ordering:** precedes `initial_liquidity_provided` in the
same transaction when seed liquidity is provided.

#### `initial_liquidity_provided` — `InitialLiquidityProvidedEvent`

Emitted by `create_pool` when `initial_liquidity > 0`.

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Pool receiving liquidity. |
| `creator` | `Address` | Liquidity provider. |
| `amount` | `i128` | Seed liquidity amount. |

**Business key:** `pool_id`.

#### `staking_closed` — `StakingClosedEvent`

Emitted by `close_staking` on the first successful call after `end_time`. Emitted **at
most once per pool** (guarded by the `StakingClosed(pool_id)` sentinel).

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Pool whose staking window closed. |
| `end_time` | `u64` | The pool's configured `end_time`. |
| `total_stake` | `i128` | Total stake locked at close. |
| `timestamp` | `u64` | Ledger time at close. |

**Business key:** `pool_id`.

#### `pool_ready` — `PoolReadyForResolutionEvent`

Emitted by `mark_pool_ready` once the resolution delay has elapsed. **At most once per
pool.**

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Pool ready for resolution. |
| `timestamp` | `u64` | Ledger time when readiness was marked. |

**Business key:** `pool_id`.

#### `pool_canceled` — `PoolCanceledEvent`

Emitted by `cancel_pool` / emergency cancellation.

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Cancelled pool. |
| `caller` | `Address` | Address that initiated the cancellation. |
| `reason` | `String` | Human-readable cancellation reason (open-ended). |
| `operator` | `Address` | Operator/admin confirming cancellation. |

**Business key:** `pool_id`.

#### `pool_disputed` — `PoolDisputedEvent`

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Disputed pool. |
| `moderator` | `Address` | Moderator raising the dispute. |
| `reason` | `String` | Dispute reason. |

**Business key:** `pool_id`.

#### `stake_limits_updated` — `StakeLimitsUpdatedEvent`

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Target pool. |
| `operator` | `Address` | Operator applying the update. |
| `min_stake` | `i128` | New per-prediction minimum. |
| `max_stake` | `i128` | New per-prediction maximum (`0` = unlimited). |

**Business key:** `pool_id`.

#### `pool_description_updated` — `PoolDescriptionUpdatedEvent`

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Target pool. |
| `caller` | `Address` | Creator or admin applying the update. |
| `new_description` | `String` | Replacement description. |

**Business key:** `pool_id`.

#### `max_stake_increased` — `MaxTotalStakeIncreasedEvent`

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Target pool. |
| `creator` | `Address` | Creator raising the cap. |
| `new_max_total_stake` | `i128` | New cap (`0` = unlimited). |

**Business key:** `pool_id`.

#### `outcome_stakes_updated` — `OutcomeStakesUpdatedEvent`

Emitted for markets with 16+ outcomes to summarise a bulk stake update.

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Target pool. |
| `options_count` | `u32` | Number of outcomes. |
| `total_stake` | `i128` | Total stake after the update. |

**Business key:** `pool_id`.

---

### 6.2 Prediction

#### `prediction_placed` — `PredictionPlacedEvent`

Emitted by `place_prediction` for every accepted stake.

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Target pool. |
| `user` | `Address` | Predictor. |
| `amount` | `i128` | Staked amount. |
| `outcome` | `u32` | Selected outcome index (0-based). |

**Business key:** `(pool_id, user)`. **Ordering:** emitted before any supplementary
`high_value_prediction` / `outcome_stakes_updated` event for the same stake.

#### `high_value_prediction` — `HighValuePredictionEvent`

Supplementary alert emitted when `amount >= HIGH_VALUE_THRESHOLD` (see `constants.rs`).

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Target pool. |
| `user` | `Address` | Predictor. |
| `amount` | `i128` | Staked amount. |
| `outcome` | `u32` | Selected outcome index. |
| `threshold` | `i128` | Threshold that was breached. |

**Business key:** `(pool_id, user)`.

#### `prediction_blocked_delisted` — `PredictionBlockedDelistedEvent`

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Target pool. |
| `user` | `Address` | Predictor that was blocked. |
| `token` | `Address` | De-listed token. |
| `timestamp` | `u64` | Ledger time of the rejection. |

**Business key:** `(pool_id, user)`.

#### `referrer_updated` — `ReferrerUpdatedEvent`

| Field | Type | Description |
| --- | --- | --- |
| `user` | `Address` | User whose referrer mapping changed. |
| `pool_id` | `u64` | Target pool. |
| `new_referrer` | `Option<Address>` | New referrer, or `None` if removed. |

**Business key:** `(pool_id, user)`.

---

### 6.3 Resolution

#### `resolution_vote_cast` — `ResolutionVoteCastEvent`

Emitted for **every** operator vote, whether or not the threshold is reached.

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Target pool. |
| `voter` | `Address` | Operator casting the vote. |
| `outcome` | `u32` | Voted outcome. |
| `vote_count` | `u32` | Votes recorded for `outcome` after this vote. |
| `required_resolutions` | `u32` | Threshold required to finalise. |

**Business key:** `(pool_id, outcome)`.

#### `resolution_conflict` — `ResolutionConflictEvent`

Emitted when an operator votes for a different outcome than an earlier vote.

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Target pool. |
| `oracle` | `Address` | Voter whose vote conflicted. |
| `outcome` | `u32` | Conflicting outcome. |
| `existing_outcome` | `u32` | Previously recorded outcome. |

**Business key:** `(pool_id, outcome)`.

#### `pool_resolved` — `PoolResolvedEvent`

Emitted by `resolve_pool` / `oracle_resolve` / `resolve_pool_from_price` when the pool
finalises.

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Resolved pool. |
| `operator` | `Address` | Address that finalised resolution (operator, oracle, or contract). |
| `outcome` | `u32` | Winning outcome index. |

**Business key:** `(pool_id, outcome)`. **Ordering:** immediately followed by
`pool_resolved_diag` in the same transaction.

#### `pool_resolved_diag` — `PoolResolvedDiagEvent`

Diagnostics published atomically with `pool_resolved`.

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Resolved pool. |
| `outcome` | `u32` | Winning outcome index. |
| `total_stake` | `i128` | Total stake at resolution. |
| `winning_stake` | `i128` | Stake on the winning outcome (`0` => no winners). |
| `timestamp` | `u64` | Ledger time at resolution. |

**Business key:** `(pool_id, outcome)`.

#### `oracle_resolved` — `OracleResolvedEvent`

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Resolved pool. |
| `oracle` | `Address` | Oracle submitting the outcome. |
| `outcome` | `u32` | Submitted outcome. |
| `proof` | `String` | Proof or URI backing the outcome. |

**Business key:** `(pool_id, outcome)`.

#### `price_feed_updated` — `PriceFeedUpdatedEvent`

| Field | Type | Description |
| --- | --- | --- |
| `oracle` | `Address` | Oracle submitting the update. |
| `feed_pair` | `Symbol` | Asset pair (e.g. `BTCUSD`). |
| `price` | `i128` | Observed price (fixed precision). |
| `confidence` | `i128` | Confidence interval margin. |
| `timestamp` | `u64` | Observation time. |
| `expires_at` | `u64` | Expiry time of the data point. |

**Business key:** `feed_pair`.

#### `price_condition_set` — `PriceConditionSetEvent`

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Target pool. |
| `feed_pair` | `Symbol` | Asset pair evaluated. |
| `target_price` | `i128` | Target price threshold. |
| `operator` | `u32` | Comparison operator flag. |
| `tolerance_bps` | `u32` | Tolerance in basis points. |

**Business key:** `pool_id`.

#### `price_resolved` — `PriceResolvedEvent`

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Resolved pool. |
| `feed_pair` | `Symbol` | Asset pair evaluated. |
| `current_price` | `i128` | Price at resolution. |
| `target_price` | `i128` | Target price. |
| `outcome` | `u32` | Winning outcome index. |

**Business key:** `(pool_id, outcome)`.

#### `oracle_init` — `OracleInitEvent`

| Field | Type | Description |
| --- | --- | --- |
| `admin` | `Address` | Admin configuring the oracle. |
| `pyth_contract` | `Address` | Pyth contract address. |
| `max_price_age` | `u64` | Maximum accepted price staleness (seconds). |
| `min_confidence_ratio` | `u32` | Minimum confidence ratio (bps). |

**Business key:** protocol singleton.

---

### 6.4 Payout

#### `winnings_claimed` — `WinningsClaimedEvent`

Emitted by `claim_winnings` for both winning payouts and the cancelled-pool
claim path.

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Pool claimed from. |
| `user` | `Address` | Recipient. |
| `amount` | `i128` | Net amount paid to the user. |

**Business key:** `(pool_id, user)`.

#### `reward_claimed` — `RewardClaimedEvent`

Companion event to `winnings_claimed` and `refund_claimed`, carrying an explicit
`claim_type`.

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Pool claimed from. |
| `user` | `Address` | Recipient. |
| `amount` | `i128` | Amount paid to the user. |
| `claim_type` | `String` | `"winnings"` for resolved/cancelled-claim path, `"refund"` for `claim_refund`. |

**Business key:** `(pool_id, user)`.

#### `refund_claimed` — `RefundClaimedEvent`

Emitted by `claim_refund` for a cancelled pool.

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Cancelled pool. |
| `user` | `Address` | Recipient. |
| `amount` | `i128` | Refunded stake (100% of stake). |

**Business key:** `(pool_id, user)`.

#### `referral_paid` — `ReferralPaidEvent`

Emitted during `claim_winnings` when the claimant has a referrer and a referral reward is
paid.

| Field | Type | Description |
| --- | --- | --- |
| `pool_id` | `u64` | Pool where the referred user predicted. |
| `referrer` | `Address` | Reward recipient. |
| `referred_user` | `Address` | User whose claim generated the fee. |
| `amount` | `i128` | Referral reward amount. |

**Business key:** `(pool_id, referrer, referred_user)`.

#### `treasury_withdrawn` — `TreasuryWithdrawnEvent`

| Field | Type | Description |
| --- | --- | --- |
| `admin` | `Address` | Admin authorising the withdrawal. |
| `token` | `Address` | Token withdrawn. |
| `amount` | `i128` | Amount withdrawn. |
| `recipient` | `Address` | Funds recipient. |
| `remaining_balance` | `i128` | Treasury balance after withdrawal. |
| `timestamp` | `u64` | Ledger time of the withdrawal. |

**Business key:** `(token, recipient)`.

#### `emergency_withdraw` — `EmergencyWithdrawEvent`

| Field | Type | Description |
| --- | --- | --- |
| `admin` | `Address` | Admin authorising the withdrawal. |
| `token` | `Address` | Token withdrawn. |
| `destination` | `Address` | Funds destination. |
| `amount` | `i128` | Amount withdrawn. |

**Business key:** `(token, destination)`.

---

## 7. Validation

The schema described above is enforced by golden tests in
`contract/contracts/predifi-contract/src/event_schema_tests.rs`:

- **Topic catalog** — asserts the exact `Symbol` used as `topics[0]` for every event in
  the four categories.
- **Payload shape** — asserts the exact set of field names and their Soroban types for
  representative lifecycle, prediction, resolution, and payout flows.
- **Golden lifecycle snapshots** — runs full flows end-to-end and compares the ordered
  `topic{field:type,...}` snapshot against a checked-in golden constant, so any drift in
  topic names, field names, or field types fails CI.

Run them with:

```bash
cd contract
cargo test -p predifi-contract event_schema
```

Any intentional schema change must update this document, the golden constants, and bump
`EVENT_SCHEMA_VERSION` when the change is breaking (see §3).

---

## 8. Version history

| Schema version | Contract version | Notes |
| --- | --- | --- |
| `1` | `1` | Initial published event schema for pool lifecycle, prediction, resolution, and payout events. |
