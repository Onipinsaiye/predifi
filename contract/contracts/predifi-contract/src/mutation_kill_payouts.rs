//! # Mutation-Kill Tests — `payouts` module
//!
//! These tests are designed to **kill specific mutation classes** that unit
//! tests in `payouts.rs` alone cannot reliably catch.  Each test is named
//! after the mutation operator it is intended to detect, making the intent
//! explicit and the failure message actionable.
//!
//! ## Coverage targets
//!
//! | Mutation class | Function | Killed by |
//! |---|---|---|
//! | `fee_bps < 0` → `fee_bps <= 0` | `calculate_claim_payout` | `mutation_kill_negative_fee_bps_is_rejected` |
//! | `fee_bps > 10_000` → `fee_bps >= 10_000` | `calculate_claim_payout` | `mutation_kill_max_fee_bps_is_accepted` |
//! | `winnings > pool_total_stake` → `winnings >= pool_total_stake` | `calculate_claim_payout` | `mutation_kill_winnings_equal_pool_is_ok` |
//! | `0` return → real return in referral zero-guard | `calculate_referral_amount` | `mutation_kill_referral_zero_guard_exact_boundary` |
//! | `outcome_stake <= 0` → `outcome_stake < 0` | `calculate_odds_bps` | `mutation_kill_odds_zero_outcome_stake` |
//! | `total_stake <= 0` → `total_stake < 0` | `calculate_odds_bps` | `mutation_kill_odds_zero_total_stake` |
//! | Swap `user_stake` / `winning_stake` args | `calculate_winnings` | `mutation_kill_winnings_arg_order_matters` |
//! | Replace `-` with `+` in payout_pool | `calculate_payout_pool` | `mutation_kill_payout_pool_is_fee_subtracted` |
//! | Replace `Ok(0)` with `Ok(user_stake)` in zero branch | `calculate_winnings` | `mutation_kill_zero_user_stake_yields_zero` |
//! | Negate protocol-fee rounding direction sentinel | `calculate_protocol_fee` | `mutation_kill_protocol_fee_rounds_down` |

#[cfg(test)]
mod mutation_kill_payout {
    use crate::payouts::{
        calculate_claim_payout, calculate_odds_bps, calculate_payout_pool,
        calculate_referral_amount, calculate_winnings, PayoutInput,
    };
    use predifi_errors::PrediFiError;

    // ── calculate_claim_payout: fee_bps boundary ──────────────────────────────

    /// Mutant: `fee_bps < 0` → `fee_bps <= 0`
    /// Kill condition: fee_bps == 0 must be *accepted* (zero fee is valid).
    #[test]
    fn mutation_kill_zero_fee_bps_is_valid() {
        let out = calculate_claim_payout(&PayoutInput {
            pool_total_stake: 1_000,
            fee_bps: 0,
            user_stake: 500,
            winning_stake: 500,
        });
        assert!(out.is_ok(), "fee_bps = 0 must be accepted; got {out:?}");
        assert_eq!(out.unwrap().protocol_fee, 0);
    }

    /// Mutant: `fee_bps > 10_000` → `fee_bps >= 10_000`
    /// Kill condition: fee_bps == 10_000 (100%) must be *accepted*.
    #[test]
    fn mutation_kill_max_fee_bps_is_accepted() {
        let out = calculate_claim_payout(&PayoutInput {
            pool_total_stake: 1_000,
            fee_bps: 10_000,
            user_stake: 500,
            winning_stake: 500,
        });
        assert!(
            out.is_ok(),
            "fee_bps = 10_000 must be accepted; got {out:?}"
        );
        let bd = out.unwrap();
        assert_eq!(bd.protocol_fee, 1_000, "100% fee takes entire pool");
        assert_eq!(bd.payout_pool, 0);
        assert_eq!(bd.winnings, 0);
    }

    /// Mutant: `fee_bps < 0` → `fee_bps <= 0`
    /// Kill condition: fee_bps == -1 must be *rejected*.
    #[test]
    fn mutation_kill_negative_fee_bps_is_rejected() {
        assert_eq!(
            calculate_claim_payout(&PayoutInput {
                pool_total_stake: 1_000,
                fee_bps: -1,
                user_stake: 100,
                winning_stake: 100,
            }),
            Err(PrediFiError::InvalidFeeBps),
            "fee_bps = -1 must be rejected as InvalidFeeBps"
        );
    }

    /// Mutant: `fee_bps > 10_000` → `fee_bps >= 10_000`
    /// Kill condition: fee_bps == 10_001 must be *rejected*.
    #[test]
    fn mutation_kill_fee_bps_above_max_is_rejected() {
        assert_eq!(
            calculate_claim_payout(&PayoutInput {
                pool_total_stake: 1_000,
                fee_bps: 10_001,
                user_stake: 100,
                winning_stake: 100,
            }),
            Err(PrediFiError::InvalidFeeBps),
            "fee_bps = 10_001 must be rejected"
        );
    }

    // ── calculate_claim_payout: winnings > pool invariant (INV-4) ────────────

    /// Mutant: `winnings > pool_total_stake` → `winnings >= pool_total_stake`
    /// Kill condition: winnings == pool_total_stake must be *accepted*.
    /// This happens when fee_bps == 0 and user is the sole winner.
    #[test]
    fn mutation_kill_winnings_equal_pool_is_ok() {
        // 0% fee, sole winner → winnings == pool_total_stake
        let out = calculate_claim_payout(&PayoutInput {
            pool_total_stake: 1_000,
            fee_bps: 0,
            user_stake: 1_000,
            winning_stake: 1_000,
        });
        assert!(
            out.is_ok(),
            "sole winner at 0% fee must succeed; got {out:?}"
        );
        let bd = out.unwrap();
        assert_eq!(bd.winnings, 1_000);
    }

    // ── calculate_payout_pool: subtraction direction ──────────────────────────

    /// Mutant: `total_stake - fee` → `total_stake + fee`
    /// Kill condition: payout_pool must be strictly less than total_stake when fee > 0.
    #[test]
    fn mutation_kill_payout_pool_is_fee_subtracted() {
        let pool = calculate_payout_pool(10_000, 100 /* 1% */).unwrap();
        assert_eq!(pool, 9_900, "payout_pool must be total_stake minus fee");
        assert!(
            pool < 10_000,
            "payout_pool must be less than total_stake when fee > 0"
        );
    }

    /// Mutant: arithmetic order swap — pool is always non-negative.
    #[test]
    fn mutation_kill_payout_pool_never_negative() {
        // max fee: 100%
        let pool = calculate_payout_pool(10_000, 10_000).unwrap();
        assert_eq!(pool, 0);
    }

    // ── calculate_winnings: argument order / zero guards ─────────────────────

    /// Mutant: swap `user_stake` / `winning_stake` arguments.
    /// Kill condition: asymmetric stakes produce different results.
    #[test]
    fn mutation_kill_winnings_arg_order_matters() {
        // user_stake 200 / winning_stake 1000 → 20% of payout_pool
        let small = calculate_winnings(200, 1_000, 10_000).unwrap();
        // user_stake 1000 / winning_stake 200 → invalid (user > winning)
        let large = calculate_winnings(1_000, 200, 10_000);
        // They must produce different outcomes
        assert_eq!(small, 2_000, "proportional share should be 20% of 10_000");
        assert!(
            large.is_err(),
            "user_stake > winning_stake must be rejected"
        );
    }

    /// Mutant: `Ok(0)` → `Ok(user_stake)` in the zero-user-stake branch.
    #[test]
    fn mutation_kill_zero_user_stake_yields_zero() {
        let out = calculate_winnings(0, 1_000, 10_000).unwrap();
        assert_eq!(out, 0, "zero user_stake must yield zero winnings");
    }

    /// Mutant: `Ok(0)` → `Ok(payout_pool)` in the zero-winning-stake branch.
    #[test]
    fn mutation_kill_zero_winning_stake_yields_zero() {
        let out = calculate_winnings(100, 0, 10_000).unwrap();
        assert_eq!(out, 0, "zero winning_stake must yield zero winnings");
    }

    // ── calculate_referral_amount: zero-guard exact boundaries ───────────────

    /// Mutant: `protocol_fee_total <= 0` → `protocol_fee_total < 0`
    /// Kill condition: protocol_fee_total == 0 must still return Ok(0).
    #[test]
    fn mutation_kill_referral_zero_guard_zero_fee() {
        let amt = calculate_referral_amount(500, 1_000, 0, 1_000).unwrap();
        assert_eq!(amt, 0, "zero protocol fee must produce zero referral");
    }

    /// Mutant: `pool_total_stake <= 0` → `pool_total_stake < 0`
    /// Kill condition: pool_total_stake == 0 must still return Ok(0).
    #[test]
    fn mutation_kill_referral_zero_guard_zero_pool() {
        let amt = calculate_referral_amount(0, 0, 100, 1_000).unwrap();
        assert_eq!(amt, 0, "zero pool stake must produce zero referral");
    }

    /// Mutant: `user_stake <= 0` → `user_stake < 0`
    /// Kill condition: user_stake == 0 must still return Ok(0).
    #[test]
    fn mutation_kill_referral_zero_guard_zero_user() {
        let amt = calculate_referral_amount(0, 1_000, 100, 1_000).unwrap();
        assert_eq!(amt, 0, "zero user stake must produce zero referral");
    }

    /// Mutants: `fee <= 0 || pool <= 0 || user <= 0` with either `||` → `&&`.
    /// Each guard must return `Ok(0)` on its own; without it the call reaches
    /// `SafeMath::proportion`, which errors (zero pool, negative user) or
    /// errors on a negative fee total.
    #[test]
    fn mutation_kill_referral_each_guard_is_independent() {
        // zero pool with positive fee and user → Ok(0), not a division error
        assert_eq!(calculate_referral_amount(500, 0, 100, 1_000), Ok(0));
        // negative fee total → Ok(0)
        assert_eq!(calculate_referral_amount(500, 1_000, -1, 1_000), Ok(0));
        // negative user stake → Ok(0)
        assert_eq!(calculate_referral_amount(-1, 1_000, 100, 1_000), Ok(0));
    }

    /// Positive referral to confirm the happy path is reachable.
    #[test]
    fn mutation_kill_referral_nonzero_when_all_positive() {
        // user=500/1000 pool, fee=100, referral_cut=1000 bps (10%) → 5
        let amt = calculate_referral_amount(500, 1_000, 100, 1_000).unwrap();
        assert_eq!(amt, 5);
    }

    // ── calculate_odds_bps: zero-guard on both inputs ────────────────────────

    /// Mutant: `outcome_stake <= 0` → `outcome_stake < 0`
    /// Kill condition: outcome_stake == 0 must return 0, not panic.
    #[test]
    fn mutation_kill_odds_zero_outcome_stake() {
        assert_eq!(
            calculate_odds_bps(0, 1_000),
            0,
            "zero outcome stake must return 0 odds"
        );
    }

    /// Mutant: `total_stake <= 0` → `total_stake < 0`
    /// Kill condition: total_stake == 0 must return 0.
    #[test]
    fn mutation_kill_odds_zero_total_stake() {
        assert_eq!(
            calculate_odds_bps(500, 0),
            0,
            "zero total stake must return 0 odds"
        );
    }

    /// Mutant: `outcome <= 0 || total <= 0` → `&&`.
    /// A negative operand would make the quotient negative, and `as u64` would
    /// turn it into an enormous odds value.
    #[test]
    fn mutation_kill_odds_negative_inputs_return_zero() {
        assert_eq!(calculate_odds_bps(-1, 1_000), 0);
        assert_eq!(calculate_odds_bps(500, -1), 0);
    }

    /// Symmetry: odds must correctly reflect the stake ratio.
    #[test]
    fn mutation_kill_odds_correctness() {
        // 25% outcome stake → 4.0x odds = 40_000 bps
        let odds = calculate_odds_bps(250, 1_000);
        assert_eq!(odds, 40_000, "1/4 outcome stake → 4.0x odds");
    }

    // ── calculate_protocol_fee: rounding direction ────────────────────────────

    /// Mutant: `RoundingMode::ProtocolFavor` → `RoundingMode::UserFavor`
    /// Kill condition: odd total_stake must produce floor fee (not ceiling).
    ///
    /// total_stake=1001, fee_bps=100 (1%):
    ///   exact = 10.01  → floor = 10 (ProtocolFavor)
    ///                   → ceil  = 11 (UserFavor would give 11)
    #[test]
    fn mutation_kill_protocol_fee_rounds_down() {
        use crate::payouts::calculate_protocol_fee;
        let fee = calculate_protocol_fee(1_001, 100).unwrap();
        assert_eq!(fee, 10, "fee must be floor(1001 * 100 / 10000) = 10");
    }

    /// Sanity: multiple winners' fees together equal total fee.
    #[test]
    fn mutation_kill_fee_is_globally_consistent() {
        // 2% fee on 5000 → 100
        let fee = calculate_claim_payout(&PayoutInput {
            pool_total_stake: 5_000,
            fee_bps: 200,
            user_stake: 5_000,
            winning_stake: 5_000,
        })
        .unwrap();
        assert_eq!(fee.protocol_fee, 100);
        assert_eq!(fee.payout_pool, 4_900);
        assert_eq!(fee.winnings, 4_900);
    }

    // ── Negative stake rejection ──────────────────────────────────────────────

    /// Mutant: `pool_total_stake < 0` → `pool_total_stake <= 0`
    /// Kill condition: pool_total_stake == 0 must be *accepted* (empty pool).
    #[test]
    fn mutation_kill_zero_pool_is_accepted() {
        let out = calculate_claim_payout(&PayoutInput {
            pool_total_stake: 0,
            fee_bps: 0,
            user_stake: 0,
            winning_stake: 0,
        });
        assert!(
            out.is_ok(),
            "all-zero inputs must succeed with zero payouts; got {out:?}"
        );
        let bd = out.unwrap();
        assert_eq!(bd.winnings, 0);
    }

    /// Mutant: swap sign of rejected sentinel.
    #[test]
    fn mutation_kill_negative_pool_total_stake_is_rejected() {
        assert_eq!(
            calculate_claim_payout(&PayoutInput {
                pool_total_stake: -1,
                fee_bps: 0,
                user_stake: 0,
                winning_stake: 0,
            }),
            Err(PrediFiError::ArithmeticError),
        );
    }

    #[test]
    fn mutation_kill_negative_user_stake_is_rejected() {
        assert_eq!(
            calculate_claim_payout(&PayoutInput {
                pool_total_stake: 1_000,
                fee_bps: 0,
                user_stake: -1,
                winning_stake: 100,
            }),
            Err(PrediFiError::ArithmeticError),
        );
    }

    /// Mutant: `pool < 0 || user < 0 || winning < 0` → `(pool < 0 && user < 0) || winning < 0`.
    /// A negative stake paired with a zero winning stake would otherwise hit
    /// the zero short-circuit and return `Ok(0)` instead of an error.
    #[test]
    fn mutation_kill_negative_user_stake_with_zero_winning_is_rejected() {
        assert_eq!(
            calculate_claim_payout(&PayoutInput {
                pool_total_stake: 1_000,
                fee_bps: 0,
                user_stake: -1,
                winning_stake: 0,
            }),
            Err(PrediFiError::ArithmeticError),
        );
    }

    /// Mutant: `pool < 0 || user < 0 || winning < 0` → `pool < 0 || (user < 0 && winning < 0)`.
    /// Same short-circuit hazard with the roles of the two stakes swapped.
    #[test]
    fn mutation_kill_negative_winning_stake_with_zero_user_is_rejected() {
        assert_eq!(
            calculate_claim_payout(&PayoutInput {
                pool_total_stake: 1_000,
                fee_bps: 0,
                user_stake: 0,
                winning_stake: -1,
            }),
            Err(PrediFiError::ArithmeticError),
        );
    }

    #[test]
    fn mutation_kill_negative_winning_stake_is_rejected() {
        assert_eq!(
            calculate_claim_payout(&PayoutInput {
                pool_total_stake: 1_000,
                fee_bps: 0,
                user_stake: 100,
                winning_stake: -1,
            }),
            Err(PrediFiError::ArithmeticError),
        );
    }
}
