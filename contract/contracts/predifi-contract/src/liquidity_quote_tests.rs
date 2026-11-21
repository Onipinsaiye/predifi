//! Unit tests for the pure liquidity quote engine (issue #274).
//!
//! These tests deliberately exercise the quote math without a Soroban host so
//! that the same engine can be verified on native builds and reused by the
//! on-chain entry points.

use crate::quote::{
    compute_quote, compute_withdraw_quote, LiquidityQuote, QuoteError, QuotePoolState,
    MAX_QUOTE_STALENESS_SECONDS, MAX_SLIPPAGE_BPS,
};

fn state(reserve: i128, total_shares: i128) -> QuotePoolState {
    QuotePoolState {
        reserve,
        total_shares,
        total_stake: reserve,
        fee_bps: 100,
        updated_at: 10_000,
    }
}

#[test]
fn bootstrap_quote_mints_one_to_one() {
    let q = compute_quote(&state(0, 0), 5_000).unwrap();
    assert_eq!(q.amount_out, 5_000);
    assert_eq!(q.timestamp, 10_000);
}

#[test]
fn proportional_deposit_quote_rounds_down() {
    // reserve = 1_000, shares = 3 → deposit 10 mints floor(10*3/1000) = 0
    assert_eq!(
        compute_quote(&state(1_000, 3), 10),
        Err(QuoteError::InvalidAmount)
    );
    let q = compute_quote(&state(1_000, 3), 1_000).unwrap();
    assert_eq!(q.amount_out, 3);
}

#[test]
fn withdrawal_quote_rounds_down_and_never_exceeds_reserve() {
    let q = compute_withdraw_quote(&state(1_000, 3), 2).unwrap();
    assert_eq!(q.amount_out, 666);
    assert!(q.amount_out <= 1_000);
}

#[test]
fn quote_rejects_invalid_amounts_and_states() {
    assert_eq!(compute_quote(&state(1_000, 1_000), 0), Err(QuoteError::InvalidAmount));
    assert_eq!(compute_quote(&state(1_000, 1_000), -1), Err(QuoteError::InvalidAmount));
    assert_eq!(
        compute_quote(&state(-1, 1_000), 1),
        Err(QuoteError::UnsupportedState)
    );
    assert_eq!(
        compute_withdraw_quote(&state(1_000, 0), 1),
        Err(QuoteError::UnsupportedState)
    );
}

#[test]
fn price_impact_is_bounded_by_max_slippage() {
    let q = compute_quote(&state(1, 1), 1_000_000_000).unwrap();
    assert!(q.price_impact_bps <= MAX_SLIPPAGE_BPS);
    let q = compute_withdraw_quote(&state(1, 1_000_000), 999_999).unwrap();
    assert!(q.price_impact_bps <= MAX_SLIPPAGE_BPS);
}

#[test]
fn quote_staleness_window_is_positive() {
    assert!(MAX_QUOTE_STALENESS_SECONDS > 0);
}

#[test]
fn quote_carries_fee_and_timestamp() {
    let q: LiquidityQuote = compute_quote(&state(10_000, 10_000), 1_000).unwrap();
    assert_eq!(q.fee, 10);
    assert_eq!(q.timestamp, 10_000);
}
