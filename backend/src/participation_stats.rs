use std::collections::HashMap;

/// Minimum number of wallets a cohort must contain before its aggregate
/// statistics may be published. Cohorts below this threshold are suppressed
/// to protect individual privacy.
pub const DEFAULT_PRIVACY_THRESHOLD: usize = 5;

/// A single wallet-level participation record. This is internal data and must
/// never be exposed through public APIs.
#[derive(Debug, Clone)]
pub struct WalletParticipation {
    /// Opaque wallet identifier. Kept private; never serialized publicly.
    pub wallet_id: String,
    /// Cohort the wallet belongs to (e.g. region, tier, campaign).
    pub cohort: String,
    /// Number of participation events recorded for this wallet.
    pub participation_count: u64,
    /// Total value contributed by this wallet (e.g. points, tokens).
    pub contribution_value: u64,
}

/// Aggregate statistics for a single cohort, safe for public consumption.
/// Contains no wallet identifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CohortAggregate {
    /// Cohort label.
    pub cohort: String,
    /// Number of wallets included in this aggregate (the denominator).
    pub wallet_count: usize,
    /// Sum of participation events across the cohort.
    pub total_participation_count: u64,
    /// Sum of contribution value across the cohort.
    pub total_contribution_value: u64,
    /// Mean participation count per wallet (integer division).
    pub mean_participation_count: u64,
    /// Mean contribution value per wallet (integer division).
    pub mean_contribution_value: u64,
}

/// Public statistics payload. Contains only thresholded aggregates plus
/// documented denominators and the sampling timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicParticipationStats {
    /// Unix timestamp (seconds) at which the underlying data was sampled.
    pub sampled_at_unix: u64,
    /// Privacy threshold applied when producing these aggregates.
    pub privacy_threshold: usize,
    /// Total number of wallets considered before suppression.
    pub total_wallets_sampled: usize,
    /// Total number of wallets included after suppression.
    pub total_wallets_published: usize,
    /// Thresholded cohort aggregates. Suppressed cohorts are omitted.
    pub cohorts: Vec<CohortAggregate>,
}

/// Computes thresholded cohort aggregates from internal wallet-level data.
///
/// Cohorts with fewer than `privacy_threshold` wallets are suppressed and do
/// not appear in the output. The returned payload contains no wallet
/// identifiers.
pub fn compute_public_stats(
    records: &[WalletParticipation],
    privacy_threshold: usize,
    sampled_at_unix: u64,
) -> PublicParticipationStats {
    let mut by_cohort: HashMap<&str, Vec<&WalletParticipation>> = HashMap::new();
    for record in records {
        by_cohort
            .entry(record.cohort.as_str())
            .or_default()
            .push(record);
    }

    // Sort cohort names for deterministic, reproducible output.
    let mut cohort_names: Vec<&str> = by_cohort.keys().copied().collect();
    cohort_names.sort_unstable();

    let mut cohorts: Vec<CohortAggregate> = Vec::new();
    let mut total_wallets_published = 0usize;

    for name in cohort_names {
        let members = &by_cohort[name];
        if members.len() < privacy_threshold {
            // Suppress small cohorts.
            continue;
        }

        let wallet_count = members.len();
        let total_participation_count: u64 =
            members.iter().map(|m| m.participation_count).sum();
        let total_contribution_value: u64 =
            members.iter().map(|m| m.contribution_value).sum();

        let mean_participation_count = total_participation_count / wallet_count as u64;
        let mean_contribution_value = total_contribution_value / wallet_count as u64;

        total_wallets_published += wallet_count;

        cohorts.push(CohortAggregate {
            cohort: name.to_string(),
            wallet_count,
            total_participation_count,
            total_contribution_value,
            mean_participation_count,
            mean_contribution_value,
        });
    }

    PublicParticipationStats {
        sampled_at_unix,
        privacy_threshold,
        total_wallets_sampled: records.len(),
        total_wallets_published,
        cohorts,
    }
}

/// Public read function. Returns only thresholded aggregates and never
/// exposes wallet identifiers.
pub fn read_public_stats(
    records: &[WalletParticipation],
    sampled_at_unix: u64,
) -> PublicParticipationStats {
    compute_public_stats(records, DEFAULT_PRIVACY_THRESHOLD, sampled_at_unix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(wallet: &str, cohort: &str, count: u64, value: u64) -> WalletParticipation {
        WalletParticipation {
            wallet_id: wallet.to_string(),
            cohort: cohort.to_string(),
            participation_count: count,
            contribution_value: value,
        }
    }

    #[test]
    fn suppresses_small_cohorts() {
        let records = vec![
            // Large cohort (5 wallets) - should be published.
            record("w1", "large", 1, 10),
            record("w2", "large", 2, 20),
            record("w3", "large", 3, 30),
            record("w4", "large", 4, 40),
            record("w5", "large", 5, 50),
            // Small cohort (2 wallets) - should be suppressed.
            record("w6", "small", 100, 1000),
            record("w7", "small", 200, 2000),
        ];

        let stats = compute_public_stats(&records, 5, 1_700_000_000);

        assert_eq!(stats.cohorts.len(), 1);
        assert_eq!(stats.cohorts[0].cohort, "large");
        assert_eq!(stats.cohorts[0].wallet_count, 5);
        assert_eq!(stats.total_wallets_sampled, 7);
        assert_eq!(stats.total_wallets_published, 5);
        assert_eq!(stats.privacy_threshold, 5);
        assert_eq!(stats.sampled_at_unix, 1_700_000_000);
    }

    #[test]
    fn suppresses_cohort_exactly_below_threshold() {
        let records = vec![
            record("w1", "a", 1, 1),
            record("w2", "a", 1, 1),
            record("w3", "a", 1, 1),
            record("w4", "a", 1, 1),
        ];
        let stats = compute_public_stats(&records, 5, 0);
        assert!(stats.cohorts.is_empty());
        assert_eq!(stats.total_wallets_published, 0);
    }

    #[test]
    fn aggregation_is_reproducible() {
        let records = vec![
            record("w1", "b", 1, 10),
            record("w2", "a", 2, 20),
            record("w3", "b", 3, 30),
            record("w4", "a", 4, 40),
            record("w5", "a", 5, 50),
            record("w6", "b", 6, 60),
            record("w7", "a", 7, 70),
            record("w8", "b", 8, 80),
            record("w9", "a", 9, 90),
            record("w10", "b", 10, 100),
        ];

        let first = compute_public_stats(&records, 5, 42);
        let second = compute_public_stats(&records, 5, 42);
        assert_eq!(first, second);

        // Order of input records must not affect output.
        let mut shuffled = records.clone();
        shuffled.reverse();
        let third = compute_public_stats(&shuffled, 5, 42);
        assert_eq!(first, third);

        // Verify aggregate math for cohort "a": wallets w2,w4,w5,w7,w9.
        let cohort_a = first
            .cohorts
            .iter()
            .find(|c| c.cohort == "a")
            .expect("cohort a present");
        assert_eq!(cohort_a.wallet_count, 5);
        assert_eq!(cohort_a.total_participation_count, 2 + 4 + 5 + 7 + 9);
        assert_eq!(cohort_a.total_contribution_value, 20 + 40 + 50 + 70 + 90);
        assert_eq!(cohort_a.mean_participation_count, (2 + 4 + 5 + 7 + 9) / 5);
        assert_eq!(cohort_a.mean_contribution_value, (20 + 40 + 50 + 70 + 90) / 5);
    }

    #[test]
    fn responses_contain_no_wallet_identifiers() {
        let records = vec![
            record("secret-wallet-1", "c", 1, 10),
            record("secret-wallet-2", "c", 2, 20),
            record("secret-wallet-3", "c", 3, 30),
            record("secret-wallet-4", "c", 4, 40),
            record("secret-wallet-5", "c", 5, 50),
        ];

        let stats = read_public_stats(&records, 123);
        let debug_repr = format!("{:?}", stats);

        for r in &records {
            assert!(
                !debug_repr.contains(&r.wallet_id),
                "public stats leaked wallet id {}",
                r.wallet_id
            );
        }
        assert!(!debug_repr.contains("wallet_id"));
    }

    #[test]
    fn read_public_stats_uses_default_threshold() {
        let records = vec![
            record("w1", "x", 1, 1),
            record("w2", "x", 1, 1),
            record("w3", "x", 1, 1),
            record("w4", "x", 1, 1),
            record("w5", "x", 1, 1),
        ];
        let stats = read_public_stats(&records, 7);
        assert_eq!(stats.privacy_threshold, DEFAULT_PRIVACY_THRESHOLD);
        assert_eq!(stats.cohorts.len(), 1);
        assert_eq!(stats.sampled_at_unix, 7);
    }
}