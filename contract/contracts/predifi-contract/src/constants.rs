//! Contract constants and configuration values.
//!
//! This module contains all constant values used throughout the PrediFi contract,
//! including storage parameters, pool limits, and default values.
//!
//! It also documents the deterministic payout remainder allocation invariant that
//! governs how integer-division dust is handled when distributing fees and winnings.

// ═══════════════════════════════════════════════════════════════════════════
// STORAGE & LEDGER CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Number of ledgers in a day (assuming ~5 second ledger close time).
///
/// **Units:** Ledgers (dimensionless)
///
/// **Rationale:** Stellar ledgers close approximately every 5 seconds. This constant
/// represents the number of ledgers that elapse in a 24-hour period (86400 seconds / 5).
///
/// **Impact of changes:**
/// - Increasing this value would cause storage TTL calculations to be too conservative,
///   potentially extending storage more frequently than necessary and increasing costs.
/// - Decreasing this value would cause storage to expire sooner than intended,
///   risking data loss if TTL extensions are not triggered in time.
/// - This value should only be changed if Stellar's ledger close time changes significantly.
///
/// **Used for:** Calculating storage TTL extensions (`BUMP_THRESHOLD`, `BUMP_AMOUNT`).
pub const DAY_IN_LEDGERS: u32 = 17280;

/// Threshold for extending storage TTL (14 days in ledgers).
///
/// **Units:** Ledgers (dimensionless)
/// **Value:** 241,920 ledgers (14 days)
///
/// **Rationale:** When persistent storage entries approach expiration, they must be
/// extended to prevent data loss. This threshold provides a 14-day buffer, ensuring
/// that even if the contract is not called frequently, storage entries remain valid.
/// The 14-day window balances storage costs with safety margins.
///
/// **Impact of changes:**
/// - Increasing this value (e.g., to 30 days) reduces the frequency of TTL extensions,
///   lowering storage costs but increasing the risk of data loss if the contract is inactive.
/// - Decreasing this value (e.g., to 7 days) increases extension frequency, raising costs
///   but providing a larger safety margin against data loss.
/// - Must be less than `BUMP_AMOUNT` to ensure storage is extended before it expires.
///
/// **Used for:** Determining when to call `extend_persistent` and `bump_ttl`.
pub const BUMP_THRESHOLD: u32 = 14 * DAY_IN_LEDGERS;

/// Amount to extend storage TTL by (30 days in ledgers).
///
/// **Units:** Ledgers (dimensionless)
/// **Value:** 518,400 ledgers (30 days)
///
/// **Rationale:** When storage TTL is extended, it's extended by a significant duration
/// to minimize the frequency of future extensions. 30 days provides a reasonable balance
/// between storage efficiency and cost. This is more than double the `BUMP_THRESHOLD`
/// to ensure that after extension, storage remains valid for a substantial period.
///
/// **Impact of changes:**
/// - Increasing this value (e.g., to 60 days) reduces extension frequency but increases
///   the maximum storage cost per entry, as Soroban charges for the maximum TTL.
/// - Decreasing this value (e.g., to 15 days) increases extension frequency, potentially
///   raising transaction costs due to more frequent bump operations.
/// - Must be greater than `BUMP_THRESHOLD` to ensure storage doesn't expire between checks.
///
/// **Used for:** Calculating the new TTL when calling `extend_persistent` and `bump_ttl`.
pub const BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS;

// ═══════════════════════════════════════════════════════════════════════════
// STORAGE RETENTION STRATEGY
// ═══════════════════════════════════════════════════════════════════════════
//
// Persistent vs. temporary storage classification
// ----------------------------------------------
// Soroban offers two storage durability classes:
//
// * **Persistent** entries survive as long as their TTL is periodically extended.
//   They are the canonical source of truth for claims, auditability, and client
//   queries, so they MUST be extended while a pool is still actionable.
// * **Temporary** entries are cheap and expire automatically without any bump.
//   They are only suitable for data that can be safely regenerated or that is
//   only meaningful for a short window (rate-limit markers, caches, etc.).
//
// The PrediFi contract classifies its state as follows:
//
// Persistent (must be retained for the full claim window):
//   - `Pool` records (including `outcome`, `end_time`, `resolved_at`).
//   - Per-user `Position` / `UserStake` entries (required to claim).
//   - `OutStakes` batch entries for pools with >= 16 outcomes.
//   - `Config`, `Admin`, `Operators`, `ContractVersion` (instance storage).
//
// Temporary (may expire without affecting claims):
//   - `LastPredictionTime(user)` cooldown markers.
//   - Any future caches, price snapshots, or derived indexes.
//
// Archival / TTL behavior
// -----------------------
// Persistent entries are bumped using `BUMP_THRESHOLD` / `BUMP_AMOUNT` above.
// Once a pool is fully resolved AND the claim window has elapsed, its entries
// become eligible for archival: the contract stops extending them and lets the
// TTL lapse naturally. Archived entries remain recoverable via Soroban's
// restore mechanism, preserving auditability without paying ongoing rent.
//
// Minimum retention period for unresolved claims
// ----------------------------------------------
// A resolved pool MUST keep its persistent entries alive for at least
// `CLAIM_WINDOW_LEDGERS` after `resolved_at`. This guarantees that every user
// who is entitled to a payout can still call `claim` during the promised
// window, even if the contract is otherwise idle. The window is deliberately
// longer than `BUMP_AMOUNT` so that a single bump at resolution time is
// sufficient to cover the entire claim period.

/// Minimum retention period for claimable pool data after resolution.
///
/// **Units:** Ledgers (dimensionless)
/// **Value:** 1,555,200 ledgers (90 days)
///
/// **Rationale:** After a pool is resolved, users must be given a guaranteed
/// window during which their positions remain claimable. 90 days is long enough
/// to accommodate inactive users, wallet recovery, and dispute resolution, while
/// still allowing the contract to eventually stop paying storage rent on pools
/// that everyone has already claimed. This value MUST be greater than
/// `BUMP_AMOUNT` so that a single TTL extension performed at resolution time
/// covers the entire claim window without requiring further bumps.
///
/// **Impact of changes:**
/// - Increasing this value extends the guaranteed claim window but increases
///   the minimum storage cost per resolved pool.
/// - Decreasing this value shortens the claim window and risks users losing
///   access to their funds if they claim late.
/// - Must remain strictly greater than `BUMP_AMOUNT`; otherwise the TTL set at
///   resolution could expire before the claim window ends.
///
/// **Used for:** Determining when resolved pool entries become eligible for
/// archival, and validating that the claim window is covered by a single bump.
pub const CLAIM_WINDOW_LEDGERS: u32 = 90 * DAY_IN_LEDGERS;

/// Retention period for temporary entries (e.g. cooldown markers).
///
/// **Units:** Ledgers (dimensionless)
/// **Value:** 17,280 ledgers (1 day)
///
/// **Rationale:** Temporary entries such as `LastPredictionTime(user)` only need
/// to outlive the longest possible cooldown. One day is comfortably longer than
/// any sane `prediction_cooldown_seconds` value while keeping temporary storage
/// cheap and self-cleaning. Entries that expire can be safely recreated on the
/// next interaction, so no data is lost.
///
/// **Impact of changes:**
/// - Increasing this value keeps temporary entries around longer, slightly
///   increasing storage cost but reducing the chance of a cooldown being reset
///   by expiry.
/// - Decreasing this value below the configured cooldown would allow users to
///   bypass rate limiting by waiting for the marker to expire.
/// - Must be >= the maximum supported `prediction_cooldown_seconds` expressed
///   in ledgers.
///
/// **Used for:** Setting the TTL on temporary storage entries.
pub const TEMPORARY_ENTRY_TTL_LEDGERS: u32 = DAY_IN_LEDGERS;

// ═══════════════════════════════════════════════════════════════════════════
// POOL CONFIGURATION CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Default minimum pool duration in seconds (1 hour).
///
/// **Units:** Seconds
/// **Value:** 3,600 seconds (1 hour)
///
/// **Rationale:** Pools need sufficient time to attract participants and generate meaningful
/// betting activity. A 1-hour minimum prevents creation of ultra-short-lived pools that
/// would provide poor user experience and negligible engagement. This also gives oracle
/// systems time to update before resolution.
///
/// **Impact of changes:**
/// - Increasing this value (e.g., to 24 hours) ensures longer engagement windows but
///   may limit the types of markets that can be created (e.g., rapid-response events).
/// - Decreasing this value (e.g., to 10 minutes) allows more granular, short-term markets
///   but may increase spam and reduce participation per pool.
/// - The global default is used only until an admin updates it via
///   `Config::set_min_pool_duration`; the stored `Config::min_pool_duration`
///   then takes precedence.
///
/// **Used for:** Validating `end_time - start_time` during pool creation.
pub const DEFAULT_MIN_POOL_DURATION: u64 = 3600;

/// Cancellation delay in seconds for overdue pools (7 days).
///
/// **Units:** Seconds
/// **Value:** 604,800 seconds (7 days)
///
/// **Rationale:** After a pool's `end_time` passes, it may remain unresolved due to oracle
/// delays, disputes, or operator inaction. This delay provides a grace period for normal
/// resolution before allowing anyone to cancel the pool. 7 days balances giving operators
/// sufficient time with preventing indefinite limbo states.
///
/// **Impact of changes:**
/// - Increasing this value (e.g., to 30 days) gives operators more time to resolve pools
///   but leaves user funds locked longer if resolution is delayed.
/// - Decreasing this value (e.g., to 3 days) allows faster cancellation of stuck pools
///   but may trigger premature cancellations before operators can act.
/// - Affects the `cancel_pool` function's time-based authorization logic.
///
/// **Used for:** Determining when any user (not just operators) can cancel an overdue pool.
pub const CANCELATION_DELAY: u64 = 604_800;

/// Default global minimum stake amount (1 unit in token base units).
///
/// **Units:** Token base units, which are the smallest divisible units accepted
/// by the Stellar token contract. For native XLM this is stroops, where
/// 1 XLM = 10,000,000 stroops.
/// **Value:** 1 base unit/stroop-equivalent
///
/// **Rationale:** To prevent spam and dust transactions, there must be a minimum stake
/// requirement. Setting this to 1 (the smallest possible unit) allows maximum flexibility
/// while still rejecting zero or negative amounts. The actual effective minimum is
/// typically set higher via per-pool `min_stake` or admin configuration.
///
/// **Impact of changes:**
/// - Increasing this value (e.g., to 1000) would reject very small stakes globally,
///   preventing participation from users with small balances but reducing spam.
/// - Decreasing this value is not possible (cannot go below 1).
/// - This is a hard floor; individual pools can have higher minimums via `pool.min_stake`.
/// - Can be overridden via `Config::min_stake` by admin.
///
/// **Used for:** Validating that `amount > 0` in `place_prediction` (via `InsufficientStake` error).
pub const DEFAULT_GLOBAL_MIN_STAKE: i128 = 1;

/// Default cooldown in seconds between consecutive `place_prediction` calls by the same user.
///
/// **Units:** Seconds
/// **Value:** 0 (disabled by default)
///
/// **Rationale:** Rate limiting prevents spam and front-running attacks by limiting how
/// quickly a single user can place predictions. Defaulting to 0 (disabled) keeps the
/// protocol permissionless and lets existing deployments keep their current behavior,
/// while operators can opt in via `Config::set_prediction_cooldown_seconds`. Enabling a
/// cooldown of 15–60 seconds is recommended on mainnet to slow down rapid-fire prediction
/// strategies that could be used to exploit temporary price discrepancies or to front-run
/// other users (see `docs/security-front-running-analysis.md`).
///
/// **Impact of changes:**
/// - Increasing this value (e.g., to 30) enforces a stricter cooldown by default, preventing
///   rapid consecutive predictions but potentially frustrating legitimate users.
/// - Decreasing this value is not possible (cannot go below 0).
/// - Setting the effective config value to 0 disables the cooldown mechanism entirely.
/// - When enabled, the cooldown is enforced via `LastPredictionTime(user)` storage.
/// - Can be overridden via `Config::prediction_cooldown_seconds` by admin.
///
/// **Used for:** Initializing `Config::prediction_cooldown_seconds` during contract initialization.
pub const DEFAULT_PREDICTION_COOLDOWN_SECONDS: u64 = 0;

/// Contract-level maximum number of outcomes allowed in a single pool.
///
/// **Units:** Count (dimensionless)
/// **Value:** 100 outcomes
///
/// **Benchmark evidence:** `create_pool` with 100 outcomes consumes ~685k CPU
/// instructions (well under Soroban's 100M instruction transaction ceiling) and
/// the outcome-stake vector adds only ~1.6 KB to the pool entry. Both metrics
/// are reported by `benchmark_tests::test_resource_ceilings_at_limits`.
///
/// **Rationale:** Each outcome adds storage and resolution bookkeeping. A cap
/// of 100 supports large tournaments (e.g. 64-team brackets) while keeping
/// creation and resolution far below execution limits.
///
/// **Impact of changes:**
/// - Increasing this value allows larger markets but grows the `OutStakes`
///   vector and the per-transaction cost of creation.
/// - Decreasing this value reduces complexity but prevents some market types.
///
/// **Used for:** Validating `options_count` during pool creation
/// (`MaxOutcomesExceeded`).
pub const MAX_OUTCOMES_PER_POOL: u32 = 100;

/// Backwards-compatible alias for the outcome ceiling.
///
/// Kept so existing callers/tests that reference `MAX_OPTIONS_COUNT` continue
/// to compile and observe the same limit.
pub const MAX_OPTIONS_COUNT: u32 = MAX_OUTCOMES_PER_POOL;

/// Contract-level maximum number of distinct predictors in a single pool.
///
/// **Units:** Count (dimensionless)
/// **Value:** 1,000 predictions
///
/// **Benchmark evidence:** Each new predictor adds 320 XDR bytes of
/// persistent state, so a pool at the cap holds ~321 KB of prediction data
/// including the pool entry. The 1,000th prediction measures ~29.9M CPU
/// instructions and ~15.4 MB of memory (the first prediction is ~0.7M CPU),
/// leaving roughly 3.3x CPU headroom under Soroban's 100M instruction limit
/// and 2.6x memory headroom under the 40 MB test budget. See
/// `benchmark_tests::test_resource_ceilings_at_limits`.
///
/// **Rationale:** This bounds per-pool storage growth while still supporting
/// high-volume markets. Repeated stake increases from an existing predictor
/// do not consume another slot.
///
/// **Used for:** Validating `pool.participants_count` before a new prediction
/// is written (`MaxPredictionsPerPoolExceeded`).
pub const MAX_PREDICTIONS_PER_POOL: u32 = 1_000;

/// Contract-level maximum aggregate stake for a single pool, in token base units.
///
/// **Units:** Token base units (stroops for native XLM)
/// **Value:** 1,000,000,000,000,000,000 base units
/// **Equivalent:** 100,000,000,000 tokens at 7 decimal places
///
/// **Benchmark evidence / safety derivation:** Aggregate stake arithmetic is
/// O(1), so the binding constraint is overflow-safe proportional payout math.
/// The hot payout path multiplies `user_stake * payout_pool`; with both bounded
/// by this cap the worst-case product is `1e18 * 1e18 = 1e36`, which is below
/// `i128::MAX` (~1.70e38) with ~170x headroom. All additions use
/// `checked_add`, so a value that would overflow is rejected as
/// `MaxAggregateStakeExceeded` before any write. The boundary tests cover
/// `MAX_AGGREGATE_STAKE_PER_POOL - 1`, the cap itself, the cap + 1, and an
/// overflowing addition.
///
/// **Used for:** Enforcing the protocol-wide aggregate stake ceiling in
/// `place_prediction`, `create_pool`, and `increase_max_total_stake`.
pub const MAX_AGGREGATE_STAKE_PER_POOL: i128 = 1_000_000_000_000_000_000;

/// Maximum initial liquidity that can be provided (100M tokens at 7 decimals).
///
/// **Units:** Token base units. For native XLM, these are stroops; for issued
/// Stellar assets, this is the asset's smallest contract unit.
/// **Value:** 100,000,000,000,000 base units
/// **Equivalent:** 100,000,000 tokens at 7 decimal places (e.g., 100M USDC)
///
/// **Rationale:** Initial liquidity represents "house money" provided by the pool creator
/// to bootstrap the market. This cap prevents excessive risk exposure for creators and
/// potential protocol-level liquidity imbalances. At 7 decimals (common for USDC on Stellar),
/// this equals 100 million tokens, a substantial but not unlimited amount.
///
/// **Impact of changes:**
/// - Increasing this value would allow larger initial liquidity, enabling bigger markets
///   but increasing creator risk and potential protocol exposure.
/// - Decreasing this value would limit market size, potentially preventing legitimate large-scale markets.
/// - The actual value in token terms depends on the token's decimal places:
///   - 7 decimals (USDC): 100,000,000 tokens
///   - 6 decimals (USDT): 1,000,000,000 tokens
///   - 7 decimals (XLM): 100,000,000 XLM
///
/// **Used for:** Validating `initial_liquidity` during pool creation.
pub const MAX_INITIAL_LIQUIDITY: i128 = 100_000_000_000_000;

/// Sentinel value used to indicate that a pool outcome has not been resolved yet.
///
/// **Units:** Outcome index (dimensionless)
/// **Value:** 4,294,967,295 (u32::MAX)
///
/// **Rationale:** When a pool is created, its outcome is unknown. We need a sentinel value
/// that cannot be confused with a valid outcome index (0, 1, 2, ...). Using `u32::MAX`
/// ensures this since valid outcomes are bounded by `MAX_OPTIONS_COUNT` (100). This allows
/// us to distinguish between "outcome 0 won" (a valid result) and "pool not resolved".
///
/// **Impact of changes:**
/// - This value should never be changed as it's used throughout the codebase as a
///   sentinel for unresolved pools.
/// - Changing this would require a storage migration to update all existing pools.
/// - Must be a value that cannot occur as a valid outcome index (given `MAX_OPTIONS_COUNT`).
///
/// **Used for:** Initializing `pool.outcome` and checking if a pool is resolved.
pub const UNRESOLVED_OUTCOME: u32 = u32::MAX;

/// Maximum allowed pool duration in seconds (365 days).
///
/// **Units:** Seconds
/// **Value:** 31,536,000 seconds (365 days)
///
/// **Rationale:** Long-running pools tie up user funds for extended periods and may become
/// irrelevant due to changing market conditions. A 1-year maximum ensures pools remain
/// timely and relevant while still allowing long-term predictions (e.g., annual events).
/// This also limits the duration over which oracle data must remain valid.
///
/// **Impact of changes:**
/// - Increasing this value (e.g., to 2 years) would allow longer-term markets but increase
///   the risk of funds being locked for extended periods and oracle data becoming stale.
/// - Decreasing this value (e.g., to 180 days) would limit market types to shorter-term events.
/// - Affects the validation of `end_time - start_time` during pool creation.
///
/// **Used for:** Validating pool duration during creation (via `InvalidTimestamp` error).
pub const MAX_POOL_DURATION: u64 = 31_536_000;

/// Maximum length (in bytes/chars) of a single outcome description.
///
/// **Units:** Bytes/characters
/// **Value:** 128 bytes/characters
///
/// **Rationale:** Outcome descriptions (e.g., "Yes", "No", "Team A wins") are stored in
/// persistent storage. Without bounds, a malicious or careless creator could create
/// pools with extremely long descriptions, causing storage bloat and excessive gas costs.
/// 128 characters is sufficient for most outcome labels while preventing abuse.
///
/// **Impact of changes:**
/// - Increasing this value (e.g., to 256) would allow more descriptive labels but increase
///   storage costs per outcome and potential for abuse.
/// - Decreasing this value (e.g., to 64) would reduce storage costs but may limit expressiveness
///   for certain market types (e.g., long team names).
/// - Affects storage size: `pool.outcome_descriptions` is a Vec<String>, so total storage
///   scales with `options_count * MAX_OUTCOME_DESCRIPTION_LEN`.
///
/// **Used for:** Validating outcome description lengths during pool creation (issue #1122).
pub const MAX_OUTCOME_DESCRIPTION_LEN: u32 = 128;

/// Minimum length (in bytes/chars) of a single outcome description.
///
/// **Units:** Bytes/characters
/// **Value:** 1 byte/character
///
/// **Rationale:** Empty or whitespace-only outcome descriptions provide no useful information
/// to users and may indicate errors or malicious intent. Requiring at least 1 character
/// ensures all outcomes have meaningful labels.
///
/// **Impact of changes:**
/// - Increasing this value (e.g., to 3) would prevent single-character labels like "A" or "B",
///   which may be too restrictive for some use cases.
/// - Decreasing this value is not possible (cannot go below 1).
/// - Works in conjunction with `MAX_OUTCOME_DESCRIPTION_LEN` to bound description length.
///
/// **Used for:** Validating outcome description lengths during pool creation (issue #1122).
pub const MIN_OUTCOME_DESCRIPTION_LEN: u32 = 1;

/// Initial-liquidity safety margin in basis points relative to `max_total_stake`.
///
/// **Units:** Basis points (bps, where 100 bps = 1%)
/// **Value:** 100 bps (1%)
///
/// **Rationale:** When a pool creator sets a `max_total_stake` cap and provides initial
/// liquidity (house money), there's a risk that early large bets could drain the pool
/// before the creator's liquidity provides meaningful coverage. This margin requires that
/// initial liquidity be at least 1% of `max_total_stake`, ensuring the creator has skin
/// in the game and the pool has sufficient buffer against early manipulation.
///
/// **Impact of changes:**
/// - Increasing this value (e.g., to 500 bps = 5%) would require more initial liquidity,
///   reducing pool creation flexibility but increasing safety against early draining.
/// - Decreasing this value (e.g., to 50 bps = 0.5%) would allow pools with less initial
///   liquidity but increase the risk of early pool draining attacks.
/// - Only applies when `max_total_stake > 0`; pools without a cap are not subject to this check.
///
/// **Used for:** Validating `initial_liquidity` relative to `max_total_stake` during pool creation (issue #1131).
pub const INITIAL_LIQUIDITY_SAFETY_MARGIN_BPS: u32 = 100;

/// Multisig threshold for the emergency-cancel flow.
///
/// **Units:** Count (dimensionless)
/// **Value:** 2 approvals
///
/// **Rationale:** Emergency cancellation is a powerful action that should require consensus
/// to prevent abuse. Requiring at least 2 distinct operator/admin approvals ensures that
/// no single actor can unilaterally cancel a pool, providing a check against malicious
/// or mistaken emergency cancellations.
///
/// **Impact of changes:**
/// - Increasing this value (e.g., to 3) would require more consensus, making emergency
///   cancellation harder but safer against collusion.
/// - Decreasing this value to 1 would allow single-actor emergency cancellation, increasing
///   the risk of abuse.
/// - Must be <= the number of active operators/admins, or pools can never be emergency-cancelled.
///
/// **Used for:** Validating that sufficient approvals have been collected before executing
/// `emergency_cancel_pool` (issue #1119).
pub const EMERGENCY_CANCEL_MULTISIG_THRESHOLD: u32 = 2;

// ═══════════════════════════════════════════════════════════════════════════
// BATCH CLAIM LIMITS
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum number of claims processed in a single `batch_claim` invocation.
///
/// **Units:** Count (dimensionless)
/// **Value:** 25 claims
///
/// **Benchmark evidence:** Each claim performs a persistent read of the
/// prediction entry, a token transfer, and a persistent write/removal. At 25
/// claims the measured cost stays within a generous fraction of Soroban's
/// 100M CPU instruction ceiling and 40 MB memory budget, leaving headroom for
/// the worst case where every claim touches a distinct prediction and triggers
/// a separate token transfer. See
/// `benchmark_tests::test_batch_claim_resource_ceilings`.
///
/// **Rationale:** Bounding the batch size by measured resource cost prevents
/// oversized batches from exhausting the transaction budget mid-execution.
/// Because the whole invocation is atomic, a batch that would exceed the
/// budget must be rejected up front rather than failing after partial
/// processing.
///
/// **Impact of changes:**
/// - Increasing this value raises per-transaction CPU/memory cost and the
///   worst-case number of token transfers; re-run the batch-claim benchmark
///   before changing.
/// - Decreasing this value reduces per-call cost but forces clients to submit
///   more transactions.
///
/// **Used for:** Rejecting oversized batches (`BatchTooLarge`) before any
/// claim is processed in `batch_claim`.
pub const MAX_BATCH_CLAIM_SIZE: u32 = 25;

/// Recommended client chunk size for `batch_claim`.
///
/// **Units:** Count (dimensionless)
/// **Value:** 10 claims
///
/// **Rationale:** Clients should submit batches well below the hard ceiling so
/// that a single oversized or failing claim does not force an entire large
/// batch to be retried. Chunking at 10 keeps each transaction comfortably
/// within budget while amortizing per-transaction overhead.
///
/// **Impact of changes:**
/// - Must remain `<= MAX_BATCH_CLAIM_SIZE`; larger values would be rejected.
/// - This is a client-side recommendation only and is not enforced on-chain.
///
/// **Used for:** Documenting recommended client chunk sizes for batch claims.
pub const RECOMMENDED_BATCH_CLAIM_CHUNK_SIZE: u32 = 10;

// ═══════════════════════════════════════════════════════════════════════════
// MONITORING & ALERT THRESHOLDS
// ═══════════════════════════════════════════════════════════════════════════

/// Stake amount above which a `HighValuePredictionEvent` is emitted.
///
/// **Units:** Token base units. For XLM-denominated pools, this value is in
/// stroops; for other Stellar tokens it uses that token contract's base units.
/// **Value:** 1,000,000,000 base units
/// **Equivalent:** 100 tokens at 7 decimal places (e.g., 100 USDC)
///
/// **Rationale:** Large stakes represent significant user exposure and potential market impact.
/// Emitting a special event when stakes exceed this threshold allows off-chain monitoring
/// systems to apply extra scrutiny, detect potential manipulation, or trigger alerts.
/// At 7 decimals (common for USDC), this equals 100 USDC, a reasonable threshold for "large" bets.
///
/// **Impact of changes:**
/// - Increasing this value (e.g., to 10,000,000,000 = 1000 USDC) would reduce the frequency
///   of high-value events, potentially missing significant bets.
/// - Decreasing this value (e.g., to 100,000,000 = 10 USDC) would increase event frequency,
///   potentially creating noise in monitoring systems.
/// - The actual token value depends on the token's decimal places.
///
/// **Used for:** Triggering `HighValuePredictionEvent` in `place_prediction`.
pub const HIGH_VALUE_THRESHOLD: i128 = 1_000_000_000;

/// Maximum tolerance in basis points (1 bp = 0.01%).
///
/// **Units:** Basis points (bps)
/// **Value:** 10,000 bps (100%)
///
/// **Rationale:** Price conditions (e.g., "price within 5% of target") use basis points
/// to express tolerance ranges. This constant represents 100% tolerance, serving as the
/// denominator for tolerance calculations. Using basis points allows precise percentage-based
/// conditions without floating-point arithmetic.
///
/// **Impact of changes:**
/// - This value should never be changed as it represents the mathematical definition of
///   100% in basis points.
/// - Changing this would break all existing price condition calculations.
/// - Used as a scaling factor: `tolerance_amount = (base_amount * tolerance_bps) / MAX_TOLERANCE`.
///
/// **Used for:** Calculating tolerance ranges in price-based oracle conditions.
pub const MAX_TOLERANCE: u32 = 10_000;

/// Default minimum accepted oracle freshness policy in seconds (5 seconds).
pub const DEFAULT_MIN_ORACLE_FRESHNESS: u64 = 5;

/// Default maximum accepted oracle freshness policy in seconds (86,400 seconds = 24 hours).
pub const DEFAULT_MAX_ORACLE_FRESHNESS: u64 = 86_400;

/// Maximum number of primitive checks allowed while matching a price condition.
///
/// **Units:** Count (dimensionless)
/// **Value:** 4 checks
///
/// **Rationale:** Price condition matching (e.g., "price > X AND price < Y") involves
/// evaluating primitive checks against oracle data. To ensure resolution remains O(1)
/// and gas-predictable, we bound the number of checks. This prevents future condition
/// logic from becoming unbounded and causing resolution failures due to gas limits.
///
/// **Impact of changes:**
/// - Increasing this value (e.g., to 8) would allow more complex conditions but increase
///   gas costs and potential for resolution failures.
/// - Decreasing this value (e.g., to 2) would limit condition expressiveness, potentially
///   preventing useful market types.
/// - This is a hard limit on condition complexity; exceeding it causes resolution to fail.
///
/// **Used for:** Validating price condition complexity during oracle-based resolution.
pub const MAX_PRICE_CONDITION_MATCH_STEPS: u32 = 4;

// ═══════════════════════════════════════════════════════════════════════════
// VERSION CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Current contract version. Bump on each release to support safe migrations.
///
/// **Units:** Version number (dimensionless)
/// **Value:** 1
///
/// **Rationale:** Contract upgrades require version tracking to enable safe state migrations.
/// This version is stored in instance storage and checked during upgrades to ensure
/// compatibility. Each release should increment this value to trigger migration logic
/// if needed.
///
/// **Impact of changes:**
/// - Must be incremented on each contract release that requires state migration.
/// - Should not be decremented (versions are monotonic).
/// - Used by upgrade logic to determine which migration steps to execute.
/// - Changing this without proper migration logic can cause upgrade failures or data corruption.
///
/// **Used for:** Version tracking in instance storage and upgrade migration logic.
pub const CONTRACT_VERSION: u32 = 1;

/// Version of the public **event schema** emitted by the contract.
///
/// **Units:** Version number (dimensionless)
/// **Value:** 1
///
/// **Rationale:** Off-chain indexers, subgraphs, and monitoring services depend on the
/// topic names and payload fields of emitted events. This constant identifies the schema
/// as a whole so consumers can detect when the shape of event data changes. A conforming
/// schema (`1`) is documented in `docs/EVENT_SCHEMAS.md` and pinned by the golden tests in
/// `src/event_schema_tests.rs`.
///
/// **Impact of changes:**
/// - Must be incremented for any **breaking** change (renamed/removed topic, renamed/
///   removed/retyped payload field, or changed event semantics).
/// - Must NOT be incremented for additive changes (new topic, new optional field).
/// - Should not be decremented (versions are monotonic).
/// - Changing this without updating the reference document and golden tests will fail CI.
///
/// **Used for:** Signalling the event schema version to external indexers.
pub const EVENT_SCHEMA_VERSION: u32 = 1;

/// Minimum timelock delay in seconds for protocol fee changes.
///
/// **Units:** Seconds
/// **Value:** 86,400 seconds (1 day)
///
/// **Rationale:** Protocol fee changes are sensitive parameters that affect user economics.
/// A timelock delay ensures users and integrators have time to observe pending fee changes
/// and react (e.g., withdraw positions, adjust strategies) before the new fee takes effect.
/// This prevents surprise fee hikes and provides governance transparency.
///
/// **Impact of changes:**
/// - Increasing this value (e.g., to 604,800 = 7 days) would provide more reaction time
///   but slow down legitimate fee adjustments.
/// - Decreasing this value (e.g., to 43,200 = 12 hours) would allow faster fee changes
///   but reduce the window for users to react.
/// - Affects the `set_fee_bps` → `apply_fee_bps` workflow via `PendingFeeChange.effective_at`.
/// - Too short a timelock could enable governance attacks or user harm.
///
/// **Used for:** Calculating `PendingFeeChange.effective_at` when queuing fee changes.
pub const FEE_CHANGE_TIMELOCK_SECONDS: u64 = 86_400;

// ═══════════════════════════════════════════════════════════════════════════
// EMERGENCY PAUSE OPERATION MATRIX
// ═══════════════════════════════════════════════════════════════════════════

/// Operation bit for pausing pool creation (`create_pool`).
///
/// **Units:** Bitmask flag (dimensionless)
/// **Value:** 1 << 0
///
/// **Rationale:** Pausing pool creation halts new market deployment without affecting
/// existing pools, allowing operators to stop new exposure while preserving all
/// user-facing exit paths.
///
/// **Used for:** Encoding the pause matrix in `PauseState`/authorization checks.
pub const PAUSE_OP_CREATE_POOL: u32 = 1 << 0;

/// Operation bit for pausing predictions (`place_prediction`).
///
/// **Units:** Bitmask flag (dimensionless)
/// **Value:** 1 << 1
///
/// **Rationale:** Pausing predictions stops new stake inflow while leaving claims,
/// refunds, and cancellations available so users can always exit.
///
/// **Used for:** Encoding the pause matrix in `PauseState`/authorization checks.
pub const PAUSE_OP_PLACE_PREDICTION: u32 = 1 << 1;

/// Operation bit for pausing pool resolution (`resolve_pool`).
///
/// **Units:** Bitmask flag (dimensionless)
/// **Value:** 1 << 2
///
/// **Rationale:** Pausing resolution freezes outcome settlement without blocking
/// claims on already-resolved pools or refunds on cancelled pools.
///
/// **Used for:** Encoding the pause matrix in `PauseState`/authorization checks.
pub const PAUSE_OP_RESOLVE_POOL: u32 = 1 << 2;

/// Bitmask of all pausable operations.
///
/// **Units:** Bitmask flag (dimensionless)
/// **Value:** `PAUSE_OP_CREATE_POOL | PAUSE_OP_PLACE_PREDICTION | PAUSE_OP_RESOLVE_POOL`
///
/// **Rationale:** Safe exits (claims, refunds, cancellations) are intentionally excluded
/// from this mask so they can never be paused, guaranteeing users can always withdraw.
///
/// **Used for:** Validating pause requests and exposing pause state to clients.
pub const PAUSE_OP_ALL: u32 =
    PAUSE_OP_CREATE_POOL | PAUSE_OP_PLACE_PREDICTION | PAUSE_OP_RESOLVE_POOL;

// NO-CONTEST & REFUND SETTLEMENT CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Sentinel outcome value indicating a pool settled under the no-contest refund policy.
///
/// **Units:** Outcome index (dimensionless)
/// **Value:** 4,294,967,294 (u32::MAX - 1)
///
/// **Rationale:** When a pool cannot be resolved (invalid oracle data, cancelled market,
/// or explicit no-contest), it settles under a single explicit refund policy rather than
/// remaining in an ambiguous lifecycle state. This sentinel is distinct from
/// `UNRESOLVED_OUTCOME` (u32::MAX) so that "not yet resolved" and "settled as no-contest"
/// are never conflated, and cannot collide with any valid outcome index (bounded by
/// `MAX_OPTIONS_COUNT`).
///
/// **Impact of changes:**
/// - Must remain distinct from `UNRESOLVED_OUTCOME` and from any valid outcome index.
/// - Changing this would require a storage migration for pools already settled as no-contest.
///
/// **Used for:** Marking `pool.outcome` when a pool settles under the refund policy.
pub const NO_CONTEST_OUTCOME: u32 = u32::MAX - 1;

/// Fee in basis points applied to no-contest refunds (0 bps = no fee).
///
/// **Units:** Basis points (bps, where 100 bps = 1%)
/// **Value:** 0 bps
///
/// **Rationale:** No-contest settlement is a safety mechanism, not a revenue event. When a
/// market is cancelled or cannot be resolved, participants should recover their stake in
/// full. Charging a protocol fee or paying referrals on a refund would penalize users for
/// an outcome they did not cause. Keeping this at 0 makes the refund policy explicit and
/// auditable: every participant recovers exactly their recorded stake.
///
/// **Impact of changes:**
/// - Increasing this value would deduct a fee from refunds, breaking the invariant that
///   total refundable value equals total escrowed stake.
/// - Must remain 0 unless the refund policy is deliberately changed and re-audited.
///
/// **Used for:** Computing the refund amount in the no-contest settlement path.
pub const NO_CONTEST_FEE_BPS: u32 = 0;

/// Whether referral rewards are paid on no-contest refunds.
///
/// **Units:** Boolean flag
/// **Value:** false
///
/// **Rationale:** Referral rewards are funded from protocol fees on resolved markets. Since
/// no-contest settlement charges no fee (`NO_CONTEST_FEE_BPS == 0`), there is no fee pool
/// from which to pay referrals. Setting this to false makes the rule explicit and prevents
/// referral payouts from silently draining the escrow that must be returned to participants.
///
/// **Impact of changes:**
/// - Setting this to true without a corresponding fee would break accounting conservation.
/// - Must remain false while `NO_CONTEST_FEE_BPS` is 0.
///
/// **Used for:** Gating referral payout logic in the no-contest settlement path.
pub const NO_CONTEST_PAYS_REFERRALS: bool = false;

/// Maximum protocol fee in basis points (10% = 1000 bps).
///
/// **Units:** Basis points (bps, where 100 bps = 1%)
/// **Value:** 1,000 bps (10%)
///
/// **Rationale:** Caps the protocol fee so governance cannot unexpectedly change
/// participant economics beyond a hard bound. Combined with the timelock, this
/// ensures fee changes are both observable and bounded.
///
/// **Impact of changes:**
/// - Increasing this value would allow higher fees, increasing protocol revenue
///   but potentially harming participant economics.
/// - Decreasing this value would further constrain governance but may limit
///   protocol sustainability.
///
/// **Used for:** Enforcing the fee cap in `set_fee_bps` / `apply_fee_bps`.
pub const MAX_FEE_BPS: u32 = 1_000;

/// Minimum timelock delay in seconds for treasury fee changes.
///
/// **Units:** Seconds
/// **Value:** 3,600 seconds (1 hour)
///
/// **Rationale:** Provides a lower bound on the delay between proposing and
/// activating a treasury fee change, ensuring governance cannot change fees
/// in a single transaction.
///
/// **Impact of changes:**
/// - Increasing this value would provide more reaction time but slow governance.
/// - Decreasing this value would allow faster changes but reduce observability.
///
/// **Used for:** Enforcing the minimum delay in `set_fee_bps` / `apply_fee_bps`.
pub const MIN_FEE_CHANGE_DELAY_SECONDS: u64 = 3_600;

/// Maximum timelock delay in seconds for treasury fee changes (30 days).
///
/// **Units:** Seconds
/// **Value:** 2,592,000 seconds (30 days)
///
/// **Rationale:** Bounds the delay so a queued fee change cannot be effectively
/// frozen indefinitely, while still allowing long observation windows.
///
/// **Impact of changes:**
/// - Increasing this value would allow longer delays but risks stalled changes.
/// - Decreasing this value would cap observation windows more tightly.
///
/// **Used for:** Enforcing the maximum delay in `set_fee_bps` / `apply_fee_bps`.
pub const MAX_FEE_CHANGE_DELAY_SECONDS: u64 = 2_592_000;

// ═══════════════════════════════════════════════════════════════════════════
// PAYOUT REMAINDER ALLOCATION INVARIANT
// ═══════════════════════════════════════════════════════════════════════════
//
// ## Invariant
//
// For every payout computation over a set of participants `P` with integer
// weights `w_i` and an integer distributable amount `T`, the contract MUST
// satisfy:
//
//     sum_{i in P} payout_i + retained_remainder == T
//
// where:
//   * `payout_i = floor(T * w_i / W)` and `W = sum_{i in P} w_i`,
//   * `retained_remainder = T - sum_{i in P} payout_i` is an explicit,
//     non-negative integer that is either (a) credited to a deterministic
//     recipient, or (b) retained by the pool escrow and reported as such.
//
// The allocation MUST be a pure function of `(T, {w_i})` and MUST NOT depend
// on the order in which participants claim, the order in which they were
// inserted, or any other mutable state. Two calls with the same inputs MUST
// produce byte-identical payout vectors.
//
// ## Deterministic tie-breaking
//
// When the remainder `R = T - sum_i floor(T * w_i / W)` is non-zero, the
// contract distributes `R` additional base units one at a time using the
// following deterministic rule:
//
//   1. Compute `r_i = (T * w_i) mod W` for each participant.
//   2. Sort participants by `(r_i DESC, index_i ASC)` where `index_i` is the
//      participant's stable position in the canonical, sorted participant
//      vector (lexicographic by address bytes).
//   3. Award one extra unit to the first `R` participants in that order.
//
// This is the "largest remainder" (Hamilton) method with a stable tie-break
// on canonical participant index. It guarantees:
//   * `sum_i payout_i == T` exactly (no dust is lost),
//   * each `payout_i` differs from `floor(T * w_i / W)` by at most 1,
//   * the result is independent of claim order and of storage iteration order.
//
// ## Application to single and batch payout paths
//
// Both `claim_winnings` (single) and `claim_winnings_batch` (batch) MUST use
// the same helper to compute the full payout vector for the pool, then select
// the caller's entry. A single claim therefore returns exactly the same amount
// as the corresponding entry of a batch claim, and the sum of all single
// claims equals the sum of a single batch claim over the same participant set.
//
// ## Retained remainder
//
// If a pool chooses to retain rather than distribute the remainder (e.g. to
// avoid dust payouts below `DEFAULT_GLOBAL_MIN_STAKE`), the retained amount
// MUST be tracked explicitly in pool state and emitted in the resolution
// event. The invariant `escrow == sum(payouts) + retained_remainder` MUST
// hold at all times after resolution.
//
// **Used for:** Documenting and validating payout math in `claim_winnings`,
// `claim_winnings_batch`, and the resolution/escrow accounting paths.

/// Fee schedule version reported for legacy pools created before per-pool fee
/// snapshots existed. Such pools have no stored `FeeSchedule` and settle against
/// the live schedule at resolution, exactly as they did before the upgrade.
pub const LEGACY_FEE_SCHEDULE_VERSION: u32 = 0;

/// Number of decimals in the canonical fixed-point price representation.
///
/// Every stored feed price, confidence and `PriceCondition.target_price` is an
/// integer number of `10^-PRICE_DECIMALS` units (i.e. exponent `-8`, the
/// common Pyth USD exponent). `1.00 USD` is stored as `100_000_000`.
pub const PRICE_DECIMALS: u32 = 8;

/// Smallest provider exponent accepted by `update_price_feed_with_expo`.
pub const MIN_PRICE_EXPONENT: i32 = -18;

/// Largest provider exponent accepted by `update_price_feed_with_expo`.
pub const MAX_PRICE_EXPONENT: i32 = 18;

/// Maximum resolution challenge window: 7 days in seconds.
///
/// Bounds how long payouts can be held back after resolution while
/// participants may still dispute the result.
pub const MAX_CHALLENGE_WINDOW: u64 = 604_800;

/// Maximum byte length of a challenge `evidence_uri`.
pub const MAX_EVIDENCE_URI_LEN: u32 = 512;

#[cfg(test)]
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;

    #[test]
    fn test_ledger_constants_are_positive() {
        assert!(DAY_IN_LEDGERS > 0);
        assert!(BUMP_THRESHOLD > 0);
        assert!(BUMP_AMOUNT > 0);
    }

    #[test]
    fn test_bump_threshold_less_than_bump_amount() {
        // Bump threshold should be less than bump amount to ensure
        // storage is extended before it expires
        assert!(BUMP_THRESHOLD < BUMP_AMOUNT);
    }

    #[test]
    fn test_pool_duration_is_reasonable() {
        // Default minimum pool duration should be at least 1 hour
        assert!(DEFAULT_MIN_POOL_DURATION >= 3600);
    }

    #[test]
    fn test_max_options_is_reasonable() {
        // Max options should be between 2 and 1000
        assert!(MAX_OUTCOMES_PER_POOL >= 2);
        assert!(MAX_OUTCOMES_PER_POOL <= 1000);
        assert_eq!(MAX_OPTIONS_COUNT, MAX_OUTCOMES_PER_POOL);
    }

    #[test]
    fn test_resource_limit_relationships() {
        // The aggregate cap exceeds the maximum initial liquidity so a pool
        // can always seed itself and still accept user stakes.
        assert!(MAX_AGGREGATE_STAKE_PER_POOL > MAX_INITIAL_LIQUIDITY);
        // The prediction cap must leave room for the pool creator plus users.
        assert!(MAX_PREDICTIONS_PER_POOL > 1);
    }

    #[test]
    fn test_aggregate_stake_keeps_payout_product_overflow_safe() {
        // The proportional payout path computes user_stake * payout_pool.
        // Keeping both operands below sqrt(i128::MAX) guarantees that product
        // cannot overflow before the division by winning_stake.
        assert!(
            MAX_AGGREGATE_STAKE_PER_POOL
                <= i128::MAX / MAX_AGGREGATE_STAKE_PER_POOL,
            "aggregate cap must leave room for overflow-safe payout products"
        );
    }

    #[test]
    fn test_max_initial_liquidity_is_positive() {
        assert!(MAX_INITIAL_LIQUIDITY > 0);
    }

    #[test]
    fn test_high_value_threshold_is_positive() {
        assert!(HIGH_VALUE_THRESHOLD > 0);
    }

    #[test]
    fn test_prediction_cooldown_is_non_negative() {
        assert_eq!(DEFAULT_PREDICTION_COOLDOWN_SECONDS, 0);
    }

    #[test]
    fn test_contract_version_is_positive() {
        assert!(CONTRACT_VERSION > 0);
    }

    #[test]
    fn test_ledger_time_constant_relationships() {
        assert_eq!(DAY_IN_LEDGERS, 17280);
        assert_eq!(DAY_IN_LEDGERS, 86400 / 5);
        assert_eq!(BUMP_THRESHOLD, 14 * DAY_IN_LEDGERS);
        assert_eq!(BUMP_AMOUNT, 30 * DAY_IN_LEDGERS);
    }

    #[test]
    fn test_bump_threshold_is_14_days() {
        assert_eq!(BUMP_THRESHOLD, 14 * DAY_IN_LEDGERS);
    }

    #[test]
    fn test_bump_amount_is_30_days() {
        assert_eq!(BUMP_AMOUNT, 30 * DAY_IN_LEDGERS);
    }

    #[test]
    fn test_claim_window_covers_bump_amount() {
        // A single TTL extension at resolution time must cover the entire
        // promised claim window, otherwise users could lose the ability to
        // claim before the window ends.
        assert!(CLAIM_WINDOW_LEDGERS > BUMP_AMOUNT);
    }

    #[test]
    fn test_claim_window_is_90_days() {
        assert_eq!(CLAIM_WINDOW_LEDGERS, 90 * DAY_IN_LEDGERS);
    }

    #[test]
    fn test_temporary_entry_ttl_is_one_day() {
        assert_eq!(TEMPORARY_ENTRY_TTL_LEDGERS, DAY_IN_LEDGERS);
    }

    #[test]
    fn test_ledger_calculations() {
        assert_eq!(BUMP_THRESHOLD, 14 * DAY_IN_LEDGERS);
        assert_eq!(BUMP_AMOUNT, 30 * DAY_IN_LEDGERS);
    }

    #[test]
    fn test_high_value_threshold_equals_100_usdc() {
        // At 7 decimals, 1 USDC = 10_000_000 base units.
        // Therefore 1_000_000_000 base units equals 100 USDC.
        assert_eq!(HIGH_VALUE_THRESHOLD, 1_000_000_000);
    }

    #[test]
    fn test_batch_claim_limits_are_consistent() {
        // The hard ceiling must be positive and small enough to stay within
        // Soroban's per-transaction resource budget.
        assert!(MAX_BATCH_CLAIM_SIZE > 0);
        assert!(MAX_BATCH_CLAIM_SIZE <= 100);
        // The recommended client chunk size must not exceed the hard ceiling.
        assert!(RECOMMENDED_BATCH_CLAIM_CHUNK_SIZE > 0);
        assert!(RECOMMENDED_BATCH_CLAIM_CHUNK_SIZE <= MAX_BATCH_CLAIM_SIZE);
    }

    #[test]
    fn test_max_batch_claim_size_regression_threshold() {
        // Regression guard: the maximum supported batch size is pinned so any
        // change requires re-running the batch-claim benchmark.
        assert_eq!(MAX_BATCH_CLAIM_SIZE, 25);
    }

    #[test]
    fn test_pause_operation_bits_are_distinct() {
        assert_eq!(PAUSE_OP_CREATE_POOL & PAUSE_OP_PLACE_PREDICTION, 0);
        assert_eq!(PAUSE_OP_CREATE_POOL & PAUSE_OP_RESOLVE_POOL, 0);
        assert_eq!(PAUSE_OP_PLACE_PREDICTION & PAUSE_OP_RESOLVE_POOL, 0);
    }

    #[test]
    fn test_pause_op_all_includes_every_pausable_operation() {
        assert_eq!(
            PAUSE_OP_ALL,
            PAUSE_OP_CREATE_POOL | PAUSE_OP_PLACE_PREDICTION | PAUSE_OP_RESOLVE_POOL
        );
    }

    #[test]
    fn test_no_contest_outcome_distinct_from_unresolved() {
        assert_ne!(NO_CONTEST_OUTCOME, UNRESOLVED_OUTCOME);
        assert!(NO_CONTEST_OUTCOME >= MAX_OPTIONS_COUNT);
    }

    #[test]
    fn test_no_contest_refund_policy_is_fee_free() {
        // Refunds must return the full stake: no protocol fee, no referral payout.
        assert_eq!(NO_CONTEST_FEE_BPS, 0);
        assert!(!NO_CONTEST_PAYS_REFERRALS);
    }

    #[test]
    fn test_no_contest_fee_bps_within_bounds() {
        assert!(NO_CONTEST_FEE_BPS <= MAX_TOLERANCE);
    }

    #[test]
    fn test_payout_remainder_invariant_documented() {
        // The remainder allocation invariant is documented above and must
        // hold for every (T, {w_i}) pair used by payout paths. This test
        // pins the invariant's arithmetic form so that any change to the
        // documented rule is a deliberate, reviewable edit.
        let t: i128 = 1_000_000_007;
        let weights: [i128; 3] = [1, 2, 3];
        let w: i128 = weights.iter().sum();
        let floors: i128 = weights.iter().map(|wi| t * wi / w).sum();
        let remainder = t - floors;
        assert!(remainder >= 0);
        assert!(remainder < weights.len() as i128);
        assert_eq!(floors + remainder, t);
    }
}
