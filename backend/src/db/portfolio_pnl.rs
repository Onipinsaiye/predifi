//! Centralized portfolio profit-and-loss domain model (#310).
//!
//! UI and API must not calculate returns independently — rounding or
//! partial-settlement divergence between two implementations erodes trust
//! fast. This module is the single, typed, DB-free source of truth for that
//! calculation: every function here is pure (no I/O), so it is fully
//! unit-testable without a database and callable from any request handler
//! or background job that needs a position's or a wallet's P&L.
//!
//! ## Fee model
//!
//! The model mirrors how pool payouts and fees are actually computed
//! elsewhere in this codebase:
//!
//! * [`crate::db::pools::calculate_creator_incentive`] skims the protocol
//!   treasury fee off a pool's `total_stake` *before* anything is paid out:
//!   `treasury_fee = floor(total_stake * treasury_fee_bps / 10_000)`.
//! * Winners split what is left (`total_stake - treasury_fee`) in
//!   proportion to their stake's share of the winning outcome's total
//!   stake — the same pro-rata split used by
//!   [`crate::db::predictions::get_users_by_winnings`]
//!   (`amount * total_stake / winning_stake_total`), just computed here
//!   with per-position fee attribution and checked integer arithmetic
//!   instead of floating point.
//!
//! A realized claim (`predictions.claimed_amount`) is chain-derived ground
//! truth and always takes precedence over this estimate: once a wallet has
//! claimed, we report the exact amount paid rather than recomputing it (and
//! risking divergence from what the chain actually paid).

use serde::{Deserialize, Serialize};

/// Version of the P&L calculation semantics. Bump this whenever the formula
/// below changes, so clients can detect when historical figures were
/// computed under a different model and avoid comparing them naively.
pub const PNL_MODEL_VERSION: u32 = 1;

/// Lifecycle state of the pool a position belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolLifecycle {
    Active,
    Closed,
    Settled,
}

impl PoolLifecycle {
    /// Parse the `pools.state` column's string representation.
    pub fn from_db_state(state: &str) -> Self {
        match state {
            "settled" => PoolLifecycle::Settled,
            "closed" => PoolLifecycle::Closed,
            _ => PoolLifecycle::Active,
        }
    }
}

/// Inputs needed to compute one wallet's P&L for one position (one
/// `(pool, outcome)` stake). All amounts are exact integers (the same
/// stroops-equivalent units used throughout this codebase), never floats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionInputs {
    /// Amount this wallet staked on `user_outcome` in this pool.
    pub stake: i64,
    /// The pool's total stake across every outcome.
    pub pool_total_stake: i64,
    /// Total stake on the winning outcome (meaningless until resolved).
    pub winning_outcome_stake: i64,
    /// Outcome index the wallet predicted.
    pub user_outcome: i32,
    /// Winning outcome index, once the pool has resolved.
    pub winning_outcome: Option<i32>,
    /// Current lifecycle state of the pool.
    pub pool_state: PoolLifecycle,
    /// Protocol treasury fee in basis points, skimmed from the pool's total
    /// stake before winners are paid.
    pub treasury_fee_bps: u32,
    /// Whether the wallet has already claimed its payout for this position.
    pub claimed: bool,
    /// The exact amount paid out on claim (0 for a claimed loser/refund-only
    /// claim), only meaningful when `claimed` is `true`. This is ground
    /// truth from the chain.
    pub claimed_amount: i64,
}

/// Why a component amount could not be computed. A typed enum rather than a
/// free-form string so callers can branch on it (e.g. to decide whether to
/// retry later or show a specific message).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingInputReason {
    /// The pool has not resolved yet, so no outcome is known.
    PoolUnresolved,
    /// The pool resolved, but nobody staked on the winning outcome, so the
    /// pro-rata split has no denominator. Reported explicitly rather than
    /// guessed or silently treated as zero.
    ZeroWinningStake,
    /// A computed amount would not fit in the target integer type.
    AmountOverflow,
    /// An input amount was negative, which is never valid for a stake,
    /// pool total, or claimed amount.
    NegativeInput,
}

/// Computed P&L for one position, in exact integer units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionPnl {
    /// Semantic version of the formula that produced this result (see
    /// [`PNL_MODEL_VERSION`]).
    pub pnl_model_version: u32,
    /// Amount staked (always known).
    pub stake: i64,
    /// `true` when `net_return` is a realized on-chain claim rather than an
    /// estimate derived from current pool state.
    pub realized: bool,
    /// Amount payable to this position before the treasury fee is
    /// subtracted, when computable. `None` for a realized claim (the
    /// pre-fee split is not re-derivable from the on-chain payout alone)
    /// and for any position whose return could not be computed.
    pub gross_payout_before_fee: Option<i64>,
    /// Treasury fee attributable to this position's share of the pool,
    /// when computable. Same `None` cases as `gross_payout_before_fee`.
    pub fees: Option<i64>,
    /// Net return paid or payable to the wallet (after fees), when
    /// computable. For a claimed position this is exactly `claimed_amount`.
    pub net_return: Option<i64>,
    /// Realized or estimated profit/loss: `net_return - stake`.
    pub net_pnl: Option<i64>,
    /// Set when any amount above could not be computed. `None` means every
    /// `Option` field above is `Some` (including the "lost, return is
    /// zero" case, which is always computable).
    pub missing_reason: Option<MissingInputReason>,
}

fn missing(stake: i64, reason: MissingInputReason) -> PositionPnl {
    PositionPnl {
        pnl_model_version: PNL_MODEL_VERSION,
        stake,
        realized: false,
        gross_payout_before_fee: None,
        fees: None,
        net_return: None,
        net_pnl: None,
        missing_reason: Some(reason),
    }
}

/// Compute the P&L for a single position.
///
/// Returns an explicit [`MissingInputReason`] (never a panic, never a
/// silently wrapped/guessed value) whenever a component cannot be computed:
/// the pool hasn't resolved, nobody staked on the winning outcome, an input
/// was negative, or a computed amount overflows `i64`.
///
/// A losing position is always computable (`net_return = 0`,
/// `net_pnl = -stake`) — losing is a known outcome, not a missing one.
///
/// A claimed position always reports the realized, chain-derived
/// `claimed_amount` as `net_return`, regardless of what the pool-state
/// estimate would say (claims are ground truth and must never be
/// overridden by a recomputation that could diverge from what the chain
/// actually paid).
pub fn calculate_position_pnl(inputs: &PositionInputs) -> PositionPnl {
    if inputs.stake < 0
        || inputs.pool_total_stake < 0
        || inputs.winning_outcome_stake < 0
        || inputs.claimed_amount < 0
    {
        return missing(inputs.stake, MissingInputReason::NegativeInput);
    }

    if inputs.claimed {
        let net_pnl = (inputs.claimed_amount as i128) - (inputs.stake as i128);
        return match i64::try_from(net_pnl) {
            Ok(net_pnl) => PositionPnl {
                pnl_model_version: PNL_MODEL_VERSION,
                stake: inputs.stake,
                realized: true,
                gross_payout_before_fee: None,
                fees: None,
                net_return: Some(inputs.claimed_amount),
                net_pnl: Some(net_pnl),
                missing_reason: None,
            },
            Err(_) => missing(inputs.stake, MissingInputReason::AmountOverflow),
        };
    }

    if inputs.pool_state != PoolLifecycle::Settled {
        return missing(inputs.stake, MissingInputReason::PoolUnresolved);
    }
    let Some(winning_outcome) = inputs.winning_outcome else {
        return missing(inputs.stake, MissingInputReason::PoolUnresolved);
    };

    // A losing position is always computable: the wallet gets nothing back.
    if inputs.user_outcome != winning_outcome {
        let net_pnl = -(inputs.stake as i128);
        return match i64::try_from(net_pnl) {
            Ok(net_pnl) => PositionPnl {
                pnl_model_version: PNL_MODEL_VERSION,
                stake: inputs.stake,
                realized: false,
                gross_payout_before_fee: Some(0),
                fees: Some(0),
                net_return: Some(0),
                net_pnl: Some(net_pnl),
                missing_reason: None,
            },
            Err(_) => missing(inputs.stake, MissingInputReason::AmountOverflow),
        };
    }

    if inputs.winning_outcome_stake == 0 {
        return missing(inputs.stake, MissingInputReason::ZeroWinningStake);
    }

    let stake = inputs.stake as i128;
    let total = inputs.pool_total_stake as i128;
    let winning = inputs.winning_outcome_stake as i128;
    let fee_bps = inputs.treasury_fee_bps as i128;

    // `i64::MAX * i64::MAX` fits comfortably in `i128`, so these two
    // multiplications of `i64`-derived values can never overflow; the
    // `checked_mul` is defensive against future callers widening the input
    // types, and the real overflow risk (checked below) is the final
    // narrowing back down to `i64`.
    let treasury_fee = match total.checked_mul(fee_bps) {
        Some(v) => v / 10_000,
        None => return missing(inputs.stake, MissingInputReason::AmountOverflow),
    };
    let gross_before_fee = match stake.checked_mul(total) {
        Some(v) => v / winning,
        None => return missing(inputs.stake, MissingInputReason::AmountOverflow),
    };
    let fee_share = match stake.checked_mul(treasury_fee) {
        Some(v) => v / winning,
        None => return missing(inputs.stake, MissingInputReason::AmountOverflow),
    };
    let net_return = gross_before_fee - fee_share;
    let net_pnl = net_return - stake;

    match (
        i64::try_from(gross_before_fee),
        i64::try_from(fee_share),
        i64::try_from(net_return),
        i64::try_from(net_pnl),
    ) {
        (Ok(gross_before_fee), Ok(fee_share), Ok(net_return), Ok(net_pnl)) => PositionPnl {
            pnl_model_version: PNL_MODEL_VERSION,
            stake: inputs.stake,
            realized: false,
            gross_payout_before_fee: Some(gross_before_fee),
            fees: Some(fee_share),
            net_return: Some(net_return),
            net_pnl: Some(net_pnl),
            missing_reason: None,
        },
        _ => missing(inputs.stake, MissingInputReason::AmountOverflow),
    }
}

/// Aggregate P&L across every position in a wallet's portfolio.
///
/// Positions whose individual P&L could not be computed (see
/// [`MissingInputReason`]) are excluded from the totals but counted in
/// `positions_missing` / `missing_reasons`, so a caller can tell "this
/// total is complete" from "part of this wallet's P&L is still unknown"
/// (e.g. because some pools haven't resolved yet — a partial exit/partial
/// settlement).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioPnl {
    pub pnl_model_version: u32,
    /// Sum of every position's stake, computable or not (staking is always
    /// known).
    pub total_stake: i128,
    /// Sum of `net_return` across positions whose P&L was computable.
    pub total_net_return: i128,
    /// Sum of `net_pnl` across positions whose P&L was computable.
    pub total_net_pnl: i128,
    /// Number of positions with a fully computed P&L.
    pub positions_computed: usize,
    /// Number of positions excluded from the totals above because a
    /// component amount was missing.
    pub positions_missing: usize,
    /// One reason per missing position, same order as encountered.
    pub missing_reasons: Vec<MissingInputReason>,
}

/// Compute the aggregate portfolio P&L for a set of positions.
pub fn calculate_portfolio_pnl(positions: &[PositionInputs]) -> PortfolioPnl {
    let mut total_stake: i128 = 0;
    let mut total_net_return: i128 = 0;
    let mut total_net_pnl: i128 = 0;
    let mut positions_computed = 0usize;
    let mut positions_missing = 0usize;
    let mut missing_reasons = Vec::new();

    for position in positions {
        total_stake += position.stake as i128;
        let pnl = calculate_position_pnl(position);
        match pnl.missing_reason {
            None => {
                positions_computed += 1;
                total_net_return += pnl.net_return.unwrap_or(0) as i128;
                total_net_pnl += pnl.net_pnl.unwrap_or(0) as i128;
            }
            Some(reason) => {
                positions_missing += 1;
                missing_reasons.push(reason);
            }
        }
    }

    PortfolioPnl {
        pnl_model_version: PNL_MODEL_VERSION,
        total_stake,
        total_net_return,
        total_net_pnl,
        positions_computed,
        positions_missing,
        missing_reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn winner(
        stake: i64,
        pool_total_stake: i64,
        winning_outcome_stake: i64,
        treasury_fee_bps: u32,
    ) -> PositionInputs {
        PositionInputs {
            stake,
            pool_total_stake,
            winning_outcome_stake,
            user_outcome: 1,
            winning_outcome: Some(1),
            pool_state: PoolLifecycle::Settled,
            treasury_fee_bps,
            claimed: false,
            claimed_amount: 0,
        }
    }

    // ── Golden example ───────────────────────────────────────────────────

    #[test]
    fn golden_example_matches_hand_computed_values() {
        // stake=100 of a 400 winning pool, 1000 total pool, 3% treasury fee.
        let inputs = winner(100, 1_000, 400, 300);
        let pnl = calculate_position_pnl(&inputs);

        assert_eq!(pnl.pnl_model_version, PNL_MODEL_VERSION);
        assert!(!pnl.realized);
        assert_eq!(pnl.missing_reason, None);
        // treasury_fee = floor(1000*300/10000) = 30
        // gross_before_fee = floor(100*1000/400) = 250
        // fee_share = floor(100*30/400) = 7
        assert_eq!(pnl.gross_payout_before_fee, Some(250));
        assert_eq!(pnl.fees, Some(7));
        assert_eq!(pnl.net_return, Some(243));
        assert_eq!(pnl.net_pnl, Some(143));
    }

    #[test]
    fn losing_position_is_always_computable() {
        let inputs = PositionInputs {
            stake: 100,
            pool_total_stake: 1_000,
            winning_outcome_stake: 400,
            user_outcome: 0,
            winning_outcome: Some(1),
            pool_state: PoolLifecycle::Settled,
            treasury_fee_bps: 300,
            claimed: false,
            claimed_amount: 0,
        };
        let pnl = calculate_position_pnl(&inputs);
        assert_eq!(pnl.missing_reason, None);
        assert_eq!(pnl.net_return, Some(0));
        assert_eq!(pnl.net_pnl, Some(-100));
    }

    #[test]
    fn claimed_position_reports_ground_truth_not_estimate() {
        // Even though the "estimate" inputs below would suggest a different
        // payout, a claimed position must report the exact claimed amount.
        let inputs = PositionInputs {
            stake: 100,
            pool_total_stake: 1_000,
            winning_outcome_stake: 400,
            user_outcome: 1,
            winning_outcome: Some(1),
            pool_state: PoolLifecycle::Settled,
            treasury_fee_bps: 300,
            claimed: true,
            claimed_amount: 999,
        };
        let pnl = calculate_position_pnl(&inputs);
        assert!(pnl.realized);
        assert_eq!(pnl.net_return, Some(999));
        assert_eq!(pnl.net_pnl, Some(899));
        assert_eq!(pnl.gross_payout_before_fee, None);
        assert_eq!(pnl.fees, None);
    }

    // ── Missing-input reasons ────────────────────────────────────────────

    #[test]
    fn unresolved_pool_is_explicitly_missing() {
        let inputs = PositionInputs {
            stake: 100,
            pool_total_stake: 1_000,
            winning_outcome_stake: 0,
            user_outcome: 1,
            winning_outcome: None,
            pool_state: PoolLifecycle::Active,
            treasury_fee_bps: 300,
            claimed: false,
            claimed_amount: 0,
        };
        let pnl = calculate_position_pnl(&inputs);
        assert_eq!(pnl.missing_reason, Some(MissingInputReason::PoolUnresolved));
        assert_eq!(pnl.net_return, None);
    }

    #[test]
    fn zero_winning_stake_is_explicitly_missing_not_a_divide_by_zero() {
        let inputs = winner(100, 1_000, 0, 300);
        let pnl = calculate_position_pnl(&inputs);
        assert_eq!(
            pnl.missing_reason,
            Some(MissingInputReason::ZeroWinningStake)
        );
    }

    #[test]
    fn negative_input_is_rejected() {
        let inputs = winner(-1, 1_000, 400, 300);
        let pnl = calculate_position_pnl(&inputs);
        assert_eq!(pnl.missing_reason, Some(MissingInputReason::NegativeInput));
    }

    // ── Extreme integer ranges / overflow ────────────────────────────────

    #[test]
    fn near_i64_max_inputs_do_not_panic_and_stay_in_range() {
        // A single winner holding the whole pool: stake == winning_outcome_stake == pool_total_stake.
        let near_max = i64::MAX / 2;
        let inputs = winner(near_max, near_max, near_max, 300);
        let pnl = calculate_position_pnl(&inputs);
        assert_eq!(pnl.missing_reason, None, "must not overflow for in-range inputs");
        assert!(pnl.net_return.unwrap() <= near_max);
    }

    #[test]
    fn inconsistent_stake_larger_than_winning_total_overflows_explicitly() {
        // Malformed/adversarial input: stake exceeds the winning pool it is
        // supposedly a share of. The projected payout exceeds i64::MAX, so
        // this must surface AmountOverflow, never panic or silently wrap.
        let inputs = winner(i64::MAX, i64::MAX, 1, 0);
        let pnl = calculate_position_pnl(&inputs);
        assert_eq!(pnl.missing_reason, Some(MissingInputReason::AmountOverflow));
    }

    // ── Invariants ───────────────────────────────────────────────────────

    #[test]
    fn components_reconstruct_the_pre_fee_payout() {
        for (stake, total, winning, fee_bps) in [
            (100i64, 1_000i64, 400i64, 300u32),
            (1, 7, 3, 9_999),
            // `stake` must never exceed `winning_outcome_stake` (a
            // position's stake is itself counted within the winning
            // total) — this case keeps that invariant while still
            // exercising large, near-i64 values.
            (499_999, 1_000_000, 500_000, 1),
            (50, 50, 50, 0),
        ] {
            let inputs = winner(stake, total, winning, fee_bps);
            let pnl = calculate_position_pnl(&inputs);
            assert_eq!(pnl.missing_reason, None, "case ({stake},{total},{winning},{fee_bps})");
            let gross = pnl.gross_payout_before_fee.unwrap();
            let fees = pnl.fees.unwrap();
            let net = pnl.net_return.unwrap();
            assert_eq!(gross, net + fees, "gross must equal net + fees");
            assert!(gross <= total, "a position's pre-fee payout must never exceed the pool total");
            assert!(net <= gross, "net return must never exceed the pre-fee payout");
        }
    }

    #[test]
    fn portfolio_aggregate_sums_only_computable_positions() {
        let computed = winner(100, 1_000, 400, 300);
        let unresolved = PositionInputs {
            stake: 50,
            pool_total_stake: 0,
            winning_outcome_stake: 0,
            user_outcome: 0,
            winning_outcome: None,
            pool_state: PoolLifecycle::Active,
            treasury_fee_bps: 300,
            claimed: false,
            claimed_amount: 0,
        };

        let portfolio = calculate_portfolio_pnl(&[computed, unresolved]);
        assert_eq!(portfolio.pnl_model_version, PNL_MODEL_VERSION);
        assert_eq!(portfolio.total_stake, 150);
        assert_eq!(portfolio.positions_computed, 1);
        assert_eq!(portfolio.positions_missing, 1);
        assert_eq!(
            portfolio.missing_reasons,
            vec![MissingInputReason::PoolUnresolved]
        );
        // Only the computed position contributes to net_return/net_pnl.
        assert_eq!(portfolio.total_net_return, 243);
        assert_eq!(portfolio.total_net_pnl, 143);
    }

    /// Partial exit: one claimed (realized) position and one still-open
    /// position in the same portfolio. The aggregate must report the
    /// realized leg accurately while flagging the open leg as missing,
    /// rather than blocking or guessing at its value.
    #[test]
    fn partial_settlement_mixes_realized_and_open_positions() {
        let claimed = PositionInputs {
            stake: 200,
            pool_total_stake: 0,
            winning_outcome_stake: 0,
            user_outcome: 1,
            winning_outcome: Some(1),
            pool_state: PoolLifecycle::Settled,
            treasury_fee_bps: 300,
            claimed: true,
            claimed_amount: 350,
        };
        let open = PositionInputs {
            stake: 80,
            pool_total_stake: 500,
            winning_outcome_stake: 0,
            user_outcome: 1,
            winning_outcome: None,
            pool_state: PoolLifecycle::Active,
            treasury_fee_bps: 300,
            claimed: false,
            claimed_amount: 0,
        };

        let portfolio = calculate_portfolio_pnl(&[claimed, open]);
        assert_eq!(portfolio.total_stake, 280);
        assert_eq!(portfolio.positions_computed, 1);
        assert_eq!(portfolio.positions_missing, 1);
        assert_eq!(portfolio.total_net_return, 350);
        assert_eq!(portfolio.total_net_pnl, 150);
    }
}
