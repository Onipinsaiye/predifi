//! # Mutation-Kill Tests — `safe_math` module
//!
//! These tests are designed to kill specific mutations inside the
//! `SafeMath` arithmetic primitives.  Each test is named after the operator
//! it defeats, making CI failure messages self-documenting.
//!
//! ## Coverage targets
//!
//! | Mutation class | Function | Killed by |
//! |---|---|---|
//! | `amount < 0` → `amount <= 0` | `percentage` | `mutation_kill_percentage_zero_amount_ok` |
//! | `bps == 0` short-circuit removed | `percentage` | `mutation_kill_percentage_zero_bps_is_zero` |
//! | `amount * bps` → `amount / bps` | `percentage` | `mutation_kill_percentage_multiplication` |
//! | `numerator > denominator` → `numerator >= denominator` | `proportion` | `mutation_kill_proportion_equal_is_valid` |
//! | `denominator <= 0` → `denominator < 0` | `proportion` | `mutation_kill_proportion_zero_denom_rejected` |
//! | `ProtocolFavor` → floor eliminated | `divide_with_rounding` | `mutation_kill_protocol_favor_is_floor` |
//! | `Neutral` half-up → truncate | `divide_with_rounding` | `mutation_kill_neutral_rounds_half_up` |
//! | `UserFavor` ceiling → floor | `divide_with_rounding` | `mutation_kill_user_favor_is_ceiling` |
//! | `user_stake > winning_stake` → `>=` | `calculate_share` | `mutation_kill_share_equal_is_valid` |
//! | `product / winning_stake` → `product * winning_stake` | `calculate_share` | `mutation_kill_share_division` |
//! | `safe_add` overflow direction | `safe_add` | `mutation_kill_safe_add_is_checked` |
//! | `safe_sub` underflow direction | `safe_sub` | `mutation_kill_safe_sub_is_checked` |

#[cfg(test)]
mod mutation_kill_safe_math {
    use crate::safe_math::{RoundingMode, SafeMath};
    use predifi_errors::PrediFiError;

    // ── SafeMath::percentage ──────────────────────────────────────────────────

    /// Mutant: `amount < 0` → `amount <= 0`
    /// Kill: amount == 0 must be accepted and return 0.
    #[test]
    fn mutation_kill_percentage_zero_amount_ok() {
        let r = SafeMath::percentage(0, 500, RoundingMode::ProtocolFavor).unwrap();
        assert_eq!(r, 0, "percentage of 0 must be 0");
    }

    /// Mutant: Remove `bps == 0` early-return.
    /// Kill: must still return 0 when bps == 0.
    #[test]
    fn mutation_kill_percentage_zero_bps_is_zero() {
        let r = SafeMath::percentage(1_000_000, 0, RoundingMode::ProtocolFavor).unwrap();
        assert_eq!(r, 0, "0 bps fee must be 0 regardless of amount");
    }

    /// Mutant: `amount * bps` → `amount + bps`.
    /// Kill: 5% of 1_000 must be 50, not 1005.
    #[test]
    fn mutation_kill_percentage_multiplication() {
        let r = SafeMath::percentage(1_000, 500, RoundingMode::ProtocolFavor).unwrap();
        assert_eq!(r, 50, "5% of 1000 must be 50");
    }

    /// Mutant: `bps > MAX_BPS` → `bps >= MAX_BPS`.
    /// Kill: bps == 10_000 (100%) must be accepted.
    #[test]
    fn mutation_kill_percentage_max_bps_accepted() {
        let r = SafeMath::percentage(1_000, 10_000, RoundingMode::ProtocolFavor).unwrap();
        assert_eq!(r, 1_000, "100% of 1000 must be 1000");
    }

    /// Mutant: `bps > MAX_BPS` → `bps >= MAX_BPS`.
    /// Kill: bps == 10_001 must be rejected.
    #[test]
    fn mutation_kill_percentage_above_max_rejected() {
        assert_eq!(
            SafeMath::percentage(1_000, 10_001, RoundingMode::ProtocolFavor),
            Err(PrediFiError::InvalidFeeBps)
        );
    }

    /// Mutant: negative amount guard flipped.
    #[test]
    fn mutation_kill_percentage_negative_amount_rejected() {
        assert_eq!(
            SafeMath::percentage(-1, 100, RoundingMode::ProtocolFavor),
            Err(PrediFiError::ArithmeticError)
        );
    }

    // ── SafeMath::proportion ──────────────────────────────────────────────────

    /// Mutant: `numerator > denominator` → `numerator >= denominator`
    /// Kill: numerator == denominator (100% share) must be accepted.
    #[test]
    fn mutation_kill_proportion_equal_is_valid() {
        let r = SafeMath::proportion(500, 500, 1_000, RoundingMode::ProtocolFavor).unwrap();
        assert_eq!(r, 1_000, "100% share must equal the full amount");
    }

    /// Mutant: `denominator <= 0` → `denominator < 0`
    /// Kill: zero denominator must be rejected.
    #[test]
    fn mutation_kill_proportion_zero_denom_rejected() {
        assert_eq!(
            SafeMath::proportion(100, 0, 1_000, RoundingMode::ProtocolFavor),
            Err(PrediFiError::ArithmeticError)
        );
    }

    /// Mutant: `numerator < 0 || denominator <= 0 || amount < 0` → `... || (denominator <= 0 && amount < 0)`.
    /// A zero denominator must be rejected even when the numerator is 0 (which
    /// would otherwise hit the `Ok(0)` short-circuit), and a negative amount
    /// must be rejected rather than producing a negative payout.
    #[test]
    fn mutation_kill_proportion_input_guards_are_independent() {
        assert_eq!(
            SafeMath::proportion(0, 0, 1_000, RoundingMode::ProtocolFavor),
            Err(PrediFiError::ArithmeticError)
        );
        assert_eq!(
            SafeMath::proportion(100, 500, -1, RoundingMode::ProtocolFavor),
            Err(PrediFiError::ArithmeticError)
        );
    }

    /// Mutant: `numerator == 0` short-circuit removed.
    /// Kill: zero numerator must return 0.
    #[test]
    fn mutation_kill_proportion_zero_numerator_is_zero() {
        let r = SafeMath::proportion(0, 500, 1_000, RoundingMode::ProtocolFavor).unwrap();
        assert_eq!(r, 0);
    }

    /// Mutant: `amount == 0` short-circuit removed.
    /// Kill: zero amount must return 0.
    #[test]
    fn mutation_kill_proportion_zero_amount_is_zero() {
        let r = SafeMath::proportion(100, 500, 0, RoundingMode::ProtocolFavor).unwrap();
        assert_eq!(r, 0);
    }

    /// Mutant: `numerator > denominator` guard removed entirely.
    /// Kill: numerator > denominator must be rejected.
    #[test]
    fn mutation_kill_proportion_numerator_exceeds_denom_rejected() {
        assert_eq!(
            SafeMath::proportion(600, 500, 1_000, RoundingMode::ProtocolFavor),
            Err(PrediFiError::ArithmeticError)
        );
    }

    // ── divide_with_rounding: all three rounding modes ────────────────────────

    /// Mutant: `ProtocolFavor` branch returns `quotient + 1` instead of `quotient`.
    /// Kill: 1001 / 10 = 100 remainder 1 → floor = 100, NOT 101.
    #[test]
    fn mutation_kill_protocol_favor_is_floor() {
        // 1001 * 100 / 10_000 = 10.01 → floor = 10
        let r = SafeMath::percentage(1_001, 100, RoundingMode::ProtocolFavor).unwrap();
        assert_eq!(r, 10, "ProtocolFavor must floor (truncate) the result");
    }

    /// Mutant: `Neutral` branch truncates instead of rounding half-up.
    /// Kill: 5 / 10 = 0 remainder 5, which is exactly half → round up to 1.
    #[test]
    fn mutation_kill_neutral_rounds_half_up() {
        // 1 * 5000 / 10_000 = 0.5 → Neutral rounds to 1
        let r = SafeMath::percentage(1, 5_000, RoundingMode::Neutral).unwrap();
        assert_eq!(r, 1, "Neutral mode must round 0.5 up to 1");
    }

    /// Mutant: `Neutral` branch always rounds up (same as UserFavor).
    /// Kill: 1 * 4999 / 10_000 = 0.4999 → must floor to 0 with Neutral.
    #[test]
    fn mutation_kill_neutral_does_not_always_round_up() {
        // 0.4999 → should truncate to 0 with Neutral (< 0.5)
        let r = SafeMath::percentage(1, 4_999, RoundingMode::Neutral).unwrap();
        assert_eq!(r, 0, "Neutral must NOT round up when remainder < half");
    }

    /// Mutant: `UserFavor` branch does not add 1 when there is a remainder.
    /// Kill: 1001 * 100 / 10_000 = 10.01 → ceiling = 11.
    #[test]
    fn mutation_kill_user_favor_is_ceiling() {
        let r = SafeMath::percentage(1_001, 100, RoundingMode::UserFavor).unwrap();
        assert_eq!(r, 11, "UserFavor must ceiling (round up) the result");
    }

    /// Mutant: `UserFavor` adds 1 even when remainder == 0 (off-by-one).
    /// Kill: exact result must not be incremented.
    #[test]
    fn mutation_kill_user_favor_no_increment_on_exact() {
        // 1000 * 100 / 10_000 = 10 exactly → no increment
        let r = SafeMath::percentage(1_000, 100, RoundingMode::UserFavor).unwrap();
        assert_eq!(
            r, 10,
            "UserFavor must not round up when there is no remainder"
        );
    }

    // ── SafeMath::calculate_share ─────────────────────────────────────────────

    /// Mutant: `user_stake > winning_stake` → `user_stake >= winning_stake`
    /// Kill: user_stake == winning_stake (sole winner) must be accepted.
    #[test]
    fn mutation_kill_share_equal_is_valid() {
        let r = SafeMath::calculate_share(1_000, 1_000, 5_000).unwrap();
        assert_eq!(r, 5_000, "sole winner receives full payout pool");
    }

    /// Mutant: `product / winning_stake` → `product * winning_stake`.
    /// Kill: proportional share of partial stake must be a fraction of pool.
    #[test]
    fn mutation_kill_share_division() {
        // 200 / 1000 * 5000 = 1000
        let r = SafeMath::calculate_share(200, 1_000, 5_000).unwrap();
        assert_eq!(r, 1_000, "20% stake → 20% of payout pool");
    }

    /// Mutant: `user_stake == 0` short-circuit removed.
    #[test]
    fn mutation_kill_share_zero_user_stake_is_zero() {
        let r = SafeMath::calculate_share(0, 1_000, 5_000).unwrap();
        assert_eq!(r, 0);
    }

    /// Mutant: `winning_stake == 0` short-circuit removed.
    #[test]
    fn mutation_kill_share_zero_winning_stake_is_zero() {
        let r = SafeMath::calculate_share(100, 0, 5_000).unwrap();
        assert_eq!(r, 0);
    }

    /// Mutant: `user_stake > winning_stake` guard removed.
    #[test]
    fn mutation_kill_share_user_exceeds_winning_rejected() {
        assert_eq!(
            SafeMath::calculate_share(1_001, 1_000, 5_000),
            Err(PrediFiError::ArithmeticError)
        );
    }

    // ── SafeMath::safe_add / safe_sub ─────────────────────────────────────────

    /// Mutant: `checked_add` → `checked_sub` (or unchecked).
    /// Kill: simple sum must be correct.
    #[test]
    fn mutation_kill_safe_add_is_correct() {
        let r = SafeMath::safe_add(300, 700).unwrap();
        assert_eq!(r, 1_000);
    }

    /// Mutant: overflow guard removed.
    /// Kill: near-overflow inputs must produce an error, not wrap.
    #[test]
    fn mutation_kill_safe_add_overflow_is_error() {
        assert_eq!(
            SafeMath::safe_add(i128::MAX, 1),
            Err(PrediFiError::ArithmeticError)
        );
    }

    /// Mutant: `checked_sub` → `checked_add`.
    /// Kill: simple subtraction must be correct.
    #[test]
    fn mutation_kill_safe_sub_is_correct() {
        let r = SafeMath::safe_sub(1_000, 300).unwrap();
        assert_eq!(r, 700);
    }

    /// Mutant: underflow guard removed.
    /// Note: `0 - 1` is a valid `i128` (-1); underflow only happens below `i128::MIN`.
    #[test]
    fn mutation_kill_safe_sub_underflow_is_error() {
        assert_eq!(SafeMath::safe_sub(0, 1), Ok(-1));
        assert_eq!(
            SafeMath::safe_sub(i128::MIN, 1),
            Err(PrediFiError::ArithmeticError)
        );
    }

    // ── SafeMath::safe_mul ────────────────────────────────────────────────────

    /// Mutant: `checked_mul` → `checked_add`.
    #[test]
    fn mutation_kill_safe_mul_is_correct() {
        let r = SafeMath::safe_mul(7, 6).unwrap();
        assert_eq!(r, 42);
    }

    /// Mutant: overflow guard removed.
    #[test]
    fn mutation_kill_safe_mul_overflow_is_error() {
        assert_eq!(
            SafeMath::safe_mul(i128::MAX, 2),
            Err(PrediFiError::InvalidAmount)
        );
    }
}
