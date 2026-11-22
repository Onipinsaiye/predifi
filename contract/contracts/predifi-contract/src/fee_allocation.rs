//! Deterministic LP fee attribution (issue #276).
//!
//! Protocol fees accrue into a pool-wide ledger.  This module splits that
//! accrued total across providers so that:
//!
//! 1. the split depends only on each provider's share balance and a canonical
//!    provider key — never on the order providers happen to be iterated in;
//! 2. every unit of collected fee is allocated (the integer-division dust is
//!    assigned via the largest-remainder method with a deterministic tie-break
//!    on the provider key), so `sum(allocations) == total_fees` exactly;
//! 3. a provider's claimable amount can be read back without recomputing.
//!
//! The function is generic over an `Ord` key so the on-chain path can use
//! `soroban_sdk::Address` while native/WASM property tests can use integers.

use alloc::vec::Vec;

/// Input for one provider in a fee allocation round.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderInput<K> {
    /// Canonical, unique key used to break remainder ties deterministically.
    pub key: K,
    /// Provider's share balance at the time accrued fees are attributed.
    pub shares: i128,
}

/// A provider's deterministic share of the accrued fees.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeAllocation<K> {
    pub key: K,
    pub amount: i128,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeeAllocationError {
    /// `total_fees` is negative.
    InvalidAmount,
    /// Total provider shares are zero or negative.
    NoShares,
    /// An intermediate product overflowed `i128`.
    Overflow,
}

/// Allocate `total_fees` across providers proportionally to their shares.
///
/// Returns allocations ordered by `key` ascending, independent of the order of
/// `providers`.  The sum of all returned amounts equals `total_fees` exactly.
pub fn allocate_fees<K: Ord + Clone>(
    total_fees: i128,
    providers: &[ProviderInput<K>],
) -> Result<Vec<FeeAllocation<K>>, FeeAllocationError> {
    if total_fees < 0 {
        return Err(FeeAllocationError::InvalidAmount);
    }
    if providers.is_empty() {
        return if total_fees == 0 {
            Ok(Vec::new())
        } else {
            Err(FeeAllocationError::NoShares)
        };
    }

    let mut total_shares: i128 = 0;
    for provider in providers {
        if provider.shares < 0 {
            return Err(FeeAllocationError::InvalidAmount);
        }
        total_shares = total_shares
            .checked_add(provider.shares)
            .ok_or(FeeAllocationError::Overflow)?;
    }
    if total_shares <= 0 {
        return Err(FeeAllocationError::NoShares);
    }

    let mut amounts: Vec<i128> = Vec::with_capacity(providers.len());
    let mut dust: Vec<(i128, K)> = Vec::with_capacity(providers.len());
    for provider in providers {
        let scaled = total_fees
            .checked_mul(provider.shares)
            .ok_or(FeeAllocationError::Overflow)?;
        amounts.push(scaled / total_shares);
        dust.push((scaled % total_shares, provider.key.clone()));
    }

    // Sum of floors can be below the total by at most (n - 1) units. Assign
    // those units by largest fractional remainder, breaking ties by key.
    let allocated: i128 = amounts.iter().copied().sum();
    let mut unallocated = total_fees - allocated;
    debug_assert!(unallocated >= 0 && unallocated <= providers.len() as i128);

    let mut order: Vec<usize> = (0..dust.len()).collect();
    order.sort_by(|&a, &b| dust[b].0.cmp(&dust[a].0).then_with(|| dust[a].1.cmp(&dust[b].1)));

    for index in order {
        if unallocated == 0 {
            break;
        }
        amounts[index] += 1;
        unallocated -= 1;
    }

    let mut result: Vec<FeeAllocation<K>> = providers
        .iter()
        .zip(amounts)
        .map(|(provider, amount)| FeeAllocation {
            key: provider.key.clone(),
            amount,
        })
        .collect();
    result.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(result)
}

/// Sum of all allocations (useful for conservation checks).
pub fn total_allocated<K>(allocations: &[FeeAllocation<K>]) -> i128 {
    allocations.iter().map(|a| a.amount).sum()
}

/// Claimable amount for one provider in an allocation result.
pub fn claimable_for<K: Eq>(allocations: &[FeeAllocation<K>], key: &K) -> i128 {
    allocations
        .iter()
        .find(|a| &a.key == key)
        .map(|a| a.amount)
        .unwrap_or(0)
}
