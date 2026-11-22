//! # Payout calculation helpers
//!
//! Pure functions for computing winner payouts, protocol fees, and referral
//! amounts from final pool state. All functions are side-effect free and can
//! be unit-tested without a Soroban environment.
//!
//! ## Payout model
//!
//! When a pool resolves:
//! 1. The protocol deducts `fee_bps` basis points from `pool_total_stake`
//!    (`protocol_fee`).
//! 2. The remaining `payout_pool = pool_total_stake - protocol_fee` is
//!    distributed to winners proportionally to their stake on the winning
//!    outcome: `winnings = user_stake * payout_pool / winning_stake`.
//! 3. If a referrer recorded their volume above the minimum threshold they
//!    receive `referral_cut_bps` of the protocol fee back as a referral bonus.
//!
//! All calculations round down (floor) in favour of the protocol. The
//! integer-division remainder is implicitly retained as a dust reserve.

use crate::PredifiError;

/// Full payout breakdown for a single winner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayoutBreakdown {
    /// Gross winnings (user_stake * payout_pool / winning_stake).
    pub winnings: i128,
    /// Total protocol fee deducted from the pool.
    pub protocol_fee: i128,
    /// Net amount transferred to the winner (winnings - user_proportional_fee).
    pub net_payout: i128,
}

/// Input parameters for payout computation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayoutInput {
    /// User's stake on the winning outcome.
    pub user_stake: i128,
    /// Total stake on the winning outcome across all participants.
    pub winning_stake: i128,
    /// Total pool stake (all outcomes, including initial liquidity).
    pub pool_total_stake: i128,
    /// Protocol fee in basis points.
    pub fee_bps: i128,
}

/// Compute odds in basis points for an outcome.
///
/// `odds_bps = outcome_stake * 10_000 / total_stake`
///
/// Returns 0 if either input is non-positive.
pub fn calculate_odds_bps(outcome_stake: i128, total_stake: i128) -> i128 {
    if outcome_stake <= 0 || total_stake <= 0 {
        return 0;
    }
    outcome_stake.saturating_mul(10_000) / total_stake
}

/// Compute the protocol fee for a given total stake and fee rate.
///
/// `fee = total_stake * fee_bps / 10_000`
pub fn calculate_protocol_fee(total_stake: i128, fee_bps: i128) -> Result<i128, PredifiError> {
    if total_stake < 0 || fee_bps < 0 || fee_bps > 10_000 {
        return Err(PredifiError::InvalidAmount);
    }
    Ok(total_stake.saturating_mul(fee_bps) / 10_000)
}

/// Compute gross winnings for a user on the winning outcome.
///
/// `winnings = user_stake * (total_stake - protocol_fee) / winning_stake`
pub fn calculate_winnings(
    user_stake: i128,
    winning_stake: i128,
    total_stake: i128,
    fee_bps: i128,
) -> Result<i128, PredifiError> {
    if user_stake < 0 || winning_stake <= 0 || total_stake < 0 {
        return Err(PredifiError::InvalidAmount);
    }
    let protocol_fee = calculate_protocol_fee(total_stake, fee_bps)?;
    let payout_pool = total_stake.saturating_sub(protocol_fee);
    let winnings = user_stake.saturating_mul(payout_pool) / winning_stake;
    Ok(winnings)
}

/// Compute the referral bonus owed to a referrer.
///
/// `referral_bonus = protocol_fee * user_stake_bps / 10_000 * referral_cut_bps / 10_000`
///
/// When called as: `calculate_referral_amount(user_stake, pool_total_stake, protocol_fee, referral_cut_bps)`
/// returns `protocol_fee * (user_stake / pool_total_stake) * referral_cut_bps`.
pub fn calculate_referral_amount(
    user_stake: i128,
    pool_total_stake: i128,
    protocol_fee: i128,
    referral_cut_bps: i128,
) -> Result<i128, PredifiError> {
    if user_stake < 0 || pool_total_stake <= 0 || protocol_fee < 0 || referral_cut_bps < 0 || referral_cut_bps > 10_000 {
        return Err(PredifiError::InvalidAmount);
    }
    // User's proportional share of protocol fee:
    // user_fee_share = protocol_fee * user_stake / pool_total_stake
    let user_fee_share = protocol_fee.saturating_mul(user_stake) / pool_total_stake;
    // Referral bonus = user_fee_share * referral_cut_bps / 10_000
    Ok(user_fee_share.saturating_mul(referral_cut_bps) / 10_000)
}

/// Compute the final claim payout for a winner, net of fees.
///
/// Returns the full [`PayoutBreakdown`] so callers can audit each component.
pub fn calculate_claim_payout(input: &PayoutInput) -> Result<PayoutBreakdown, PredifiError> {
    if input.user_stake < 0 || input.winning_stake <= 0 || input.pool_total_stake < 0 {
        return Err(PredifiError::InvalidAmount);
    }
    let protocol_fee = calculate_protocol_fee(input.pool_total_stake, input.fee_bps)?;
    let payout_pool = input.pool_total_stake.saturating_sub(protocol_fee);
    let winnings = input
        .user_stake
        .saturating_mul(payout_pool)
        / input.winning_stake;
    // User's net payout = winnings (already net of fee since payout_pool is fee-adjusted)
    let net_payout = winnings;

    Ok(PayoutBreakdown {
        winnings,
        protocol_fee,
        net_payout,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_odds_bps_basic() {
        // 50% odds = 5000 bps
        assert_eq!(calculate_odds_bps(500, 1000), 5000);
    }

    #[test]
    fn test_calculate_odds_bps_zero_inputs() {
        assert_eq!(calculate_odds_bps(0, 1000), 0);
        assert_eq!(calculate_odds_bps(500, 0), 0);
        assert_eq!(calculate_odds_bps(-1, 1000), 0);
        assert_eq!(calculate_odds_bps(500, -1), 0);
    }

    #[test]
    fn test_calculate_protocol_fee() {
        // 1% fee on 1000 = 10
        assert_eq!(calculate_protocol_fee(1000, 100), Ok(10));
    }

    #[test]
    fn test_calculate_winnings_equal_split() {
        // User with 500/1000 stake, pool = 1000, fee = 0
        assert_eq!(calculate_winnings(500, 1000, 1000, 0), Ok(500));
    }

    #[test]
    fn test_calculate_claim_payout_zero_fee() {
        let input = PayoutInput {
            user_stake: 500,
            winning_stake: 1000,
            pool_total_stake: 1000,
            fee_bps: 0,
        };
        let breakdown = calculate_claim_payout(&input).unwrap();
        assert_eq!(breakdown.winnings, 500);
        assert_eq!(breakdown.protocol_fee, 0);
    }
}
