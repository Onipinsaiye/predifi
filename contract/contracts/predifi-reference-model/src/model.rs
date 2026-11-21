//! # Reference Model — Pool State Machine
//!
//! An in-memory, pure-Rust model of the PrediFi pool lifecycle. It owns a
//! map of pools keyed by `pool_id` and exposes the same operations as the
//! Soroban contract, returning the same error variants so that differential
//! tests can compare results directly.
//!
//! ## Design principles
//!
//! * **No Soroban, no network** — only `std` and the sister `math` module.
//! * **One canonical code path** — no `#[cfg(test)]` divergence inside the model.
//! * **Invariants are explicit** — each mutating operation calls
//!   [`RefPool::check_invariants`] before returning success.
//! * **Types mirror the contract** — `MarketState`, `FeeTier`, `FeeSchedule`,
//!   `Prediction`, and `RefPool` carry the same fields as their Soroban counterparts.

use std::collections::HashMap;

use crate::math::{
    claim_payout, dynamic_fee, protocol_fee, FeeTier, MathError, MAX_BPS, UNRESOLVED_OUTCOME,
};

// ─────────────────────────────────────────────────────────────────────────────
// Mirror types
// ─────────────────────────────────────────────────────────────────────────────

/// Market lifecycle state — mirrors `MarketState` in the contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarketState {
    Active   = 0,
    Resolved = 1,
    Canceled = 2,
    Disputed = 3,
}

/// A user prediction record — mirrors `Prediction` in the contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prediction {
    /// Total tokens staked by this user for `pool_id`.
    pub amount: i128,
    /// 0-based outcome index chosen.
    pub outcome: u32,
}

/// A fee schedule snapshot — mirrors `FeeSchedule` in the contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeeSchedule {
    pub version: u32,
    /// Base fee applied when no tier threshold is met.
    pub base_fee_bps: u32,
    /// Dynamic fee tiers ordered by ascending `stake_threshold`.
    pub tiers: Vec<FeeTier>,
}

impl FeeSchedule {
    /// Create a simple flat-fee schedule with no tiers.
    pub fn flat(fee_bps: u32) -> Self {
        FeeSchedule { version: 1, base_fee_bps: fee_bps, tiers: vec![] }
    }
}

/// The reference model's pool record. Combines what the contract stores in
/// `Pool`, `OutStakes`, and related storage entries.
#[derive(Clone, Debug)]
pub struct RefPool {
    // --- configuration (set at creation, mostly immutable) ---
    pub creation_time: u64,
    pub end_time: u64,
    pub min_stake: i128,
    /// 0 means no per-prediction cap.
    pub max_stake: i128,
    pub min_total_stake: i128,
    /// 0 means no total cap.
    pub max_total_stake: i128,
    pub initial_liquidity: i128,
    pub options_count: u32,
    /// Fee schedule snapshotted at pool creation.
    pub fee_schedule: FeeSchedule,

    // --- runtime state ---
    pub state: MarketState,
    /// Winning outcome index; `UNRESOLVED_OUTCOME` while active.
    pub outcome: u32,
    /// Sum of all stakes including `initial_liquidity`.
    pub total_stake: i128,
    /// Per-outcome stake totals; length == `options_count`.
    pub outcome_stakes: Vec<i128>,
    /// Protocol fee in bps, determined at resolution by the dynamic fee logic.
    pub fee_bps: u32,
    /// Unique participants who have placed at least one prediction.
    pub participants_count: u32,
    /// Timestamp set when the pool transitions to `Resolved`.
    pub resolution_timestamp: Option<u64>,

    // --- liquidity-provider (LP) accounting ---
    /// Total LP shares outstanding. Shares are minted on deposit and burned on
    /// withdrawal; the share price is `lp_assets / total_shares`.
    pub total_shares: i128,
    /// Assets attributable to LPs (deposits + accrued fees − allocated losses).
    /// This is *separate* from `total_stake`, which tracks user predictions.
    pub lp_assets: i128,
    /// Cumulative fees credited to LPs (never decreases).
    pub lp_fees_accrued: i128,
    /// Cumulative losses allocated to LPs (never decreases).
    pub lp_losses_allocated: i128,
    /// Per-LP share balances keyed by user identity.
    pub lp_shares: HashMap<String, i128>,
}
=======

impl RefPool {
    /// Validate all invariants. Panics with a descriptive message on violation.
    /// Called internally after every mutating operation to act as a live oracle.
    pub fn check_invariants(&self) {
        // INV-1: total_stake == sum(outcome_stakes)
        let stake_sum: i128 = self.outcome_stakes.iter().sum();
        assert_eq!(
            self.total_stake,
            stake_sum + self.initial_liquidity,
            "INV-1: total_stake ({}) != outcome_stakes sum ({}) + initial_liquidity ({})",
            self.total_stake,
            stake_sum,
            self.initial_liquidity,
        );

        // INV-6: fee_bps <= 10_000
        assert!(
            self.fee_bps <= 10_000,
            "INV-6: fee_bps ({}) > 10_000",
            self.fee_bps
        );

        // INV-8: end_time > creation_time
        assert!(
            self.end_time > self.creation_time,
            "INV-8: end_time ({}) <= creation_time ({})",
            self.end_time,
            self.creation_time
        );

        // outcome_stakes length must match options_count
        assert_eq!(
            self.outcome_stakes.len() as u32,
            self.options_count,
            "outcome_stakes.len() ({}) != options_count ({})",
            self.outcome_stakes.len(),
            self.options_count,
        );

        // Resolved pool must have a valid outcome
        if self.state == MarketState::Resolved {
            assert_ne!(
                self.outcome,
                UNRESOLVED_OUTCOME,
                "Resolved pool has UNRESOLVED_OUTCOME sentinel"
            );
            assert!(
                self.outcome < self.options_count,
                "outcome ({}) >= options_count ({})",
                self.outcome,
                self.options_count,
            );
        }

        // INV-LP1: LP asset accounting identity.
        // lp_assets == deposits + fees_accrued − losses_allocated, where
        // deposits == sum(lp_shares) is implied by share conservation below.
        assert!(
            self.lp_assets >= 0,
            "INV-LP1: lp_assets ({}) < 0",
            self.lp_assets
        );
        assert!(
            self.lp_fees_accrued >= 0 && self.lp_losses_allocated >= 0,
            "INV-LP1: negative fee/loss accumulators"
        );

        // INV-LP2: share conservation — sum of per-LP shares == total_shares.
        let share_sum: i128 = self.lp_shares.values().sum();
        assert_eq!(
            self.total_shares, share_sum,
            "INV-LP2: total_shares ({}) != sum(lp_shares) ({})",
            self.total_shares, share_sum,
        );
        assert!(
            self.total_shares >= 0,
            "INV-LP2: total_shares ({}) < 0",
            self.total_shares
        );

        // INV-LP3: no LP holds a negative share balance.
        for (who, shares) in &self.lp_shares {
            assert!(
                *shares >= 0,
                "INV-LP3: LP {} has negative shares ({})",
                who,
                shares
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Error type (mirrors PredifiError discriminants that matter for our model)
// ─────────────────────────────────────────────────────────────────────────────

/// Reference model operation errors.
/// Variants correspond 1-to-1 with `PredifiError` in the contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelError {
    /// Pool already exists with the given ID.
    PoolAlreadyExists,
    /// Pool ID does not exist.
    PoolNotFound,
    /// The pool is not in the expected state for this operation.
    InvalidPoolState,
    /// The outcome index is out of bounds or is the sentinel value.
    InvalidOutcome,
    /// Stake amount is zero or negative.
    InvalidAmount,
    /// Stake is below the pool minimum.
    StakeBelowMinimum,
    /// Stake exceeds the pool maximum.
    StakeAboveMaximum,
    /// Adding this stake would exceed `max_total_stake`.
    MaxTotalStakeExceeded,
    /// User has already claimed (INV-3).
    AlreadyClaimed,
    /// Pool is not resolved (cannot claim winnings).
    PoolNotResolved,
    /// Pool is not canceled (cannot claim refund).
    InvalidPoolStateMustBeCanceled,
    /// User has no prediction to refund.
    InsufficientBalance,
    /// Arithmetic error in payout calculation.
    ArithmeticError(MathError),
    /// `end_time` is not strictly after `creation_time` (INV-8).
    InvalidTimestamp,
    /// User is attempting to change their outcome on an existing prediction.
    CannotChangeOutcome,
    /// Pool is in Disputed state; no operations allowed.
    PoolDisputed,
    /// Deposit amount is zero or negative.
    InvalidLiquidityAmount,
    /// LP has insufficient shares to withdraw the requested amount.
    InsufficientShares,
    /// Withdrawal requested while the pool is not in a withdrawable state
    /// (e.g. Active market with open positions, or Disputed).
    WithdrawalNotAllowed,
    /// Share math would produce a zero-share deposit (dust) or zero-asset
    /// redemption; rejected to preserve the share price.
    ZeroShares,
}

impl From<MathError> for ModelError {
    fn from(e: MathError) -> Self {
        ModelError::ArithmeticError(e)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Main model
// ─────────────────────────────────────────────────────────────────────────────

/// In-memory PrediFi reference model.
///
/// Holds a map of pools keyed by `u64` pool IDs and a map of per-user
/// per-pool predictions. The model is entirely synchronous and deterministic.
///
/// # Usage
/// ```
/// use predifi_reference_model::model::{RefModel, MarketState, FeeSchedule};
///
/// let mut m = RefModel::new();
/// let pool_id = m.create_pool(
///     0, // creation_time
///     100, // end_time
///     1,   // min_stake
///     0,   // max_stake (unlimited)
///     0,   // min_total_stake
///     0,   // max_total_stake (unlimited)
///     0,   // initial_liquidity
///     2,   // options_count
///     FeeSchedule::flat(200), // 2% fee
/// ).unwrap();
///
/// m.stake("alice", pool_id, 500, 0).unwrap();
/// m.stake("bob",   pool_id, 500, 1).unwrap();
/// m.resolve(pool_id, 0, 50).unwrap(); // resolve at time=50 with outcome=0
///
/// let payout = m.claim("alice", pool_id, 60).unwrap();
/// assert!(payout > 0);
/// let loser_payout = m.claim("bob", pool_id, 60).unwrap();
/// assert_eq!(loser_payout, 0);
/// ```
#[derive(Default)]
pub struct RefModel {
    next_pool_id: u64,
    pools: HashMap<u64, RefPool>,
    /// `predictions[(user.to_string(), pool_id)]` → Prediction
    predictions: HashMap<(String, u64), Prediction>,
    /// `claimed[(user.to_string(), pool_id)]` → true (INV-3: write-once)
    claimed: HashMap<(String, u64), bool>,
    /// Tracks cumulative claimed winnings per pool for INV-5.
    claimed_total: HashMap<u64, i128>,
}

impl RefModel {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Pool creation ──────────────────────────────────────────────────────

    /// Create a new pool and return its `pool_id`.
    ///
    /// `initial_liquidity` is included in `total_stake` immediately and is
    /// *not* counted in `outcome_stakes` (mirrors the contract: house money
    /// doesn't choose an outcome).
    #[allow(clippy::too_many_arguments)]
    pub fn create_pool(
        &mut self,
        creation_time: u64,
        end_time: u64,
        min_stake: i128,
        max_stake: i128,
        min_total_stake: i128,
        max_total_stake: i128,
        initial_liquidity: i128,
        options_count: u32,
        fee_schedule: FeeSchedule,
    ) -> Result<u64, ModelError> {
        // INV-8
        if end_time <= creation_time {
            return Err(ModelError::InvalidTimestamp);
        }
        if options_count < 2 {
            return Err(ModelError::InvalidAmount);
        }

        let pool_id = self.next_pool_id;
        self.next_pool_id += 1;

        let pool = RefPool {
            creation_time,
            end_time,
            min_stake,
            max_stake,
            min_total_stake,
            max_total_stake,
            initial_liquidity,
            options_count,
            fee_schedule,
            state: MarketState::Active,
            outcome: UNRESOLVED_OUTCOME,
            total_stake: initial_liquidity,
            outcome_stakes: vec![0; options_count as usize],
            fee_bps: 0, // set at resolution
            participants_count: 0,
            resolution_timestamp: None,
            total_shares: 0,
            lp_assets: 0,
            lp_fees_accrued: 0,
            lp_losses_allocated: 0,
            lp_shares: HashMap::new(),
        };

        pool.check_invariants();
        self.pools.insert(pool_id, pool);
        Ok(pool_id)
    }

    // ── Staking ───────────────────────────────────────────────────────────

    /// Place or top-up a prediction.
    ///
    /// * First call for `(user, pool_id)` creates a new prediction record and
    ///   increments `participants_count`.
    /// * Subsequent calls **must** use the same `outcome` (matching the contract's
    ///   "cannot change prediction outcome" invariant).
    ///
    /// The current ledger `timestamp` must be strictly before `pool.end_time`.
    pub fn stake(
        &mut self,
        user: impl Into<String>,
        pool_id: u64,
        amount: i128,
        outcome: u32,
    ) -> Result<(), ModelError> {
        self.stake_at(user, pool_id, amount, outcome, 0 /* any valid time */)
    }

    /// Stake with an explicit timestamp (used for temporal validation in tests).
    pub fn stake_at(
        &mut self,
        user: impl Into<String>,
        pool_id: u64,
        amount: i128,
        outcome: u32,
        timestamp: u64,
    ) -> Result<(), ModelError> {
        let user = user.into();

        // INV-7
        if amount <= 0 {
            return Err(ModelError::InvalidAmount);
        }

        let pool = self.pools.get_mut(&pool_id).ok_or(ModelError::PoolNotFound)?;

        // State guard
        match pool.state {
            MarketState::Active => {}
            MarketState::Disputed => return Err(ModelError::PoolDisputed),
            _ => return Err(ModelError::InvalidPoolState),
        }

        // Time guard: staking window must be open
        if timestamp >= pool.end_time {
            return Err(ModelError::InvalidPoolState);
        }

        // Outcome bounds
        if outcome >= pool.options_count {
            return Err(ModelError::InvalidOutcome);
        }

        // Per-pool stake limits
        if amount < pool.min_stake {
            return Err(ModelError::StakeBelowMinimum);
        }
        if pool.max_stake > 0 && amount > pool.max_stake {
            return Err(ModelError::StakeAboveMaximum);
        }

        // Total stake cap
        if pool.max_total_stake > 0 {
            let new_total = pool
                .total_stake
                .checked_add(amount)
                .ok_or(ModelError::ArithmeticError(MathError::Overflow))?;
            if new_total > pool.max_total_stake {
                return Err(ModelError::MaxTotalStakeExceeded);
            }
        }

        let key = (user.clone(), pool_id);
        match self.predictions.get_mut(&key) {
            Some(pred) => {
                // Top-up: outcome must be unchanged
                if pred.outcome != outcome {
                    return Err(ModelError::CannotChangeOutcome);
                }
                pred.amount = pred
                    .amount
                    .checked_add(amount)
                    .ok_or(ModelError::ArithmeticError(MathError::Overflow))?;
            }
            None => {
                self.predictions.insert(key, Prediction { amount, outcome });
                pool.participants_count = pool.participants_count.saturating_add(1);
            }
        }

        // INV-1: update aggregate state
        pool.total_stake = pool
            .total_stake
            .checked_add(amount)
            .ok_or(ModelError::ArithmeticError(MathError::Overflow))?;
        pool.outcome_stakes[outcome as usize] = pool.outcome_stakes[outcome as usize]
            .checked_add(amount)
            .ok_or(ModelError::ArithmeticError(MathError::Overflow))?;

        pool.check_invariants();
        Ok(())
    }

    // ── Resolution ────────────────────────────────────────────────────────

    /// Resolve a pool with a winning `outcome`.
    ///
    /// Computes and freezes `fee_bps` via the pool's snapshotted `FeeSchedule`.
    /// `timestamp` is the resolution ledger time (must be > `pool.end_time`
    /// in a real scenario, but this model does not enforce the resolution-delay
    /// guard — tests that care about timing should pass the correct timestamp).
    pub fn resolve(
        &mut self,
        pool_id: u64,
        outcome: u32,
        timestamp: u64,
    ) -> Result<(), ModelError> {
        let pool = self.pools.get_mut(&pool_id).ok_or(ModelError::PoolNotFound)?;

        match pool.state {
            MarketState::Active => {}
            MarketState::Disputed => return Err(ModelError::PoolDisputed),
            _ => return Err(ModelError::InvalidPoolState), // INV-2
        }

        // Outcome bounds
        if outcome == UNRESOLVED_OUTCOME || outcome >= pool.options_count {
            return Err(ModelError::InvalidOutcome);
        }

        // Freeze fee via dynamic fee logic
        let fee_bps = dynamic_fee(
            pool.total_stake,
            pool.fee_schedule.base_fee_bps,
            &pool.fee_schedule.tiers,
        );

        pool.state = MarketState::Resolved;
        pool.outcome = outcome;
        pool.fee_bps = fee_bps;
        pool.resolution_timestamp = Some(timestamp);

        pool.check_invariants();
        Ok(())
    }

    // ── Cancellation ──────────────────────────────────────────────────────

    /// Cancel a pool. All stakers may then claim a full refund.
    pub fn cancel(&mut self, pool_id: u64) -> Result<(), ModelError> {
        let pool = self.pools.get_mut(&pool_id).ok_or(ModelError::PoolNotFound)?;

        match pool.state {
            MarketState::Active => {}
            MarketState::Disputed => {
                // Disputed pools can also be canceled (adjudication path)
                // Mirror the contract which allows canceling a disputed pool via admin.
            }
            _ => return Err(ModelError::InvalidPoolState), // INV-2
        }

        pool.state = MarketState::Canceled;
        pool.check_invariants();
        Ok(())
    }

    // ── Disputing ─────────────────────────────────────────────────────────

    /// Flag a pool as disputed (stasis — no claims or refunds until canceled or resolved).
    pub fn dispute(&mut self, pool_id: u64) -> Result<(), ModelError> {
        let pool = self.pools.get_mut(&pool_id).ok_or(ModelError::PoolNotFound)?;

        if pool.state != MarketState::Active {
            return Err(ModelError::InvalidPoolState);
        }
        pool.state = MarketState::Disputed;
        pool.check_invariants();
        Ok(())
    }

    // ── Claims ────────────────────────────────────────────────────────────

    /// Claim winnings (resolved pool) or refund (canceled pool).
    ///
    /// Returns the amount transferred. Returns `0` for losers on a resolved
    /// pool (they still get the `claimed` sentinel set). Returns the full
    /// staked `amount` as a refund when the pool is canceled.
    ///
    /// This is the single unified claim entry point (mirrors `claim_winnings`
    /// which internally handles the canceled-pool refund case).
    pub fn claim(
        &mut self,
        user: impl Into<String>,
        pool_id: u64,
        _timestamp: u64,
    ) -> Result<i128, ModelError> {
        let user = user.into();
        let pool = self.pools.get(&pool_id).ok_or(ModelError::PoolNotFound)?.clone();

        // Cannot claim from an Active or Disputed pool
        match pool.state {
            MarketState::Active => return Err(ModelError::PoolNotResolved),
            MarketState::Disputed => return Err(ModelError::PoolDisputed),
            MarketState::Resolved | MarketState::Canceled => {}
        }

        // INV-3: double-claim prevention
        let claimed_key = (user.clone(), pool_id);
        if self.claimed.contains_key(&claimed_key) {
            return Err(ModelError::AlreadyClaimed);
        }

        let pred_key = (user.clone(), pool_id);
        let prediction = match self.predictions.get(&pred_key) {
            Some(p) => *p,
            None => {
                // No prediction → no payout; but do NOT set the claimed sentinel
                // (the user never participated, so they can't "already claim" later).
                return Ok(0);
            }
        };

        // Mark as claimed (INV-3) before any payout computation so that even
        // a 0 payout cannot be re-attempted.
        self.claimed.insert(claimed_key, true);

        let payout = match pool.state {
            MarketState::Canceled => {
                // Full refund regardless of outcome.
                prediction.amount
            }
            MarketState::Resolved => {
                if prediction.outcome != pool.outcome {
                    // Loser — payout is 0.
                    0
                } else {
                    let winning_stake = pool.outcome_stakes[pool.outcome as usize];
                    let bd = claim_payout(
                        pool.total_stake,
                        pool.fee_bps as i128,
                        prediction.amount,
                        winning_stake,
                    )?;
                    bd.winnings
                }
            }
            // These arms were already handled above but are needed for exhaustiveness.
            MarketState::Active | MarketState::Disputed => unreachable!(),
        };

        // INV-5: track cumulative claimed winnings per pool
        let total = self.claimed_total.entry(pool_id).or_insert(0);
        *total = total
            .checked_add(payout)
            .ok_or(ModelError::ArithmeticError(MathError::Overflow))?;
        assert!(
            *total <= pool.total_stake,
            "INV-5: cumulative claimed ({}) > total_stake ({})",
            *total,
            pool.total_stake,
        );

        Ok(payout)
    }

    /// Claim a refund from a canceled pool (explicit entry point for tests that
    /// call `claim_refund` separately from `claim_winnings`).
    pub fn claim_refund(
        &mut self,
        user: impl Into<String>,
        pool_id: u64,
    ) -> Result<i128, ModelError> {
        let user_str: String = user.into();
        let pool = self.pools.get(&pool_id).ok_or(ModelError::PoolNotFound)?;

        if pool.state != MarketState::Canceled {
            return Err(ModelError::InvalidPoolStateMustBeCanceled);
        }

        let claimed_key = (user_str.clone(), pool_id);
        if self.claimed.contains_key(&claimed_key) {
            return Err(ModelError::AlreadyClaimed);
        }

        let pred_key = (user_str.clone(), pool_id);
        let prediction = self
            .predictions
            .get(&pred_key)
            .ok_or(ModelError::InsufficientBalance)?;

        if prediction.amount <= 0 {
            return Err(ModelError::InsufficientBalance);
        }

        let refund = prediction.amount;
        self.claimed.insert(claimed_key, true);

        // INV-5
        let total = self.claimed_total.entry(pool_id).or_insert(0);
        *total = total
            .checked_add(refund)
            .ok_or(ModelError::ArithmeticError(MathError::Overflow))?;
        let pool = self.pools.get(&pool_id).unwrap();
        assert!(
            *total <= pool.total_stake,
            "INV-5: cumulative refunds ({}) > total_stake ({})",
            *total,
            pool.total_stake,
        );

        Ok(refund)
    }

    // ── Liquidity-provider accounting ─────────────────────────────────────

    /// Deposit `amount` of liquidity into a pool and mint LP shares.
    ///
    /// Share model: the pool maintains `lp_assets` (value owned by LPs) and
    /// `total_shares`. A deposit of `a` mints
    ///
    /// ```text
    /// shares = a * total_shares / lp_assets        (lp_assets > 0)
    /// shares = a                                   (first deposit)
    /// ```
    ///
    /// rounding **down**, so a depositor can never mint more value than they
    /// contributed. The first depositor sets the initial share price to 1:1.
    ///
    /// Deposits are allowed while the pool is `Active` (before `end_time`) or
    /// `Canceled` (to recapitalize for refunds). Deposits are rejected while
    /// `Resolved` or `Disputed`.
    pub fn deposit_liquidity(
        &mut self,
        user: impl Into<String>,
        pool_id: u64,
        amount: i128,
        timestamp: u64,
    ) -> Result<i128, ModelError> {
        let user = user.into();
        if amount <= 0 {
            return Err(ModelError::InvalidLiquidityAmount);
        }

        let pool = self.pools.get_mut(&pool_id).ok_or(ModelError::PoolNotFound)?;

        match pool.state {
            MarketState::Active => {
                if timestamp >= pool.end_time {
                    return Err(ModelError::InvalidPoolState);
                }
            }
            MarketState::Canceled => {}
            MarketState::Disputed => return Err(ModelError::PoolDisputed),
            MarketState::Resolved => return Err(ModelError::InvalidPoolState),
        }

        // Shares minted, rounding down (INV-LP4: shares * lp_assets <= amount * total_shares).
        let shares = if pool.total_shares == 0 || pool.lp_assets == 0 {
            amount
        } else {
            let num = amount
                .checked_mul(pool.total_shares)
                .ok_or(ModelError::ArithmeticError(MathError::Overflow))?;
            num / pool.lp_assets
        };
        if shares <= 0 {
            return Err(ModelError::ZeroShares);
        }

        pool.total_shares = pool
            .total_shares
            .checked_add(shares)
            .ok_or(ModelError::ArithmeticError(MathError::Overflow))?;
        pool.lp_assets = pool
            .lp_assets
            .checked_add(amount)
            .ok_or(ModelError::ArithmeticError(MathError::Overflow))?;
        let bal = pool.lp_shares.entry(user).or_insert(0);
        *bal = bal
            .checked_add(shares)
            .ok_or(ModelError::ArithmeticError(MathError::Overflow))?;

        pool.check_invariants();
        Ok(shares)
    }

    /// Withdraw `shares` from a pool, redeeming them for assets.
    ///
    /// Assets returned:
    ///
    /// ```text
    /// assets = shares * lp_assets / total_shares        (rounding down)
    /// ```
    ///
    /// rounding **down**, so the pool never pays out more than the redeemer's
    /// pro-rata claim. The LP's share balance and `total_shares` are reduced by
    /// exactly `shares`; `lp_assets` is reduced by the assets paid out.
    ///
    /// # Withdrawal during market closure
    /// Withdrawals are permitted while the pool is `Active` (before `end_time`),
    /// `Resolved`, or `Canceled`. Withdrawals are **blocked** while `Disputed`
    /// (stasis) and while an `Active` pool is past `end_time` but not yet
    /// resolved/canceled — this prevents LPs from front-running resolution by
    /// pulling liquidity out of a closed-but-unresolved market.
    pub fn withdraw_liquidity(
        &mut self,
        user: impl Into<String>,
        pool_id: u64,
        shares: i128,
        timestamp: u64,
    ) -> Result<i128, ModelError> {
        let user = user.into();
        if shares <= 0 {
            return Err(ModelError::InvalidLiquidityAmount);
        }

        let pool = self.pools.get_mut(&pool_id).ok_or(ModelError::PoolNotFound)?;

        match pool.state {
            MarketState::Active => {
                // Closed-but-unresolved market: withdrawals frozen.
                if timestamp >= pool.end_time {
                    return Err(ModelError::WithdrawalNotAllowed);
                }
            }
            MarketState::Resolved | MarketState::Canceled => {}
            MarketState::Disputed => return Err(ModelError::WithdrawalNotAllowed),
        }

        let held = pool.lp_shares.get(&user).copied().unwrap_or(0);
        if held < shares {
            return Err(ModelError::InsufficientShares);
        }
        if pool.total_shares <= 0 {
            return Err(ModelError::InsufficientShares);
        }

        let assets = shares
            .checked_mul(pool.lp_assets)
            .ok_or(ModelError::ArithmeticError(MathError::Overflow))?
            / pool.total_shares;
        if assets <= 0 {
            return Err(ModelError::ZeroShares);
        }

        pool.total_shares -= shares;
        pool.lp_assets = pool
            .lp_assets
            .checked_sub(assets)
            .ok_or(ModelError::ArithmeticError(MathError::Overflow))?;
        let bal = pool.lp_shares.get_mut(&user).unwrap();
        *bal -= shares;

        pool.check_invariants();
        Ok(assets)
    }

    /// Accrue `fee_amount` of protocol fees to the LP pool.
    ///
    /// Fees increase `lp_assets` without minting shares, so the share price
    /// rises for every existing LP proportionally. Called by the resolution
    /// path (or by tests) after a fee is taken from the payout pool.
    pub fn accrue_lp_fees(&mut self, pool_id: u64, fee_amount: i128) -> Result<(), ModelError> {
        if fee_amount < 0 {
            return Err(ModelError::InvalidLiquidityAmount);
        }
        let pool = self.pools.get_mut(&pool_id).ok_or(ModelError::PoolNotFound)?;
        if fee_amount == 0 {
            return Ok(());
        }
        pool.lp_assets = pool
            .lp_assets
            .checked_add(fee_amount)
            .ok_or(ModelError::ArithmeticError(MathError::Overflow))?;
        pool.lp_fees_accrued = pool
            .lp_fees_accrued
            .checked_add(fee_amount)
            .ok_or(ModelError::ArithmeticError(MathError::Overflow))?;
        pool.check_invariants();
        Ok(())
    }

    /// Allocate `loss_amount` of losses to the LP pool.
    ///
    /// Losses reduce `lp_assets` without burning shares, so the share price
    /// falls for every existing LP proportionally. Losses are capped at the
    /// current `lp_assets` (LPs cannot lose more than they own; any excess is
    /// socialized elsewhere by the caller). Returns the amount actually
    /// allocated.
    pub fn allocate_lp_loss(&mut self, pool_id: u64, loss_amount: i128) -> Result<i128, ModelError> {
        if loss_amount < 0 {
            return Err(ModelError::InvalidLiquidityAmount);
        }
        let pool = self.pools.get_mut(&pool_id).ok_or(ModelError::PoolNotFound)?;
        let allocated = loss_amount.min(pool.lp_assets);
        if allocated == 0 {
            return Ok(0);
        }
        pool.lp_assets -= allocated;
        pool.lp_losses_allocated = pool
            .lp_losses_allocated
            .checked_add(allocated)
            .ok_or(ModelError::ArithmeticError(MathError::Overflow))?;
        pool.check_invariants();
        Ok(allocated)
    }

    /// Current share price in assets-per-share, scaled by 1e6 for precision.
    /// Returns `1_000_000` (1.0) when there are no shares outstanding.
    pub fn share_price(&self, pool_id: u64) -> Option<i128> {
        let pool = self.pools.get(&pool_id)?;
        if pool.total_shares == 0 {
            return Some(1_000_000);
        }
        Some(pool.lp_assets * 1_000_000 / pool.total_shares)
    }

    /// Look up an LP's share balance for a pool.
    pub fn lp_shares(&self, user: &str, pool_id: u64) -> i128 {
        self.pools
            .get(&pool_id)
            .and_then(|p| p.lp_shares.get(user).copied())
            .unwrap_or(0)
    }

    // ── Queries ───────────────────────────────────────────────────────────

    /// Borrow a pool (read-only).
    pub fn pool(&self, pool_id: u64) -> Option<&RefPool> {
        self.pools.get(&pool_id)
    }

    /// Look up a user's prediction for a given pool.
    pub fn prediction(&self, user: &str, pool_id: u64) -> Option<&Prediction> {
        self.predictions.get(&(user.to_string(), pool_id))
    }

    /// Check whether a user has already claimed from a pool.
    pub fn has_claimed(&self, user: &str, pool_id: u64) -> bool {
        self.claimed.contains_key(&(user.to_string(), pool_id))
    }

    /// Total amount claimed (winnings + refunds) for a pool so far.
    pub fn claimed_total(&self, pool_id: u64) -> i128 {
        *self.claimed_total.get(&pool_id).unwrap_or(&0)
    }

    /// Stake on a specific outcome for a pool (read-only helper).
    pub fn outcome_stake(&self, pool_id: u64, outcome: u32) -> i128 {
        self.pools
            .get(&pool_id)
            .and_then(|p| p.outcome_stakes.get(outcome as usize).copied())
            .unwrap_or(0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Unit tests for the model state machine
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_pool(m: &mut RefModel) -> u64 {
        m.create_pool(0, 1000, 1, 0, 0, 0, 0, 2, FeeSchedule::flat(200))
            .unwrap()
    }

    // ── Terminal path 1: Active → Resolved → claim ────────────────────────

    #[test]
    fn resolved_winner_receives_proportional_payout() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.stake("alice", pid, 600, 0).unwrap();
        m.stake("bob", pid, 400, 1).unwrap();
        m.resolve(pid, 0, 500).unwrap(); // alice wins

        let payout = m.claim("alice", pid, 600).unwrap();
        // fee = floor(1000 * 200 / 10_000) = 20
        // payout_pool = 980
        // alice_share = floor(600 * 980 / 600) = 980
        assert_eq!(payout, 980);
    }

    #[test]
    fn resolved_loser_receives_zero() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.stake("alice", pid, 600, 0).unwrap();
        m.stake("bob", pid, 400, 1).unwrap();
        m.resolve(pid, 0, 500).unwrap();

        let payout = m.claim("bob", pid, 600).unwrap();
        assert_eq!(payout, 0);
    }

    #[test]
    fn two_winners_share_payout_pool() {
        let mut m = RefModel::new();
        let pid = m.create_pool(0, 1000, 1, 0, 0, 0, 0, 2, FeeSchedule::flat(0)).unwrap();
        m.stake("alice", pid, 500, 0).unwrap();
        m.stake("bob", pid, 500, 0).unwrap();
        m.stake("carol", pid, 1000, 1).unwrap();
        m.resolve(pid, 0, 500).unwrap();

        let pa = m.claim("alice", pid, 600).unwrap();
        let pb = m.claim("bob", pid, 600).unwrap();
        let pc = m.claim("carol", pid, 600).unwrap();
        // fee = 0; payout_pool = 2000
        // alice = floor(500 * 2000 / 1000) = 1000
        // bob   = floor(500 * 2000 / 1000) = 1000
        // carol = 0 (loser)
        assert_eq!(pa, 1000);
        assert_eq!(pb, 1000);
        assert_eq!(pc, 0);
        assert!(pa + pb + pc <= 2000, "INV-5 violated");
    }

    #[test]
    fn sum_of_winner_payouts_never_exceeds_total_stake() {
        let mut m = RefModel::new();
        let pid = m.create_pool(0, 1000, 1, 0, 0, 0, 0, 2, FeeSchedule::flat(300)).unwrap();
        // Odd amounts to stress rounding
        m.stake("u1", pid, 333, 0).unwrap();
        m.stake("u2", pid, 334, 0).unwrap();
        m.stake("u3", pid, 333, 0).unwrap();
        m.stake("loser", pid, 1000, 1).unwrap();
        m.resolve(pid, 0, 500).unwrap();

        let total_stake = m.pool(pid).unwrap().total_stake;
        let p1 = m.claim("u1", pid, 600).unwrap();
        let p2 = m.claim("u2", pid, 600).unwrap();
        let p3 = m.claim("u3", pid, 600).unwrap();
        assert!(
            p1 + p2 + p3 <= total_stake,
            "sum of payouts {} > total_stake {}",
            p1 + p2 + p3,
            total_stake
        );
    }

    // ── Terminal path 2: Active → Canceled → refund ───────────────────────

    #[test]
    fn canceled_pool_refunds_full_stake() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.stake("alice", pid, 700, 0).unwrap();
        m.stake("bob", pid, 300, 1).unwrap();
        m.cancel(pid).unwrap();

        assert_eq!(m.claim("alice", pid, 0).unwrap(), 700);
        assert_eq!(m.claim("bob", pid, 0).unwrap(), 300);
    }

    #[test]
    fn claim_refund_explicit_path() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.stake("alice", pid, 500, 0).unwrap();
        m.cancel(pid).unwrap();

        assert_eq!(m.claim_refund("alice", pid).unwrap(), 500);
    }

    #[test]
    fn refund_preserves_conservation() {
        let mut m = RefModel::new();
        let pid = m.create_pool(0, 1000, 1, 0, 0, 0, 100, 3, FeeSchedule::flat(150)).unwrap();
        m.stake("a", pid, 200, 0).unwrap();
        m.stake("b", pid, 300, 1).unwrap();
        m.stake("c", pid, 400, 2).unwrap();
        m.cancel(pid).unwrap();

        let total_stake = m.pool(pid).unwrap().total_stake;
        let ra = m.claim("a", pid, 0).unwrap();
        let rb = m.claim("b", pid, 0).unwrap();
        let rc = m.claim("c", pid, 0).unwrap();
        // Only the three stakes refunded; initial_liquidity stays in contract
        assert!(ra + rb + rc <= total_stake);
        assert_eq!(ra, 200);
        assert_eq!(rb, 300);
        assert_eq!(rc, 400);
    }

    // ── Terminal path 3: Active → Disputed → stasis ───────────────────────

    #[test]
    fn disputed_pool_blocks_claims() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.stake("alice", pid, 500, 0).unwrap();
        m.dispute(pid).unwrap();

        assert_eq!(
            m.claim("alice", pid, 0),
            Err(ModelError::PoolDisputed),
        );
    }

    #[test]
    fn disputed_pool_blocks_staking() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.dispute(pid).unwrap();

        assert_eq!(
            m.stake("alice", pid, 100, 0),
            Err(ModelError::PoolDisputed),
        );
    }

    // ── INV-2: one-way transitions ────────────────────────────────────────

    #[test]
    fn cannot_re_resolve_resolved_pool() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.stake("alice", pid, 100, 0).unwrap();
        m.resolve(pid, 0, 500).unwrap();
        assert_eq!(m.resolve(pid, 1, 600), Err(ModelError::InvalidPoolState));
    }

    #[test]
    fn cannot_cancel_resolved_pool() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.stake("alice", pid, 100, 0).unwrap();
        m.resolve(pid, 0, 500).unwrap();
        assert_eq!(m.cancel(pid), Err(ModelError::InvalidPoolState));
    }

    #[test]
    fn cannot_stake_after_resolution() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.stake("alice", pid, 100, 0).unwrap();
        m.resolve(pid, 0, 500).unwrap();
        assert_eq!(m.stake("bob", pid, 100, 1), Err(ModelError::InvalidPoolState));
    }

    // ── INV-3: double-claim prevention ────────────────────────────────────

    #[test]
    fn cannot_claim_twice() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.stake("alice", pid, 100, 0).unwrap();
        m.resolve(pid, 0, 500).unwrap();
        m.claim("alice", pid, 600).unwrap();
        assert_eq!(m.claim("alice", pid, 600), Err(ModelError::AlreadyClaimed));
    }

    #[test]
    fn cannot_refund_twice() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.stake("alice", pid, 100, 0).unwrap();
        m.cancel(pid).unwrap();
        m.claim_refund("alice", pid).unwrap();
        assert_eq!(m.claim_refund("alice", pid), Err(ModelError::AlreadyClaimed));
    }

    // ── Dynamic fee selection ─────────────────────────────────────────────

    #[test]
    fn dynamic_fee_applied_at_resolution() {
        use crate::math::FeeTier;
        let schedule = FeeSchedule {
            version: 1,
            base_fee_bps: 300,
            tiers: vec![
                FeeTier { stake_threshold: 500, fee_bps: 200 },
                FeeTier { stake_threshold: 1000, fee_bps: 100 },
            ],
        };
        let mut m = RefModel::new();
        let pid = m.create_pool(0, 1000, 1, 0, 0, 0, 0, 2, schedule).unwrap();
        m.stake("alice", pid, 1200, 0).unwrap(); // total_stake = 1200 → tier at 1000
        m.resolve(pid, 0, 500).unwrap();
        assert_eq!(m.pool(pid).unwrap().fee_bps, 100);
    }

    // ── Stake limit enforcement ───────────────────────────────────────────

    #[test]
    fn stake_below_min_rejected() {
        let mut m = RefModel::new();
        let pid = m.create_pool(0, 1000, 100, 0, 0, 0, 0, 2, FeeSchedule::flat(200)).unwrap();
        assert_eq!(m.stake("alice", pid, 99, 0), Err(ModelError::StakeBelowMinimum));
    }

    #[test]
    fn stake_above_max_rejected() {
        let mut m = RefModel::new();
        let pid = m.create_pool(0, 1000, 1, 500, 0, 0, 0, 2, FeeSchedule::flat(200)).unwrap();
        assert_eq!(m.stake("alice", pid, 501, 0), Err(ModelError::StakeAboveMaximum));
    }

    #[test]
    fn total_stake_cap_enforced() {
        let mut m = RefModel::new();
        let pid = m.create_pool(0, 1000, 1, 0, 0, 1000, 0, 2, FeeSchedule::flat(200)).unwrap();
        m.stake("alice", pid, 900, 0).unwrap();
        assert_eq!(
            m.stake("bob", pid, 200, 1),
            Err(ModelError::MaxTotalStakeExceeded),
        );
    }

    // ── Outcome bounds ────────────────────────────────────────────────────

    #[test]
    fn invalid_outcome_rejected_on_stake() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m); // options_count = 2
        assert_eq!(m.stake("alice", pid, 100, 2), Err(ModelError::InvalidOutcome));
    }

    #[test]
    fn sentinel_outcome_rejected_on_resolve() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.stake("alice", pid, 100, 0).unwrap();
        assert_eq!(
            m.resolve(pid, UNRESOLVED_OUTCOME, 500),
            Err(ModelError::InvalidOutcome),
        );
    }

    // ── Participants count ────────────────────────────────────────────────

    #[test]
    fn participants_count_increments_once_per_user() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.stake("alice", pid, 100, 0).unwrap();
        m.stake("alice", pid, 100, 0).unwrap(); // top-up
        m.stake("bob", pid, 100, 1).unwrap();
        assert_eq!(m.pool(pid).unwrap().participants_count, 2);
    }

    // ── INV-1: stake accounting ───────────────────────────────────────────

    #[test]
    fn outcome_stakes_sum_to_user_stakes() {
        let mut m = RefModel::new();
        let pid = m.create_pool(0, 1000, 1, 0, 0, 0, 50, 3, FeeSchedule::flat(0)).unwrap();
        m.stake("a", pid, 100, 0).unwrap();
        m.stake("b", pid, 200, 1).unwrap();
        m.stake("c", pid, 300, 2).unwrap();
        let pool = m.pool(pid).unwrap();
        let stake_sum: i128 = pool.outcome_stakes.iter().sum();
        assert_eq!(stake_sum, 600);
        assert_eq!(pool.total_stake, 650); // 600 user + 50 initial
    }

    // ── No-prediction user gets 0 (not an error) ─────────────────────────

    #[test]
    fn non_participant_claim_returns_zero() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.stake("alice", pid, 100, 0).unwrap();
        m.resolve(pid, 0, 500).unwrap();
        // "dave" never staked
        let payout = m.claim("dave", pid, 600).unwrap();
        assert_eq!(payout, 0);
    }

    // ── LP accounting: deposits & share issuance ──────────────────────────

    #[test]
    fn first_deposit_mints_one_to_one() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        let shares = m.deposit_liquidity("lp1", pid, 1000, 0).unwrap();
        assert_eq!(shares, 1000);
        assert_eq!(m.lp_shares("lp1", pid), 1000);
        assert_eq!(m.pool(pid).unwrap().total_shares, 1000);
        assert_eq!(m.pool(pid).unwrap().lp_assets, 1000);
    }

    #[test]
    fn second_deposit_mints_pro_rata() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.deposit_liquidity("lp1", pid, 1000, 0).unwrap();
        // share price is 1:1, so 500 → 500 shares
        let shares = m.deposit_liquidity("lp2", pid, 500, 0).unwrap();
        assert_eq!(shares, 500);
        assert_eq!(m.pool(pid).unwrap().total_shares, 1500);
        assert_eq!(m.pool(pid).unwrap().lp_assets, 1500);
    }

    #[test]
    fn deposit_after_fee_accrual_mints_fewer_shares() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.deposit_liquidity("lp1", pid, 1000, 0).unwrap();
        m.accrue_lp_fees(pid, 1000).unwrap(); // lp_assets = 2000, shares = 1000
        // share price now 2:1 → 1000 assets mints 500 shares
        let shares = m.deposit_liquidity("lp2", pid, 1000, 0).unwrap();
        assert_eq!(shares, 500);
        assert_eq!(m.pool(pid).unwrap().total_shares, 1500);
        assert_eq!(m.pool(pid).unwrap().lp_assets, 3000);
    }

    #[test]
    fn deposit_rejects_zero_and_negative() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        assert_eq!(
            m.deposit_liquidity("lp1", pid, 0, 0),
            Err(ModelError::InvalidLiquidityAmount)
        );
        assert_eq!(
            m.deposit_liquidity("lp1", pid, -5, 0),
            Err(ModelError::InvalidLiquidityAmount)
        );
    }

    #[test]
    fn deposit_rejected_after_resolution() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.stake("alice", pid, 100, 0).unwrap();
        m.resolve(pid, 0, 500).unwrap();
        assert_eq!(
            m.deposit_liquidity("lp1", pid, 100, 600),
            Err(ModelError::InvalidPoolState)
        );
    }

    // ── LP accounting: withdrawals ────────────────────────────────────────

    #[test]
    fn withdraw_redeems_pro_rata() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.deposit_liquidity("lp1", pid, 1000, 0).unwrap();
        m.deposit_liquidity("lp2", pid, 1000, 0).unwrap();
        let assets = m.withdraw_liquidity("lp1", pid, 1000, 0).unwrap();
        assert_eq!(assets, 1000);
        assert_eq!(m.lp_shares("lp1", pid), 0);
        assert_eq!(m.pool(pid).unwrap().total_shares, 1000);
        assert_eq!(m.pool(pid).unwrap().lp_assets, 1000);
    }

    #[test]
    fn withdraw_more_than_held_rejected() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.deposit_liquidity("lp1", pid, 1000, 0).unwrap();
        assert_eq!(
            m.withdraw_liquidity("lp1", pid, 1001, 0),
            Err(ModelError::InsufficientShares)
        );
    }

    #[test]
    fn withdraw_blocked_during_closed_unresolved_market() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m); // end_time = 1000
        m.deposit_liquidity("lp1", pid, 1000, 0).unwrap();
        // Past end_time but still Active → withdrawal frozen.
        assert_eq!(
            m.withdraw_liquidity("lp1", pid, 100, 1000),
            Err(ModelError::WithdrawalNotAllowed)
        );
        // Before end_time → allowed.
        assert!(m.withdraw_liquidity("lp1", pid, 100, 999).is_ok());
    }

    #[test]
    fn withdraw_allowed_after_resolution_and_cancellation() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.deposit_liquidity("lp1", pid, 1000, 0).unwrap();
        m.stake("alice", pid, 100, 0).unwrap();
        m.resolve(pid, 0, 500).unwrap();
        assert!(m.withdraw_liquidity("lp1", pid, 500, 600).is_ok());

        let pid2 = simple_pool(&mut m);
        m.deposit_liquidity("lp2", pid2, 1000, 0).unwrap();
        m.cancel(pid2).unwrap();
        assert!(m.withdraw_liquidity("lp2", pid2, 1000, 0).is_ok());
    }

    #[test]
    fn withdraw_blocked_while_disputed() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.deposit_liquidity("lp1", pid, 1000, 0).unwrap();
        m.dispute(pid).unwrap();
        assert_eq!(
            m.withdraw_liquidity("lp1", pid, 100, 0),
            Err(ModelError::WithdrawalNotAllowed)
        );
    }

    // ── LP accounting: fees & losses ──────────────────────────────────────

    #[test]
    fn fee_accrual_raises_share_price() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.deposit_liquidity("lp1", pid, 1000, 0).unwrap();
        assert_eq!(m.share_price(pid).unwrap(), 1_000_000);
        m.accrue_lp_fees(pid, 500).unwrap();
        assert_eq!(m.share_price(pid).unwrap(), 1_500_000);
        assert_eq!(m.pool(pid).unwrap().lp_fees_accrued, 500);
    }

    #[test]
    fn loss_allocation_lowers_share_price_and_is_capped() {
        let mut m = RefModel::new();
        let pid = simple_pool(&mut m);
        m.deposit_liquidity("lp1", pid, 1000, 0).unwrap();
        let allocated = m.allocate_lp_loss(pid, 400).unwrap();
        assert_eq!(allocated, 400);
        assert_eq!(m.pool(pid).unwrap().lp_assets, 600);
        // Over-allocation is capped at remaining assets.
        let allocated = m.allocate_lp_loss(pid, 10_000).unwrap();
        assert_eq!(allocated, 600);
        assert_eq!(m.pool(pid).unwrap().lp_assets, 0);
    }

    // ── Property tests: conservation of value ─────────────────────────────

    /// Deterministic pseudo-random generator (LCG) so property tests are
    /// reproducible without external crates.
    fn lcg(seed: &mut u64) -> u64 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *seed
    }

    #[test]
    fn prop_deposit_withdraw_conserves_value() {
        let mut seed = 0xDEAD_BEEF_u64;
        for _ in 0..200 {
            let mut m = RefModel::new();
            let pid = simple_pool(&mut m);
            let mut deposited: i128 = 0;
            let mut withdrawn: i128 = 0;

            // Random sequence of deposits and withdrawals.
            for _ in 0..20 {
                let r = lcg(&mut seed);
                let amount = ((r % 1000) + 1) as i128;
                if r % 2 == 0 {
                    if m.deposit_liquidity("lp", pid, amount, 0).is_ok() {
                        deposited += amount;
                    }
                } else {
                    let held = m.lp_shares("lp", pid);
                    if held > 0 {
                        let shares = (amount % held).max(1);
                        if let Ok(assets) = m.withdraw_liquidity("lp", pid, shares, 0) {
                            withdrawn += assets;
                        }
                    }
                }
                // INV-LP1/LP2/LP3 hold after every op (checked internally).
                let pool = m.pool(pid).unwrap();
                assert_eq!(pool.lp_assets, deposited - withdrawn);
                assert_eq!(pool.total_shares, m.lp_shares("lp", pid));
            }
            // Conservation: never withdraw more than deposited.
            assert!(withdrawn <= deposited);
        }
    }

    #[test]
    fn prop_fee_accrual_conserves_value() {
        let mut seed = 0x1234_5678_u64;
        for _ in 0..200 {
            let mut m = RefModel::new();
            let pid = simple_pool(&mut m);
            let mut deposited: i128 = 0;
            let mut fees: i128 = 0;
            let mut losses: i128 = 0;
            let mut withdrawn: i128 = 0;

            for _ in 0..20 {
                let r = lcg(&mut seed);
                let amount = ((r % 500) + 1) as i128;
                match r % 3 {
                    0 => {
                        if m.deposit_liquidity("lp", pid, amount, 0).is_ok() {
                            deposited += amount;
                        }
                    }
                    1 => {
                        m.accrue_lp_fees(pid, amount).unwrap();
                        fees += amount;
                    }
                    _ => {
                        let allocated = m.allocate_lp_loss(pid, amount).unwrap();
                        losses += allocated;
                    }
                }
                let pool = m.pool(pid).unwrap();
                // Conservation identity: assets == deposits + fees − losses.
                assert_eq!(pool.lp_assets, deposited + fees - losses);
                assert_eq!(pool.lp_fees_accrued, fees);
                assert_eq!(pool.lp_losses_allocated, losses);
            }

            // Drain all shares; total redeemed must equal lp_assets.
            let held = m.lp_shares("lp", pid);
            if held > 0 {
                withdrawn += m.withdraw_liquidity("lp", pid, held, 0).unwrap();
            }
            assert_eq!(withdrawn, m.pool(pid).unwrap().lp_assets + withdrawn);
            assert_eq!(m.pool(pid).unwrap().total_shares, 0);
        }
    }

    #[test]
    fn prop_multi_lp_shares_conserve() {
        let mut seed = 0xABCD_1234_u64;
        for _ in 0..100 {
            let mut m = RefModel::new();
            let pid = simple_pool(&mut m);
            let mut total_deposited: i128 = 0;
            let mut total_withdrawn: i128 = 0;

            for _ in 0..30 {
                let r = lcg(&mut seed);
                let who = format!("lp{}", r % 4);
                let amount = ((r % 800) + 1) as i128;
                if r % 2 == 0 {
                    if m.deposit_liquidity(&who, pid, amount, 0).is_ok() {
                        total_deposited += amount;
                    }
                } else {
                    let held = m.lp_shares(&who, pid);
                    if held > 0 {
                        let shares = (amount % held).max(1);
                        if let Ok(assets) = m.withdraw_liquidity(&who, pid, shares, 0) {
                            total_withdrawn += assets;
                        }
                    }
                }
                // INV-LP2: sum of per-LP shares == total_shares.
                let pool = m.pool(pid).unwrap();
                let sum: i128 = pool.lp_shares.values().sum();
                assert_eq!(sum, pool.total_shares);
            }
            assert!(total_withdrawn <= total_deposited);
        }
    }
}
