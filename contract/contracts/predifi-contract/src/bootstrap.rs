//! Pool bootstrap depth limits (issue #275).
//!
//! A freshly created pool is bootstrapped with an optional initial liquidity
//! amount.  Dust-level bootstrap values produce meaningless quotes, while an
//! unbounded amount can overflow share-mint arithmetic and breach the
//! per-pool aggregate stake ceiling.  This module holds the pure validation
//! rule so it can be exercised identically on native and WASM builds and
//! reused by every pool-creation entry point.

use crate::{MAX_POOL_BOOTSTRAP_DEPTH, MIN_POOL_BOOTSTRAP_DEPTH};

/// Stable, typed failures for an invalid bootstrap amount.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootstrapError {
    /// Amount is negative (or below the configured positive minimum).
    DepthTooLow,
    /// Amount exceeds the protocol's maximum bootstrap depth.
    DepthTooHigh,
}

/// Validate an `initial_liquidity` value at pool creation.
///
/// `0` is intentionally accepted: it means "create the pool without seeded
/// liquidity", a supported state where the first LP deposit bootstraps the
/// share price.  Any positive amount must be at least
/// [`MIN_POOL_BOOTSTRAP_DEPTH`], and no amount may exceed
/// [`MAX_POOL_BOOTSTRAP_DEPTH`].
///
/// The rule is a `const fn` with no host dependencies, so native and optimised
/// WASM builds evaluate exactly the same comparisons and produce the same
/// typed error for every input.
pub const fn validate_bootstrap_depth(amount: i128) -> Result<(), BootstrapError> {
    if amount < 0 || (amount > 0 && amount < MIN_POOL_BOOTSTRAP_DEPTH) {
        Err(BootstrapError::DepthTooLow)
    } else if amount > MAX_POOL_BOOTSTRAP_DEPTH {
        Err(BootstrapError::DepthTooHigh)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_zero_and_the_smallest_representable_depth() {
        assert_eq!(validate_bootstrap_depth(0), Ok(()));
        assert_eq!(validate_bootstrap_depth(MIN_POOL_BOOTSTRAP_DEPTH), Ok(()));
    }

    #[test]
    fn rejects_just_below_the_boundary() {
        assert_eq!(
            validate_bootstrap_depth(MIN_POOL_BOOTSTRAP_DEPTH - 1),
            Err(BootstrapError::DepthTooLow)
        );
        assert_eq!(validate_bootstrap_depth(-1), Err(BootstrapError::DepthTooLow));
    }

    #[test]
    fn accepts_the_maximum_and_rejects_one_above() {
        assert_eq!(validate_bootstrap_depth(MAX_POOL_BOOTSTRAP_DEPTH), Ok(()));
        assert_eq!(
            validate_bootstrap_depth(MAX_POOL_BOOTSTRAP_DEPTH + 1),
            Err(BootstrapError::DepthTooHigh)
        );
    }
}
