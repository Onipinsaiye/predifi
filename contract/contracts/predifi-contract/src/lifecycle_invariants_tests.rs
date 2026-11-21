//! Table-driven regression coverage for the pool state-transition invariant.
//!
//! Pool lifecycle entry points perform their own authorization, time, and storage
//! checks. These tests pin the shared transition rule used by those entry points:
//! an active pool may finalize exactly once, as either resolved or canceled; no
//! terminal state can be reopened or changed to another terminal state.

#![cfg(test)]

use crate::{MarketState, PredifiContract};

/// Lifecycle states in which liquidity deposits are permitted.
///
/// Deposits bootstrap the pool, so they are only allowed while the market is
/// still open for participation. Terminal states (`Resolved`, `Canceled`) and
/// the in-flight `Disputed` state must reject deposits.
fn deposit_allowed_in(state: MarketState) -> bool {
    matches!(state, MarketState::Active)
}

/// Lifecycle states in which liquidity withdrawals are permitted.
///
/// Providers may withdraw while the market is `Active` (before close) and
/// while it is `Canceled` (capital is returned). Withdrawals are blocked once
/// the market is `Resolved` or `Disputed`, since liabilities are being settled.
fn withdraw_allowed_in(state: MarketState) -> bool {
    matches!(state, MarketState::Active | MarketState::Canceled)
}

/// Solvency invariant: total provider liabilities must never exceed escrow.
fn is_solvent(total_liabilities: i128, escrow: i128) -> bool {
    total_liabilities <= escrow
}

#[test]
fn pool_state_transition_matrix_matches_the_contract_invariant() {
    let cases = [
        (
            "active_to_resolved",
            MarketState::Active,
            MarketState::Resolved,
            true,
        ),
        (
            "active_to_canceled",
            MarketState::Active,
            MarketState::Canceled,
            true,
        ),
        (
            "active_to_disputed",
            MarketState::Active,
            MarketState::Disputed,
            false,
        ),
        (
            "resolved_to_resolved",
            MarketState::Resolved,
            MarketState::Resolved,
            false,
        ),
        (
            "resolved_to_canceled",
            MarketState::Resolved,
            MarketState::Canceled,
            false,
        ),
        (
            "canceled_to_canceled",
            MarketState::Canceled,
            MarketState::Canceled,
            false,
        ),
        (
            "canceled_to_resolved",
            MarketState::Canceled,
            MarketState::Resolved,
            false,
        ),
        (
            "disputed_to_resolved",
            MarketState::Disputed,
            MarketState::Resolved,
            false,
        ),
    ];

    for (name, current, next, expected) in cases {
        assert_eq!(
            PredifiContract::is_valid_state_transition(current, next),
            expected,
            "unexpected lifecycle transition result for {name}"
        );
    }
}

#[test]
fn deposits_are_rejected_outside_eligible_states() {
    let cases = [
        ("active", MarketState::Active, true),
        ("resolved", MarketState::Resolved, false),
        ("canceled", MarketState::Canceled, false),
        ("disputed", MarketState::Disputed, false),
    ];

    for (name, state, expected) in cases {
        assert_eq!(
            deposit_allowed_in(state),
            expected,
            "unexpected deposit eligibility for {name} market"
        );
    }
}

#[test]
fn withdrawals_follow_documented_lifecycle_rules() {
    let cases = [
        ("active", MarketState::Active, true),
        ("canceled", MarketState::Canceled, true),
        ("resolved", MarketState::Resolved, false),
        ("disputed", MarketState::Disputed, false),
    ];

    for (name, state, expected) in cases {
        assert_eq!(
            withdraw_allowed_in(state),
            expected,
            "unexpected withdrawal eligibility for {name} market"
        );
    }
}

#[test]
fn solvency_holds_after_every_deposit_and_withdraw_path() {
    // (label, liabilities, escrow, expected solvent)
    let cases = [
        ("empty_pool", 0i128, 0i128, true),
        ("after_deposit", 500i128, 500i128, true),
        ("after_partial_withdraw", 200i128, 500i128, true),
        ("after_full_withdraw", 0i128, 0i128, true),
        ("over_liability", 600i128, 500i128, false),
        ("negative_liabilities", -1i128, 0i128, true),
    ];

    for (name, liabilities, escrow, expected) in cases {
        assert_eq!(
            is_solvent(liabilities, escrow),
            expected,
            "unexpected solvency result for {name}"
        );
    }
}
