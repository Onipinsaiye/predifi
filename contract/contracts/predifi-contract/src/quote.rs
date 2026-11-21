//! # Liquidity Quote Module (issue #274)
//!
//! Provides purely functional quote computation for liquidity deposit and
//! withdrawal operations.  The quote engine is separated from the state-
//! mutating entry points so that:
//!
//! 1. Off-chain callers can estimate shares/assets without paying gas for a
//!    full deposit or withdrawal.
//! 2. The slippage enforcement path in `deposit_liquidity` / `withdraw_liquidity`
//!    can compare the executed price against a quote the user signed pre-flight.
//! 3. The logic is unit-testable without a full Soroban environment.
//!
//! ## Quote model
//!
//! The share price is derived from the pool's current reserve and outstanding
//! shares:
//!
//! ```text
//! share_price = reserve / total_shares   (when total_shares > 0)
//!
//! Deposit:   shares_out  = floor(amount_in  * total_shares / reserve)
//! Withdraw:  assets_out  = floor(shares_in  * reserve / total_shares)
//!
//! price_impact_bps = shares_out * 10_000 / (total_shares + shares_out)
//! ```
//!
//! Both operations round **down** (integer floor), keeping the rounding
//! remainder inside the pool.  `price_impact_bps` measures how much the
//! provider's position changes the pool's outstanding share count in
//! percentage terms (basis points).
//!
//! ## Slippage enforcement (issue #274)
//!
//! Callers supply a `max_slippage_bps` tolerance.  The contract rejects the
//! operation if the computed `price_impact_bps > max_slippage_bps`, preventing
//! the user from executing against a stale or heavily moved quote.

/// Maximum age (in seconds) of a pool's accounting snapshot before quotes
/// derived from it are considered stale.
///
/// **Value:** 300 seconds (5 minutes).
///
/// **Rationale:** LP deposits and withdrawals change the pool's total_assets
/// and total_shares.  A quote computed against a snapshot that is more than
/// 5 minutes old may no longer reflect the current share price, exposing the
/// user to unexpected slippage.  Five minutes is a generous bound: Stellar
/// ledgers close in ~5 seconds, so a genuinely stale snapshot implies
/// sustained inactivity, not ordinary network jitter.
pub const MAX_QUOTE_STALENESS_SECONDS: u64 = 300;

/// Maximum allowed slippage in basis points (100% = 10,000 bps).
///
/// A caller-supplied `max_slippage_bps` above this value is normalised to
/// this cap rather than rejected, so the constant acts as an absolute upper
/// bound on the tolerance parameter.
pub const MAX_SLIPPAGE_BPS: u32 = 10_000;

/// Pool state snapshot required to compute a liquidity quote.
///
/// This is a pure data transfer object: it carries no Soroban types so
/// the quote engine can be unit-tested outside the contract environment.
#[derive(Clone, Debug, PartialEq)]
pub struct QuotePoolState {
    /// Total token assets owned by LPs (reserve backing outstanding shares).
    pub reserve: i128,
    /// Total outstanding LP shares.
    pub total_shares: i128,
    /// Total stake (all users + initial liquidity) for price-impact context.
    pub total_stake: i128,
    /// Protocol fee in basis points applied to this pool at resolution.
    pub fee_bps: u32,
    /// Ledger timestamp at which this snapshot was last updated.
    pub updated_at: u64,
}

/// A fully computed liquidity quote.
#[derive(Clone, Debug, PartialEq)]
pub struct LiquidityQuote {
    /// Input amount (assets deposited or shares burned).
    pub amount_in: i128,
    /// Estimated output (shares minted or assets returned).
    pub amount_out: i128,
    /// Estimated price impact in basis points.
    pub price_impact_bps: u32,
    /// Protocol fee withheld in base token units (informational; the actual
    /// fee is deducted at resolution, not at deposit time).
    pub fee: i128,
    /// Ledger timestamp embedded in the state snapshot used to derive this
    /// quote (equals `QuotePoolState::updated_at`).
    pub timestamp: u64,
}

/// Errors returned by the pure quote computation path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteError {
    /// `amount_in` is zero or negative.
    InvalidAmount,
    /// The pool state cannot produce a meaningful quote (e.g. zero shares
    /// while `reserve > 0` indicates an accounting inconsistency).
    UnsupportedState,
    /// The pool snapshot is stale (the caller should check `updated_at`
    /// before invoking this function).
    StaleState,
    /// An intermediate multiplication overflowed `i128`.
    Overflow,
}

/// Compute a deposit quote: how many LP shares would `amount_in` assets mint?
///
/// Returns a [`LiquidityQuote`] with:
/// - `amount_out` = shares to be minted (floored)
/// - `price_impact_bps` = change in share supply as a fraction of the new
///   total (always non-negative, capped at `MAX_SLIPPAGE_BPS`)
/// - `fee` = protocol fee estimate in base token units (informational)
///
/// # Errors
/// - [`QuoteError::InvalidAmount`] when `amount_in <= 0` or shares would be zero.
/// - [`QuoteError::UnsupportedState`] when `state.total_shares < 0` or the
///   share arithmetic would divide by zero.
/// - [`QuoteError::Overflow`] when any intermediate product overflows `i128`.
pub fn compute_quote(
    state: &QuotePoolState,
    amount_in: i128,
) -> Result<LiquidityQuote, QuoteError> {
    if amount_in <= 0 {
        return Err(QuoteError::InvalidAmount);
    }
    if state.total_shares < 0 || state.reserve < 0 {
        return Err(QuoteError::UnsupportedState);
    }

    // Shares minted: 1:1 on first deposit; proportional thereafter.
    let shares_out = if state.total_shares == 0 || state.reserve == 0 {
        // Bootstrap: first deposit mints shares 1:1 with the deposit amount.
        amount_in
    } else {
        // Subsequent deposits: floor(amount_in * total_shares / reserve)
        amount_in
            .checked_mul(state.total_shares)
            .ok_or(QuoteError::Overflow)?
            / state.reserve
    };

    if shares_out <= 0 {
        return Err(QuoteError::InvalidAmount);
    }

    // Price impact: how much does this deposit move the total share supply?
    // impact_bps = shares_out * 10_000 / (total_shares + shares_out)
    let new_total_shares = state
        .total_shares
        .checked_add(shares_out)
        .ok_or(QuoteError::Overflow)?;
    let price_impact_bps: u32 = if new_total_shares == 0 {
        0
    } else {
        let impact_num = shares_out
            .checked_mul(10_000)
            .ok_or(QuoteError::Overflow)?;
        let impact = impact_num / new_total_shares;
        if impact > MAX_SLIPPAGE_BPS as i128 {
            MAX_SLIPPAGE_BPS
        } else {
            impact as u32
        }
    };

    // Fee estimate (informational only; deducted at resolution).
    let fee = amount_in
        .checked_mul(state.fee_bps as i128)
        .ok_or(QuoteError::Overflow)?
        / 10_000;

    Ok(LiquidityQuote {
        amount_in,
        amount_out: shares_out,
        price_impact_bps,
        fee,
        timestamp: state.updated_at,
    })
}

/// Compute a withdrawal quote: how many assets would burning `shares_in` return?
///
/// Returns a [`LiquidityQuote`] with:
/// - `amount_out` = assets returned (floored)
/// - `price_impact_bps` = reduction in share supply relative to old total
/// - `fee` = protocol fee estimate (informational)
///
/// # Errors
/// - [`QuoteError::InvalidAmount`] when `shares_in <= 0` or assets would be zero.
/// - [`QuoteError::UnsupportedState`] when `state.total_shares <= 0`
///   (cannot withdraw from a pool with no shares).
/// - [`QuoteError::Overflow`] when any intermediate product overflows `i128`.
pub fn compute_withdraw_quote(
    state: &QuotePoolState,
    shares_in: i128,
) -> Result<LiquidityQuote, QuoteError> {
    if shares_in <= 0 {
        return Err(QuoteError::InvalidAmount);
    }
    if state.total_shares <= 0 || state.reserve < 0 {
        return Err(QuoteError::UnsupportedState);
    }

    // assets_out = floor(shares_in * reserve / total_shares)
    let assets_out = shares_in
        .checked_mul(state.reserve)
        .ok_or(QuoteError::Overflow)?
        / state.total_shares;

    if assets_out <= 0 {
        return Err(QuoteError::InvalidAmount);
    }

    // Price impact: share supply drops by `shares_in` / old total.
    let price_impact_bps: u32 = {
        let impact_num = shares_in
            .checked_mul(10_000)
            .ok_or(QuoteError::Overflow)?;
        let impact = impact_num / state.total_shares;
        if impact > MAX_SLIPPAGE_BPS as i128 {
            MAX_SLIPPAGE_BPS
        } else {
            impact as u32
        }
    };

    let fee = assets_out
        .checked_mul(state.fee_bps as i128)
        .ok_or(QuoteError::Overflow)?
        / 10_000;

    Ok(LiquidityQuote {
        amount_in: shares_in,
        amount_out: assets_out,
        price_impact_bps,
        fee,
        timestamp: state.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(reserve: i128, total_shares: i128) -> QuotePoolState {
        QuotePoolState {
            reserve,
            total_shares,
            total_stake: reserve,
            fee_bps: 100,
            updated_at: 1_000_000,
        }
    }

    #[test]
    fn test_bootstrap_deposit_mints_one_to_one() {
        let state = make_state(0, 0);
        let q = compute_quote(&state, 1_000_000).unwrap();
        assert_eq!(q.amount_out, 1_000_000);
    }

    #[test]
    fn test_proportional_deposit() {
        // 1_000 reserve, 1_000 shares → deposit 500 → mint 500 shares
        let state = make_state(1_000, 1_000);
        let q = compute_quote(&state, 500).unwrap();
        assert_eq!(q.amount_out, 500);
    }

    #[test]
    fn test_deposit_zero_returns_error() {
        let state = make_state(1_000, 1_000);
        assert_eq!(compute_quote(&state, 0), Err(QuoteError::InvalidAmount));
    }

    #[test]
    fn test_withdraw_proportional() {
        // 1_000 reserve, 1_000 shares → burn 250 → get 250 assets
        let state = make_state(1_000, 1_000);
        let q = compute_withdraw_quote(&state, 250).unwrap();
        assert_eq!(q.amount_out, 250);
    }

    #[test]
    fn test_withdraw_zero_shares_returns_error() {
        let state = make_state(1_000, 1_000);
        assert_eq!(
            compute_withdraw_quote(&state, 0),
            Err(QuoteError::InvalidAmount)
        );
    }

    #[test]
    fn test_withdraw_no_shares_returns_unsupported_state() {
        let state = make_state(1_000, 0);
        assert_eq!(
            compute_withdraw_quote(&state, 100),
            Err(QuoteError::UnsupportedState)
        );
    }

    #[test]
    fn test_price_impact_capped_at_max_slippage() {
        // Large deposit into tiny pool should cap impact at MAX_SLIPPAGE_BPS.
        let state = make_state(1, 1);
        let q = compute_quote(&state, 1_000_000_000).unwrap();
        assert!(q.price_impact_bps <= MAX_SLIPPAGE_BPS);
    }

    #[test]
    fn test_fee_estimate_is_informational() {
        // fee_bps = 100 bps = 1% → fee = 1_000 * 100 / 10_000 = 10
        let state = make_state(10_000, 10_000);
        let q = compute_quote(&state, 1_000).unwrap();
        assert_eq!(q.fee, 10);
    }

    #[test]
    fn test_withdraw_fee_estimate() {
        // assets_out = 500; fee = 500 * 100 / 10_000 = 5
        let state = make_state(10_000, 10_000);
        let q = compute_withdraw_quote(&state, 500).unwrap();
        assert_eq!(q.fee, 5);
    }

    #[test]
    fn test_deposit_negative_amount_returns_error() {
        let state = make_state(1_000, 1_000);
        assert_eq!(compute_quote(&state, -1), Err(QuoteError::InvalidAmount));
    }

    #[test]
    fn test_withdraw_negative_shares_returns_error() {
        let state = make_state(1_000, 1_000);
        assert_eq!(
            compute_withdraw_quote(&state, -1),
            Err(QuoteError::InvalidAmount)
        );
    }

    #[test]
    fn test_unsupported_state_negative_reserve() {
        let state = QuotePoolState {
            reserve: -1,
            total_shares: 100,
            total_stake: 100,
            fee_bps: 100,
            updated_at: 0,
        };
        assert_eq!(compute_quote(&state, 10), Err(QuoteError::UnsupportedState));
    }

    #[test]
    fn test_deposit_price_impact_increases_with_deposit_size() {
        let state = make_state(1_000_000, 1_000_000);
        let small = compute_quote(&state, 1_000).unwrap();
        let large = compute_quote(&state, 500_000).unwrap();
        assert!(large.price_impact_bps >= small.price_impact_bps);
    }

    #[test]
    fn test_withdraw_price_impact_increases_with_burn_size() {
        let state = make_state(1_000_000, 1_000_000);
        let small = compute_withdraw_quote(&state, 1_000).unwrap();
        let large = compute_withdraw_quote(&state, 500_000).unwrap();
        assert!(large.price_impact_bps >= small.price_impact_bps);
    }
}
