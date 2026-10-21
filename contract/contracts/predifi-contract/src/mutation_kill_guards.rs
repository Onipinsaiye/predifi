//! # Mutation-Kill Tests — pool-state and fee guards in `lib.rs`
//!
//! Pure checks of the guard helpers mutated by the `contract-guards` suite
//! (see `mutation-testing/README.md`, issue #128). The role/pause guards need
//! a live `Env` and are killed by the access-control audit and pause tests;
//! these two helpers are pure and get exhaustive tables here.

use crate::{MarketState, PredifiContract};

const ALL_STATES: [MarketState; 4] = [
    MarketState::Active,
    MarketState::Resolved,
    MarketState::Canceled,
    MarketState::Disputed,
];

/// INV-2: the only legal transitions are Active → Resolved and Active → Canceled.
/// Kills `-> true`, `-> false` and any widening/narrowing of the match.
#[test]
fn mutation_kill_state_transition_table() {
    for current in ALL_STATES {
        for next in ALL_STATES {
            let expected = current == MarketState::Active
                && matches!(next, MarketState::Resolved | MarketState::Canceled);
            assert_eq!(
                PredifiContract::is_valid_state_transition(current, next),
                expected,
                "{current:?} -> {next:?}"
            );
        }
    }
}

/// INV-6: fee_bps ≤ 10_000. Kills `-> true`, `-> false` and `<=` → `>` / `<`.
#[test]
fn mutation_kill_fee_bps_boundary() {
    assert!(PredifiContract::is_valid_fee_bps(0));
    assert!(PredifiContract::is_valid_fee_bps(10_000));
    assert!(!PredifiContract::is_valid_fee_bps(10_001));
    assert!(!PredifiContract::is_valid_fee_bps(u32::MAX));
}
