//! Table-driven regression coverage for the pool state-transition invariant.
//!
//! Pool lifecycle entry points perform their own authorization, time, and storage
//! checks. These tests pin the shared transition rule used by those entry points:
//! an active pool may finalize exactly once, as either resolved or canceled; no
//! terminal state can be reopened or changed to another terminal state.

#![cfg(test)]

use crate::{MarketState, PredifiContract};

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
