//! # Payout Arithmetic
//!
//! Pure integer arithmetic that mirrors `safe_math.rs` and `payouts.rs` in the
//! Soroban contract. Every function documents its rounding direction so that
//! differential tests can validate it exhaustively.
//!
//! The key invariant preserved by these routines is that no combination of
//! claimants can receive more tokens in total than `pool.total_stake`.
//! This holds because:
//!   - `protocol_fee` is floored (≤ exact value), so `payout_pool` is slightly
//!     generous, but every `winnings` calculation floors again.
//!   - For any partition of `winning_stake` into user stakes
//!     `u₁ + u₂ + … + uₙ = winning_stake`, the sum of floored shares satisfies
//!     `Σ floor(uᵢ * payout_pool / winning_stake) ≤ payout_pool ≤ total_stake`.

/// Maximum basis points (100%).
pub const MAX_BPS: i128 = 10_000;

/// Sentinel value for an unresolved pool outcome (mirrors `UNRESOLVED_OUTCOME`).
pub const UNRESOLVED_OUTCOME: u32 = u32::MAX;

/// Calculate protocol fee from total stake and fee rate.
///
/// # Rounding: floor (matches `RoundingMode::ProtocolFavor`)
/// `result = total_stake * fee_bps / 10_000`
///
/// # Errors
/// Returns `Err(MathError::InvalidFeeBps)` when `fee_bps > 10_000`.
/// Returns `Err(MathError::NegativeInput)` when `total_stake < 0`.
/// Returns `Err(MathError::Overflow)` on i128 overflow (extremely large inputs).
pub fn protocol_fee(total_stake: i128, fee_bps: i128) -> Result<i128, MathError> {
    if total_stake < 0 {
        return Err(MathError::NegativeInput);
    }
    if fee_bps < 0 || fee_bps > MAX_BPS {
        return Err(MathError::InvalidFeeBps);
    }
    if total_stake == 0 || fee_bps == 0 {
        return Ok(0);
    }
    let numerator = total_stake
        .checked_mul(fee_bps)
        .ok_or(MathError::Overflow)?;
    Ok(numerator / MAX_BPS) // floor division
}

/// Amount remaining for winners after protocol fee is deducted.
///
/// # Rounding: exact subtraction (no new rounding)
pub fn payout_pool(total_stake: i128, fee_bps: i128) -> Result<i128, MathError> {
    let fee = protocol_fee(total_stake, fee_bps)?;
    total_stake
        .checked_sub(fee)
        .ok_or(MathError::Overflow)
}

/// Proportional winner payout.
///
/// Returns `0` when `winning_stake == 0` or `user_stake == 0`.
///
/// # Rounding: floor (matches `RoundingMode::ProtocolFavor`)
/// `result = user_stake * pool / winning_stake`
///
/// # Errors
/// Returns `Err(MathError::NegativeInput)` on negative inputs.
/// Returns `Err(MathError::Overflow)` on i128 overflow.
pub fn winner_share(user_stake: i128, winning_stake: i128, pool: i128) -> Result<i128, MathError> {
    if user_stake < 0 || winning_stake < 0 || pool < 0 {
        return Err(MathError::NegativeInput);
    }
    if winning_stake == 0 || user_stake == 0 {
        return Ok(0);
    }
    let numerator = user_stake
        .checked_mul(pool)
        .ok_or(MathError::Overflow)?;
    Ok(numerator / winning_stake) // floor division
}

/// Full payout breakdown for one claim: fee → payout pool → user share.
///
/// This is the reference implementation of `calculate_claim_payout` in the contract.
pub fn claim_payout(
    total_stake: i128,
    fee_bps: i128,
    user_stake: i128,
    winning_stake: i128,
) -> Result<PayoutBreakdown, MathError> {
    if total_stake < 0 || user_stake < 0 || winning_stake < 0 {
        return Err(MathError::NegativeInput);
    }
    if fee_bps < 0 || fee_bps > MAX_BPS {
        return Err(MathError::InvalidFeeBps);
    }

    let fee = protocol_fee(total_stake, fee_bps)?;
    let pool = total_stake
        .checked_sub(fee)
        .ok_or(MathError::Overflow)?;
    let winnings = winner_share(user_stake, winning_stake, pool)?;

    // INV-4: defense in depth
    if winnings > total_stake {
        return Err(MathError::InvariantViolated("INV-4: winnings > total_stake"));
    }

    Ok(PayoutBreakdown {
        protocol_fee: fee,
        payout_pool: pool,
        winnings,
    })
}

/// Referral cut from the protocol fee attributable to a specific claimer.
///
/// # Rounding: nearest, ties round up (matches `RoundingMode::Neutral`)
/// Intentionally more generous than floor because it comes from the treasury
/// share, not from user payouts.
///
/// `result = round_half_up(user_stake / total_stake * protocol_fee * referral_cut_bps / 10_000)`
pub fn referral_amount(
    user_stake: i128,
    total_stake: i128,
    protocol_fee_total: i128,
    referral_cut_bps: i128,
) -> Result<i128, MathError> {
    if protocol_fee_total <= 0 || total_stake <= 0 || user_stake <= 0 {
        return Ok(0);
    }
    // user's proportional share of the protocol fee — round nearest
    let user_fee_num = user_stake
        .checked_mul(protocol_fee_total)
        .ok_or(MathError::Overflow)?;
    let user_fee_share = round_nearest(user_fee_num, total_stake)?;

    // referral cut of that share — round nearest
    let referral_num = user_fee_share
        .checked_mul(referral_cut_bps)
        .ok_or(MathError::Overflow)?;
    round_nearest(referral_num, MAX_BPS)
}

/// Round `numerator / denominator` to nearest integer, ties round up.
fn round_nearest(numerator: i128, denominator: i128) -> Result<i128, MathError> {
    if denominator == 0 {
        return Err(MathError::DivisionByZero);
    }
    // (num + denom/2) / denom  — but we need to avoid overflow from the addition.
    // Use: num/denom + (num%denom*2 >= denom ? 1 : 0)
    let q = numerator / denominator;
    let r = numerator % denominator;
    if r * 2 >= denominator {
        Ok(q + 1)
    } else {
        Ok(q)
    }
}

/// Select the dynamic fee rate for a pool at resolution time.
///
/// Walks `tiers` and picks the highest `stake_threshold ≤ total_stake`.
/// Falls back to `base_fee_bps` if no tier matches.
///
/// This exactly mirrors `calculate_dynamic_fee` in the contract.
pub fn dynamic_fee(total_stake: i128, base_fee_bps: u32, tiers: &[FeeTier]) -> u32 {
    let mut applied = base_fee_bps;
    let mut max_threshold: i128 = -1;
    for tier in tiers {
        if total_stake >= tier.stake_threshold && tier.stake_threshold > max_threshold {
            max_threshold = tier.stake_threshold;
            applied = tier.fee_bps;
        }
    }
    applied
}

// ─────────────────────────────────────────────────────────────────────────────
// Supporting types
// ─────────────────────────────────────────────────────────────────────────────

/// Full payout breakdown returned by [`claim_payout`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PayoutBreakdown {
    pub protocol_fee: i128,
    pub payout_pool: i128,
    pub winnings: i128,
}

/// Arithmetic error variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathError {
    Overflow,
    NegativeInput,
    InvalidFeeBps,
    DivisionByZero,
    InvariantViolated(&'static str),
}

/// A single dynamic fee tier (mirrors `FeeTier` in the contract).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeeTier {
    /// Minimum `total_stake` for this tier to activate.
    pub stake_threshold: i128,
    /// Protocol fee in basis points when this tier is active.
    pub fee_bps: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Liquidity-provider accounting (RFC #268)
// ─────────────────────────────────────────────────────────────────────────────
//
// Share model: an LP pool holds `total_assets` (tokens) and `total_shares`.
// A deposit of `amount` mints `floor(amount * total_shares / total_assets)`
// shares (or `amount` shares when the pool is empty). A withdrawal of `shares`
// returns `floor(shares * total_assets / total_shares)` tokens.
//
// Conservation invariants (see RFC #268):
//   LP-1  Σ minted shares == total_shares
//   LP-2  Σ withdrawn tokens ≤ total_assets
//   LP-3  total_assets == deposits - withdrawals + fees_accrued - losses
//   LP-4  every rounding step floors, so the pool never pays out more than it
//         holds (dust accrues to remaining LPs, never to the protocol).

/// Shares minted for a deposit into an LP pool.
///
/// # Rounding: floor (favors the pool / existing LPs)
/// `shares = amount * total_shares / total_assets`
///
/// When the pool is empty (`total_assets == 0` or `total_shares == 0`) the
/// deposit is 1:1 and `shares == amount`.
///
/// # Errors
/// Returns `Err(MathError::NegativeInput)` on negative inputs.
/// Returns `Err(MathError::Overflow)` on i128 overflow.
pub fn lp_shares_for_deposit(
    amount: i128,
    total_assets: i128,
    total_shares: i128,
) -> Result<i128, MathError> {
    if amount < 0 || total_assets < 0 || total_shares < 0 {
        return Err(MathError::NegativeInput);
    }
    if amount == 0 {
        return Ok(0);
    }
    if total_assets == 0 || total_shares == 0 {
        return Ok(amount); // bootstrap: 1:1
    }
    let numerator = amount.checked_mul(total_shares).ok_or(MathError::Overflow)?;
    Ok(numerator / total_assets) // floor division
}

/// Tokens returned for burning `shares` from an LP pool.
///
/// # Rounding: floor (favors the pool / remaining LPs)
/// `assets = shares * total_assets / total_shares`
///
/// # Errors
/// Returns `Err(MathError::NegativeInput)` on negative inputs.
/// Returns `Err(MathError::DivisionByZero)` when `total_shares == 0` and
/// `shares > 0`.
/// Returns `Err(MathError::Overflow)` on i128 overflow.
pub fn lp_assets_for_withdrawal(
    shares: i128,
    total_assets: i128,
    total_shares: i128,
) -> Result<i128, MathError> {
    if shares < 0 || total_assets < 0 || total_shares < 0 {
        return Err(MathError::NegativeInput);
    }
    if shares == 0 {
        return Ok(0);
    }
    if total_shares == 0 {
        return Err(MathError::DivisionByZero);
    }
    let numerator = shares.checked_mul(total_assets).ok_or(MathError::Overflow)?;
    Ok(numerator / total_shares) // floor division
}

/// Accrue a fee into the LP pool, increasing assets without minting shares.
///
/// # Rounding: exact (no new rounding)
/// `new_total_assets = total_assets + fee`
///
/// # Errors
/// Returns `Err(MathError::NegativeInput)` on negative inputs.
/// Returns `Err(MathError::Overflow)` on i128 overflow.
pub fn lp_accrue_fee(total_assets: i128, fee: i128) -> Result<i128, MathError> {
    if total_assets < 0 || fee < 0 {
        return Err(MathError::NegativeInput);
    }
    total_assets.checked_add(fee).ok_or(MathError::Overflow)
}

/// Allocate a loss across the LP pool by reducing `total_assets`.
///
/// Losses are socialised pro-rata via the share price: `total_shares` is
/// unchanged, so each share is worth less. A loss larger than the pool is
/// clamped to the pool balance (LPs cannot lose more than they deposited).
///
/// # Rounding: exact (no new rounding)
/// `new_total_assets = max(total_assets - loss, 0)`
///
/// # Errors
/// Returns `Err(MathError::NegativeInput)` on negative inputs.
pub fn lp_allocate_loss(total_assets: i128, loss: i128) -> Result<i128, MathError> {
    if total_assets < 0 || loss < 0 {
        return Err(MathError::NegativeInput);
    }
    Ok((total_assets - loss).max(0))
}

/// Share price in basis points: assets per share scaled by `MAX_BPS`.
///
/// # Rounding: floor
/// `price_bps = total_assets * 10_000 / total_shares`
///
/// Returns `MAX_BPS` (1.0) for an empty pool.
///
/// # Errors
/// Returns `Err(MathError::NegativeInput)` on negative inputs.
/// Returns `Err(MathError::Overflow)` on i128 overflow.
pub fn lp_share_price_bps(total_assets: i128, total_shares: i128) -> Result<i128, MathError> {
    if total_assets < 0 || total_shares < 0 {
        return Err(MathError::NegativeInput);
    }
    if total_shares == 0 {
        return Ok(MAX_BPS);
    }
    let numerator = total_assets.checked_mul(MAX_BPS).ok_or(MathError::Overflow)?;
    Ok(numerator / total_shares) // floor division
}

// ─────────────────────────────────────────────────────────────────────────────
// Unit tests for the math primitives
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_fee_floor() {
        // 1% of 999 = 9.99 → floor → 9
        assert_eq!(protocol_fee(999, 100).unwrap(), 9);
    }

    #[test]
    fn protocol_fee_zero_stake() {
        assert_eq!(protocol_fee(0, 500).unwrap(), 0);
    }

    #[test]
    fn protocol_fee_zero_bps() {
        assert_eq!(protocol_fee(1_000_000, 0).unwrap(), 0);
    }

    #[test]
    fn protocol_fee_full_bps() {
        assert_eq!(protocol_fee(1_000, 10_000).unwrap(), 1_000);
    }

    #[test]
    fn protocol_fee_invalid_bps() {
        assert_eq!(protocol_fee(1_000, 10_001), Err(MathError::InvalidFeeBps));
    }

    #[test]
    fn payout_pool_equals_stake_minus_fee() {
        let total = 10_000i128;
        let bps = 250i128;
        let fee = protocol_fee(total, bps).unwrap();
        assert_eq!(payout_pool(total, bps).unwrap(), total - fee);
    }

    #[test]
    fn winner_share_sole_winner() {
        // Sole winner gets the entire payout pool
        assert_eq!(winner_share(500, 500, 990).unwrap(), 990);
    }

    #[test]
    fn winner_share_proportional() {
        // 250 / 500 * 1000 = 500
        assert_eq!(winner_share(250, 500, 1_000).unwrap(), 500);
    }

    #[test]
    fn winner_share_floor() {
        // 1 / 3 * 10 = 3.333… → floor → 3
        assert_eq!(winner_share(1, 3, 10).unwrap(), 3);
    }

    #[test]
    fn winner_share_zero_winning_stake() {
        assert_eq!(winner_share(100, 0, 1_000).unwrap(), 0);
    }

    #[test]
    fn claim_payout_invariant_iv() {
        let out = claim_payout(1_000, 100, 400, 400).unwrap();
        assert_eq!(out.protocol_fee, 10);
        assert_eq!(out.payout_pool, 990);
        assert_eq!(out.winnings, 990); // sole winner
        assert!(out.winnings <= 1_000);
    }

    #[test]
    fn claim_payout_loser_gets_zero() {
        // User is on the losing side — handled by the model, not this function,
        // but we verify a zero winning_stake path returns 0
        let out = claim_payout(1_000, 100, 100, 0).unwrap();
        assert_eq!(out.winnings, 0);
    }

    #[test]
    fn dynamic_fee_no_tiers() {
        assert_eq!(dynamic_fee(999_999, 300, &[]), 300);
    }

    #[test]
    fn dynamic_fee_picks_highest_met_threshold() {
        let tiers = vec![
            FeeTier { stake_threshold: 1_000, fee_bps: 250 },
            FeeTier { stake_threshold: 5_000, fee_bps: 200 },
            FeeTier { stake_threshold: 10_000, fee_bps: 150 },
        ];
        // total_stake = 7_500 → tier at 5_000 matches, tier at 10_000 doesn't
        assert_eq!(dynamic_fee(7_500, 300, &tiers), 200);
    }

    #[test]
    fn dynamic_fee_below_first_threshold() {
        let tiers = vec![FeeTier { stake_threshold: 5_000, fee_bps: 150 }];
        assert_eq!(dynamic_fee(4_999, 300, &tiers), 300);
    }

    #[test]
    fn dynamic_fee_at_exact_threshold() {
        let tiers = vec![FeeTier { stake_threshold: 5_000, fee_bps: 150 }];
        assert_eq!(dynamic_fee(5_000, 300, &tiers), 150);
    }

    #[test]
    fn referral_amount_proportional() {
        // User has 50% of pool; fee total = 100; referral cut = 10% → 5
        let amount = referral_amount(500, 1_000, 100, 1_000).unwrap();
        assert_eq!(amount, 5);
    }

    #[test]
    fn referral_amount_zero_when_no_fee() {
        assert_eq!(referral_amount(500, 1_000, 0, 1_000).unwrap(), 0);
    }

    #[test]
    fn sum_of_floored_winner_shares_never_exceeds_pool() {
        // 3 equal winners sharing a payout_pool that doesn't divide evenly
        let pool = 10i128;
        let winning_stake = 3i128;
        let w1 = winner_share(1, winning_stake, pool).unwrap(); // floor(10/3) = 3
        let w2 = winner_share(1, winning_stake, pool).unwrap();
        let w3 = winner_share(1, winning_stake, pool).unwrap();
        assert!(w1 + w2 + w3 <= pool, "sum of shares must not exceed pool");
    }

    // ── LP accounting tests (RFC #268) ──────────────────────────────────────

    #[test]
    fn lp_deposit_bootstrap_is_one_to_one() {
        assert_eq!(lp_shares_for_deposit(1_000, 0, 0).unwrap(), 1_000);
    }

    #[test]
    fn lp_deposit_proportional() {
        // Pool: 1_000 assets / 1_000 shares. Deposit 500 → 500 shares.
        assert_eq!(lp_shares_for_deposit(500, 1_000, 1_000).unwrap(), 500);
    }

    #[test]
    fn lp_deposit_floors() {
        // 1 * 3 / 2 = 1.5 → floor → 1
        assert_eq!(lp_shares_for_deposit(1, 2, 3).unwrap(), 1);
    }

    #[test]
    fn lp_withdrawal_proportional() {
        assert_eq!(lp_assets_for_withdrawal(500, 1_000, 1_000).unwrap(), 500);
    }

    #[test]
    fn lp_withdrawal_floors() {
        // 1 * 10 / 3 = 3.333… → floor → 3
        assert_eq!(lp_assets_for_withdrawal(1, 10, 3).unwrap(), 3);
    }

    #[test]
    fn lp_withdrawal_empty_pool_errors() {
        assert_eq!(
            lp_assets_for_withdrawal(1, 0, 0),
            Err(MathError::DivisionByZero)
        );
    }

    #[test]
    fn lp_fee_accrual_increases_share_price() {
        let assets = lp_accrue_fee(1_000, 100).unwrap();
        assert_eq!(assets, 1_100);
        // 1_100 / 1_000 shares = 1.1 → 11_000 bps
        assert_eq!(lp_share_price_bps(assets, 1_000).unwrap(), 11_000);
    }

    #[test]
    fn lp_loss_allocation_reduces_assets() {
        assert_eq!(lp_allocate_loss(1_000, 250).unwrap(), 750);
    }

    #[test]
    fn lp_loss_allocation_clamps_at_zero() {
        assert_eq!(lp_allocate_loss(100, 250).unwrap(), 0);
    }

    #[test]
    fn lp_share_price_empty_pool_is_one() {
        assert_eq!(lp_share_price_bps(0, 0).unwrap(), MAX_BPS);
    }

    #[test]
    fn lp_negative_inputs_rejected() {
        assert_eq!(
            lp_shares_for_deposit(-1, 1_000, 1_000),
            Err(MathError::NegativeInput)
        );
        assert_eq!(
            lp_assets_for_withdrawal(-1, 1_000, 1_000),
            Err(MathError::NegativeInput)
        );
        assert_eq!(lp_accrue_fee(-1, 1), Err(MathError::NegativeInput));
        assert_eq!(lp_allocate_loss(-1, 1), Err(MathError::NegativeInput));
        assert_eq!(lp_share_price_bps(-1, 1), Err(MathError::NegativeInput));
    }

    /// LP-2: a single withdrawal never returns more than the pool holds.
    #[test]
    fn lp_withdrawal_never_exceeds_assets() {
        for shares in 0..50i128 {
            for assets in 0..50i128 {
                for total_shares in 1..50i128 {
                    let out = lp_assets_for_withdrawal(shares, assets, total_shares).unwrap();
                    assert!(out <= assets, "withdrawal {out} > assets {assets}");
                }
            }
        }
    }

    /// LP-1/LP-2: depositing then immediately withdrawing never yields more
    /// than was deposited (rounding dust stays in the pool).
    #[test]
    fn lp_deposit_then_withdraw_never_profits() {
        for amount in 0..50i128 {
            for assets in 0..50i128 {
                for shares in 0..50i128 {
                    let minted = lp_shares_for_deposit(amount, assets, shares).unwrap();
                    let new_assets = assets + amount;
                    let new_shares = shares + minted;
                    if new_shares == 0 {
                        continue;
                    }
                    let back =
                        lp_assets_for_withdrawal(minted, new_assets, new_shares).unwrap();
                    assert!(back <= amount, "round-trip {back} > deposit {amount}");
                }
            }
        }
    }

    /// LP-3: assets evolve consistently through fee accrual and loss.
    #[test]
    fn lp_conservation_through_fees_and_losses() {
        let mut assets = 1_000i128;
        let shares = 1_000i128;
        assets = lp_accrue_fee(assets, 100).unwrap();
        assets = lp_allocate_loss(assets, 50).unwrap();
        // 1_000 + 100 - 50 = 1_050
        assert_eq!(assets, 1_050);
        // share price = 1_050 / 1_000 = 1.05 → 10_500 bps
        assert_eq!(lp_share_price_bps(assets, shares).unwrap(), 10_500);
    }
}
