# Contract Reference

Complete reference for all PrediFi contract methods, events, and data structures.

## Core Functions

### `init`

Initialize the contract with configuration parameters.

```rust
pub fn init(
    env: Env,
    access_control: Address,
    treasury: Address,
    fee_bps: u32
)
```

**Parameters:**

| Parameter        | Type      | Description                            |
| ---------------- | -------- | ------------------------------------- |
| `access_control` | `Address` | Access control contract address        |
| `treasury`       | `Address` | Treasury address for fee collection    |
| `fee_bps`        | `u32`      | Fee in basis points (max 10000 = 100%) |

**Returns:** None

**Events:** `InitEvent`

**Notes:**

- Idempotent - safe to call multiple times
-Only sets config if not already initialized

---

### `create_pool`

Create a new prediction market pool.

```rust
pub fn create_pool(
    env: Env,
    end_time: u64,
    token: Address,
    description: String,
    metadata_url: String
') -> u64
```

**Parameters:**

| Parameter      | Type      | Description                                  |
| -------------- | --------- | -------------------------------------------- |
| `end_time`     | `u64`      | Unix timestamp after which predictions close |
| `token`        | `Address` | Stellar token contract for staking           |
| `description`  | `String`  | Event description (max 256 bytes)            |
| `metadata_url` | `String`  | Extended metadata URL (max 512 bytes)        |

**Returns:** `u64` - New pool ID

**Events:** `PoolCreatedEvent`

**Validations:**

- (end_time` must be in the future
- `description` length ≤ 256 bytes
- (metadata_url` length ≤ 512 bytes

**Example:**

```rust
let pool_id = contract.create_pool(
    env,
    1735689600, // Dec 31, 2024
    token_address,
    String::from_str(&env, "Will BTC hit $100k?"),
    String::from_str(&env, "ipfs://QmXxx...")
);
```

---

### `place_prediction`

Place a prediction on an active pool. Optionally specify a referrer to earn a share of the protocol fee when the referred user claims winnings.

```rust
pub fn place_prediction(
    env: Env,
    user: Address,
    pool_id: u64,
    amount: i128,
    outcome: u32,
    referrer: Option<Address>
)
```

**Parameters:**

| Parameter  | Type              | Description                                                                             |
| --------- | ---------------- | ------------------------------------------------------------------------------------------------------- |
| `user`    | `Address`         | User placing the prediction                                                                 |
| `pool_id`  | `u64`             | Pool to predict on                                                                      |
| `amount`   | `i128`            | Prediction amount (in token's smallest unit)                                                 |
| `outcome`  | `u32`             | Outcome index (0, 1, 2, etc.)                                                              |
| `referrer` | `Option<Address>` | Optional address that referred this user; stored on first prediction for (user, pool) and used for referral payout on claim |

**Returns:** None

**Events:** `PredictionPlacedEvent`

**Validations:**

- Pool must exist
- Pool must not be resolved
- Current time < pool.end_time
- Amount > 0
- User must have sufficient token balance
- If `referrer` is set: cannot be the user or the contract address

**Token Transfer:**

- Transfers `amount` tokens from user to contract

**Example:**

```rust
contract.place_prediction(
    env,
    user_address,
    pool_id,
    1000000000, // 100 tokens
    1, // Outcome: "Yes"
    Some(referrer_address) // or None
);
```

---

### `resolve_pool`

Resolve a pool with the winning outcome. Requires Operator role (1).

```rust
pub fn resolve_pool(
    env: Env,
    operator: Address,
    pool_id: u64,
    outcome: u32
) -> Result<(), PredifiError>
```

**Parameters:**

| Parameter  | Type      | Description                         |
| --------- | --------- | ------------------------------------------ |
| `operator` | `Address` | Operator address (must have role 1) |
| `pool_id`  | `u64`     | Pool to resolve                         |
| `outcome`  | `u32`     | Winning outcome index                   |

**Returns:** `Result<(), PredifiError>

**Events:** `PoolResolvedEvent`

**Validations:**

- Operator must have role 1
- Pool must exist
- Pool must not already be resolved

**Errors:**

- `Unauthorized` - Operator lacks required role
- `PoolNotFound` - Pool doesn't exist
- `PoolAlreadyResolved` - Pool already resolved

**Example:**

```rust
contract.resolve_pool(
    env,
    operator_address,
    pool_id,
    1 // Winning outcome
)?;
```

---

### `claim_winnings`

Claim winnings from a resolved pool. Protocol fee is deducted from the pool before distribution; if the user was referred, a portion of that fee is paid to the referrer.

```rust
pub fn claim_winnings(
    env: Env,
    user: Address,
    pool_id: u64
) -> Result<i128, PredifiError>
```

**Parameters:**

| Parameter | Type      | Description            |
| -------- | --------- | ---------------------- |
| `user`   | `Address` | User claiming winnings |
| `pool_id` | `u64`      | Pool to claim from     |

**Returns:** `Result<i128, PredifiError>`  - Amount claimed (0 if didn't win)

**Events:** `WinningsClaimedEvent`, `ReferralPaidEvent` (when user had a referrer and wins)

**Validations:**

- Pool must be resolved
- User must not have already claimed
- User must have placed a prediction

**Reward Calculation:**

- Protocol fee = `total_pool_stake × fee_bps / 10_000`
- Payout pool = `total_pool_stake - protocol_fee`
- `winnings = (user_stake / winning_outcome_total_stake) × payout_pool`
- If user has a referrer: referrer receives `referral_cut_bps` of the user's share of the protocol fee (the rest remains for treasury withdrawal).

**Remainder Allocation Invariant:**

Integer division truncates, so per-user payouts may not sum exactly to the payout pool. To keep accounting deterministic and independent of claim order, the contract retains the truncation dust rather than redistributing it:

- `sum(all user winnings) + retained_remainder = payout_pool`
- `retained_remainder = payout_pool - sum(floor(user_stake × payout_pool / winning_outcome_total_stake))`
- The retained remainder stays in escrow and is withdrawable by the treasury; it is never assigned to a specific claimant.
- This rule is applied identically on the single-claim path (`claim_winnings`) and any batch payout path, so the same stake vector always yields the same per-user amounts regardless of claim order.

**Errors:**

- `PoolNotResolved` - Pool not yet resolved
- `AlreadyClaimed` - User already claimed winnings

**Example:**

```rust
let winnings = contract.claim_winnings(
    env,
    user_address,
    pool_id
)?;

if winnings > 0 {
    // User won and received winnings
}
```

**Property Tests:**

Payout property tests generate many participant counts and stake vectors and assert that escrow equals the sum of payouts plus the explicit retained remainder, accounting for every token unit.

---

### `get_user_predictions`

Get a paginated list of a user's predictions.

```rust
pub fn get_user_predictions(
    env: Env,
    user: Address,
    offset: u32,
    limit: u32
) -> Vec<UserPredictionDetail>
```

**Parameters:**

| Parameter | Type      | Description               |
| -------- | --------- | -------------------------- |
| `user`   | `Address` | User address              |
| `offset`  | `u32`      | Pagination offset             |
| `limit`   | `u32`      | Maximum results to return     |

**Returns:** `Vec<UserPredictionDetail>`

**Example:**

```rust
let predictions = contract.get_user_predictions(
    env,
    user_address,
    0,  // offset
    10  // limit
);
```

---

### `get_resolution_evidence`

Get the verifiable evidence committed when a pool was resolved through the
price-feed path (`resolve_pool_from_price`). The record is the hash preimage of
the `evidence_hash` carried by `ResolutionEvidenceEvent`: auditors can re-encode
it to XDR, hash it with SHA-256, and re-derive the outcome from
(`price`, `target_price`, `comparison_op`, `tolerance_bps`) to independently
verify the resolution decision. The record is immutable once written.

```rust
pub fn get_resolution_evidence(
    env: Env,
    pool_id: u64
) -> Option<ResolutionEvidence>
```

**Parameters:**

| Parameter | Type  | Description     |
| --------- | ----- | --------------- |
| `pool_id` | `u64` | Pool to inspect |

**Returns:** `Option<ResolutionEvidence>` — `None` for unresolved pools and for
pools resolved via operator voting or oracle voting (those paths carry no feed
observation to commit).

**Example:**

```rust
if let Some(evidence) = contract.get_resolution_evidence(env, pool_id) {
    // Recompute: sha256(evidence.to_xdr()) == emitted evidence_hash
    // Re-derive: compare evidence.price to evidence.target_price
    //            under evidence.comparison_op within evidence.tolerance_bps
}
```

---

## Admin Functions

### `pause`

Pause operations according to the pause matrix. Requires Admin role (0).

```rust
pub fn pause(env: Env, admin: Address)
```

**Parameters:**

| Parameter | Type      | Description                      |
| -------- | --------- | ------------------------------- |
| `admin`   | `Address` | Admin address (must have role 0) |

**Events:** `PauseEvent`

**Pause Matrix:**

Pausing is scoped by operation. Some entry points are blocked while paused; others remain available so users can always exit safely.

| Operation               | Paused behavior                                                                 |
| ---------------------- | ---------------------------------------------------------------------------------------- |
| `create_pool`           | Blocked (returns `Paused`)                                                              |
| `place_prediction`       | Blocked (returns `Paused`)                                                              |
| `resolve_pool`           | Blocked (returns `Paused`)                                                              |
| `oracle_resolve`        | Blocked (returns `Paused`)                                                              |
| `resolve_pool_from_price` | Blocked (returns `Paused`)                                                            |
| `claim_winnings`         | **Allowed** — eligible claims always remain available                                     |
| `refund`                 | **Allowed** — refunds always remain available                                                   |
| `withdraw_fees`          | Blocked (returns `Paused`)                                                              |
| `set_fee_bps`            | Blocked (returns `Paused`)                                                              |
| `set_referral_cut_bps`  | Blocked (returns `Paused`)                                                              |
| `set_treasury`           | Blocked (returns `Paused`)                                                              |
| `unpause`               | **Allowed** — must always be callable to restore operations                             |
| `view functions`        | **Allowed** — read-only queries are never paused                                             |

**Notes:**

-  `claim_winings` and `refund` are explicitly exempt from pause so users can always withdraw funds they are entitled to.
-  `unpause` and view functions are never guarded by the pause flag.
-  Every guarded entry point returns `Paused` when the contract is paused.

---

### `unpause`

Resume contract operations. Requires Admin role (0).

```rust
pub fn unpause(env: Env, admin: Address)
```

**Parameters:**

| Parameter | Type      | Description                      |
| -------- | --------- | ------------------------------- |
| `admin`   | `Address` | Admin address (must have role 0) |

**Events:** `UnpauseEvent`

**Notes:**

- Always allowed, even when the contract is paused.
- Restores all operations guarded by the pause matrix.

---

### `is_paused`

Expose the current pause state to clients.

```rust
pub fn is_paused(env: Env) -> bool
```

**Returns:** `true` if the contract is paused, `false` otherwise.

**Notes:**

- View function - always callable, never guarded by the pause flag.
- Clients should use this to disable mutating UI actions while keeping claim and refund actions enabled.

---

### `set_fee_bps`

Update protocol fee. Requires Admin role (0).

```rust
pub fn set_fee_bps(
    env: Env,
    admin: Address,
    fee_bps: u32
) -> Result<(), PredifiError>
```

**Parameters:**

| Parameter | Type      | Description                         |
| -------- | --------- | ---------------------------------- |
| `admin`   | `Address` | Admin address                       |
| `fee_bps` | `u32`     | New fee in basis points (max 10000) |

**Events:** `FeeUpdateEvent`

**Pause behavior:** Blocked when paused (returns `Paused`).

---

### `set_referral_cut_bps`

Set the referral cut in basis points (e.g. 5000 = 50% of the referrer's fee share). Requires Admin role (0). Must be ≤ 10_000.

```rust
pub fn set_referral_cut_bps(
    env: Env,
    admin: Address,
    referral_cut_bps: u32
) -> Result<(), PredifiError>
```

**Parameters:**

| Parameter          | Type      | Description                                                                 |
| ----------------- | --------- | --------------------------------------------------------------------------------------------------- |
| `admin`            | `Address` | Admin address                                                                        |
| `referral_cut_bps` | `u32`      | Share of protocol fee (attributable to referred user) paid to referrer (max 10000) |

**Pause behavior:** Blocked when paused (returns `Paused`).

**Referral view functions:**

- **get_referral_cut_bps(env: Env) -> u32** — Returns the current referral cut in basis points (default 5000).
- **`get_referred_volume(env: Env, referrer: Address, pool_id: u64) -> i128`** — Returns the total stake amount referred by `referrer` on the given pool (for analytics and payout context).

**Remainder view:**

- **`get_retained_remainder(env: Env, pool_id: u64) -> i128`** — Returns the truncation dust retained in escrow for a resolved pool (payout pool minus the sum of all distributed winnings). Always `>= 0` and deterministic for a given stake distribution.

---

### `set_treasury`

Update treasury address. Requires Admin role (0).

```rust
pub fn set_treasury(
    env: Env,
    admin: Address,
    treasury: Address
) -> Result<(), PredifiError>
```

**Parameters:**

| Parameter  | Type      | Description          |
| -------- | --------- | ------------------ |
| `admin`   | `Address` | Admin address        |
| `treasury` | `Address` | New treasury address |

**Events:** `TreasuryUpdateEvent`

**Pause behavior:** Blocked when paused (returns `Paused`).

---

## Data Structures

### `Pool`

```rust
pub struct Pool {
    pub end_time: u64,
    pub resolved: bool,
    pub outcome: u32,
    pub token: Address,
    pub total_stake: i128,
    pub description: String,
    pub metadata_url: String,
}
```

### `Prediction`

```rust
pub struct Prediction {
    pub amount: i128,
    pub outcome: u32,
}
```

### `UserPredictionDetail`

```rust
pub struct UserPredictionDetail {
    pub pool_id: u64,
    pub amount: i128,
    pub user_outcome: u32,
    pub pool_end_time: u64,
    pub pool_resolved: bool,
    pub pool_outcome: u32,
}
```

### `Config`

```rust
pub struct Config {
    pub fee_bps: u32,
    pub treasury: Address,
    pub access_control: Address,
}
```

### `ResolutionEvidence`

The stable evidence schema persisted for price-feed resolutions. This exact
record — XDR-encoded — is the preimage of the SHA-256 `evidence_hash` emitted in
`ResolutionEvidenceEvent`.

```rust
pub struct ResolutionEvidence {
    pub feed_pair: Symbol,   // Feed that decided the market
    pub price: i128,         // Observed feed value
    pub feed_timestamp: u64, // Feed-side observation timestamp
    pub comparison_op: u32,  // Rule applied: 0=Equal, 1=Greater, 2=Less
    pub target_price: i128,  // Target compared against
    pub tolerance_bps: u32,  // Tolerance around target, in basis points
    pub outcome: u32,        // Outcome index derived and recorded
}
```

The record is stored under `DataKey::ResolutionEvidence(pool_id)`, so the
`pool_id` binding comes from the storage key itself.

---

## Events

For the versioned, indexer-facing event schema reference — including topic names, field
types, compatibility rules, and the stable identifiers needed for idempotent indexing —
see [Event Schemas](./EVENT_SCHEMAS.md). The core events are listed below.

### `PoolCreatedEvent`

Emitted when a new pool is created.

```rust
pub struct PoolCreatedEvent {
    pub pool_id: u64,
    pub end_time: u64,
    pub token: Address,
    pub metadata_url: String,
}
```

### `PredictionPlacedEvent`

Emitted when a user places a prediction.

```rust
pub struct PredictionPlacedEvent {
    pub pool_id: u64,
    pub user: Address,
    pub amount: i128,
    pub outcome: u32,
}
```

### `PoolResolvedEvent`

Emitted when a pool is resolved and the outcome is determined. This occurs when:

-  An operator calls `resolve_pool()` with a valid outcome after the resolution delay
-  An oracle calls `oracle_resolve()` and the voting threshold is reached
-  `resolve_pool_from_price()` is called and the price condition is evaluated

The `operator` field indicates the address (operator or oracle) that triggered the resolution, or the contract itself if resolved automatically via price feed.

```rust
pub struct PoolResolvedEvent {
    pub pool_id: u64,     // Unique identifier for the resolved pool
    pub operator: Address, // Address that triggered the resolution
    pub outcome: u32,      // The determined winning outcome (0-based index)
}
```

**Example**: When outcome 1 is selected for pool 12345 by operator addr123:

```rust
PoolResolvedEvent {
    pool_id: 12345,
    operator: addr123,
    outcome: 1,
}
```
on, or the contract itself if resolved automatically via price feed.

```rust
pub struct PoolResolvedEvent {
    pub pool_id: u64,     // Unique identifier for the resolved pool
    pub operator: Address, // Address that triggered the resolution
    pub outcome: u32,      // The determined winning outcome (0-based index)
}
```

**Example**: When outcome 1 is selected for pool 12345 by operator addr123:

```rust
PoolResolvedEvent {
    pool_id: 12345,
    operator: addr123,
    outcome: 1,
}
```

### `PoolResolvedDiagEvent`

Emitted alongside `PoolResolvedEvent` with enriched numeric context for monitoring and payout calculations. This diagnostic event allows off-chain systems to:

- Calculate implied payouts
- Flag anomalies (e.g., `winning_stake == 0` indicates no winners)
- Track liquidity and stake distribution at resolution time

This event is **always** published immediately after `PoolResolvedEvent`.

```rust
pub struct PoolResolvedDiagEvent {
    pub pool_id: u64,           // Same pool as PoolResolvedEvent
    pub outcome: u32,            // Same outcome as PoolResolvedEvent
    pub total_stake: i128,       // Total stake across all outcomes
    pub winning_stake: i128,     // Stake on the winning outcome (0 ⟹ no winners)
    pub timestamp: u64,          // Ledger timestamp at resolution
}
```

**Example**: Diagnostic data for the same resolution:

```rust
PoolResolvedDiagEvent {
    pool_id: 12345,
    outcome: 1,
    total_stake: 5000,          // 5000 total tokens staked
    winning_stake: 2000,        // 2000 on outcome 1
    timestamp: 1234567890,
}
```

### `ResolutionEvidenceEvent`

Emitted atomically alongside `PoolResolvedEvent` when a pool is resolved through
`resolve_pool_from_price`. Carries the full evidence preimage fields plus
`evidence_hash`, the SHA-256 commitment over the stored `ResolutionEvidence`
record's XDR encoding.

```rust
pub struct ResolutionEvidenceEvent {
    pub pool_id: u64,           // Same pool as PoolResolvedEvent
    pub feed_pair: Symbol,      // Feed that decided the market
    pub price: i128,            // Observed feed value used in the comparison
    pub feed_timestamp: u64,    // Feed-side observation timestamp
    pub comparison_op: u32,     // 0=Equal, 1=Greater, 2=Less
    pub target_price: i128,     // Target compared against
    pub tolerance_bps: u32,     // Tolerance around target, in basis points
    pub outcome: u32,           // Same outcome as PoolResolvedEvent
    pub evidence_hash: BytesN<32>, // sha256(ResolutionEvidence XDR encoding)
}
```

### `WinningsClaimedEvent`

Emitted when a user claims winnings.

```rust
pub struct WinningsClaimedEvent {
    pub pool_id: u64,
    pub user: Address,
    pub amount: i128,
}
```

### `RewardClaimedEvent`

Emitted when a user claims a payout from the contract. This event is published for both winning claims and refund claims.

```rust
pub struct RewardClaimedEvent {
    pub pool_id: u64,
    pub user: Address,
    pub amount: i128,
    pub claim_type: String,
}
```

---

## Error Codes

See [Troubleshooting](./troubleshooting.md) for complete error reference.

| Code | Error             | Description                   |
| ---- | ----------------- | ----------------------------- |
| 10   | `Unauthorized`    | Caller lacks required role    |
| 22   | `PoolNotResolved` | Pool not yet resolved         |
| 60   | `AlreadyClaimed`  | User already claimed winnings |

---

## Next Steps

- Start with [Quickstart](./quickstart.md)
- Understand [Prediction Lifecycle](./prediction-lifecycle.md)
- Learn about [Oracles](./oracles.md)
- Review [Troubleshooting](./troubleshooting.md)
