//! # Multi-source oracle quorum resolution (issue #22)
//!
//! An adapter strategy that compares independent price-feed observations and
//! produces a resolution value **only** when the configured agreement
//! requirements are met. It is a pure `no_std` function with no storage, so the
//! contract can adopt it per market without touching the existing single-source
//! path.
//!
//! ## Specification
//!
//! | Concern          | Rule                                                                 |
//! |------------------|----------------------------------------------------------------------|
//! | Source identity  | Every observation names a source from the configured allowlist. An unknown source or a second observation from the same source rejects the whole batch (`UnknownSource` / `DuplicateSource`), so one provider can never be counted twice. |
//! | Freshness        | An observation is fresh when `now - observed_at <= max_age_secs`. Observations from the future (`observed_at > now`) are treated as stale. Stale observations are discarded, never averaged in. |
//! | Quorum           | `min_agreeing` must be a strict majority of the configured sources and at least 2 (`InvalidConfig` otherwise). Fewer fresh observations than `min_agreeing` fails with `InsufficientFreshSources`. |
//! | Disagreement     | The reference point is the lower median of fresh values. A fresh value agrees when `|v - median| * 10_000 <= max_deviation_bps * median`; others are outliers. Fewer agreeing values than `min_agreeing` fails with `InsufficientAgreement` — the market stays unresolved. |
//! | Result           | The lower median of the agreeing values.                            |
//!
//! Because the quorum is a strict majority, a single stale or outlier source can
//! neither satisfy the quorum on its own nor move the median outside the range
//! spanned by agreeing honest sources.
//!
//! ## Backward compatibility
//!
//! The existing single-source flow (`resolve_pool_from_price` with one
//! whitelisted oracle) is unchanged; this crate is not wired into it. Switching a
//! market to quorum resolution requires an explicitly approved migration that
//! stores a [`QuorumConfig`] per market and routes resolution through
//! [`resolve`].

#![no_std]

/// Upper bound on configured sources (keeps the sort on a fixed stack array).
pub const MAX_SOURCES: usize = 16;

/// Basis-point denominator.
pub const BPS_DENOMINATOR: u32 = 10_000;

/// Largest accepted observation value; keeps `value * 10_000` within `i128`.
pub const MAX_VALUE: i128 = i128::MAX / BPS_DENOMINATOR as i128;

/// One reading from one feed provider, already normalized to the market's
/// fixed-point price units.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation<S> {
    pub source: S,
    pub value: i128,
    pub observed_at: u64,
}

/// Per-market agreement requirements.
#[derive(Clone, Copy, Debug)]
pub struct QuorumConfig<'a, S> {
    /// Allowlist of independent feed providers.
    pub sources: &'a [S],
    /// Minimum number of fresh, mutually agreeing sources.
    pub min_agreeing: u32,
    /// Maximum observation age in seconds.
    pub max_age_secs: u64,
    /// Maximum deviation from the median, in basis points, to count as agreeing.
    pub max_deviation_bps: u32,
}

/// Successful quorum resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuorumOutcome {
    /// Resolved value (lower median of the agreeing set).
    pub value: i128,
    /// Number of sources that agreed.
    pub agreeing: u32,
    /// Observations discarded as stale (or from the future).
    pub stale: u32,
    /// Fresh observations discarded as outliers.
    pub outliers: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuorumError {
    /// Allowlist empty, too large, duplicated, or `min_agreeing` not a strict
    /// majority of at least 2.
    InvalidConfig,
    /// Observation from a source not in the allowlist.
    UnknownSource,
    /// More than one observation from the same source.
    DuplicateSource,
    /// Observation value is `<= 0` or above [`MAX_VALUE`].
    InvalidValue,
    /// Not enough fresh observations to reach quorum.
    InsufficientFreshSources { fresh: u32, required: u32 },
    /// Fresh observations disagree; not enough of them agree to reach quorum.
    InsufficientAgreement { agreeing: u32, required: u32 },
}

impl<S: PartialEq> QuorumConfig<'_, S> {
    /// Validate the configuration on its own (used at configuration time and
    /// again on every resolution).
    pub fn validate(&self) -> Result<(), QuorumError> {
        let n = self.sources.len();
        if n == 0 || n > MAX_SOURCES || self.max_deviation_bps > BPS_DENOMINATOR {
            return Err(QuorumError::InvalidConfig);
        }
        let min = self.min_agreeing as usize;
        if min < 2 || min > n || min * 2 <= n {
            return Err(QuorumError::InvalidConfig);
        }
        for (i, s) in self.sources.iter().enumerate() {
            if self.sources[..i].contains(s) {
                return Err(QuorumError::InvalidConfig);
            }
        }
        Ok(())
    }
}

/// Resolve a market value from independent observations at ledger time `now`.
pub fn resolve<S: PartialEq>(
    config: &QuorumConfig<'_, S>,
    observations: &[Observation<S>],
    now: u64,
) -> Result<QuorumOutcome, QuorumError> {
    config.validate()?;
    let required = config.min_agreeing;

    let mut fresh = [0i128; MAX_SOURCES];
    let mut fresh_len = 0usize;
    let mut stale = 0u32;

    for (i, obs) in observations.iter().enumerate() {
        if !config.sources.contains(&obs.source) {
            return Err(QuorumError::UnknownSource);
        }
        if observations[..i].iter().any(|o| o.source == obs.source) {
            return Err(QuorumError::DuplicateSource);
        }
        if obs.value <= 0 || obs.value > MAX_VALUE {
            return Err(QuorumError::InvalidValue);
        }
        let is_fresh = obs.observed_at <= now && now - obs.observed_at <= config.max_age_secs;
        if is_fresh {
            // Bounded: sources are unique and allowlisted, allowlist <= MAX_SOURCES.
            fresh[fresh_len] = obs.value;
            fresh_len += 1;
        } else {
            stale += 1;
        }
    }

    if (fresh_len as u32) < required {
        return Err(QuorumError::InsufficientFreshSources {
            fresh: fresh_len as u32,
            required,
        });
    }

    let fresh = &mut fresh[..fresh_len];
    fresh.sort_unstable();
    let median = lower_median(fresh);

    let mut agreeing = [0i128; MAX_SOURCES];
    let mut agreeing_len = 0usize;
    for &v in fresh.iter() {
        // Both sides fit in i128: values are capped at MAX_VALUE.
        let deviation = (v - median).abs() * BPS_DENOMINATOR as i128;
        if deviation <= config.max_deviation_bps as i128 * median {
            agreeing[agreeing_len] = v;
            agreeing_len += 1;
        }
    }

    if (agreeing_len as u32) < required {
        return Err(QuorumError::InsufficientAgreement {
            agreeing: agreeing_len as u32,
            required,
        });
    }

    // `fresh` was sorted, so `agreeing` is sorted too.
    Ok(QuorumOutcome {
        value: lower_median(&agreeing[..agreeing_len]),
        agreeing: agreeing_len as u32,
        stale,
        outliers: (fresh_len - agreeing_len) as u32,
    })
}

fn lower_median(sorted: &[i128]) -> i128 {
    sorted[(sorted.len() - 1) / 2]
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    const NOW: u64 = 1_000_000;
    const PRICE: i128 = 500_000_000_000; // 50_000 with 7 decimals

    /// A mock feed provider: reports a fixed price observed `lag` seconds ago.
    #[derive(Clone, Copy)]
    struct MockFeed {
        id: u32,
        price: i128,
        lag: u64,
    }

    impl MockFeed {
        fn honest(id: u32) -> Self {
            Self {
                id,
                price: PRICE,
                lag: 10,
            }
        }
        fn priced(id: u32, price: i128) -> Self {
            Self {
                price,
                ..Self::honest(id)
            }
        }
        fn lagging(id: u32, lag: u64) -> Self {
            Self {
                lag,
                ..Self::honest(id)
            }
        }
        fn observe(&self, now: u64) -> Observation<u32> {
            Observation {
                source: self.id,
                value: self.price,
                observed_at: now - self.lag,
            }
        }
    }

    fn observe_all(feeds: &[MockFeed]) -> Vec<Observation<u32>> {
        feeds.iter().map(|f| f.observe(NOW)).collect()
    }

    fn config(sources: &[u32], min_agreeing: u32) -> QuorumConfig<'_, u32> {
        QuorumConfig {
            sources,
            min_agreeing,
            max_age_secs: 60,
            max_deviation_bps: 100, // 1%
        }
    }

    const SOURCES: [u32; 3] = [1, 2, 3];

    #[test]
    fn all_sources_agree_resolves_to_median() {
        let feeds = [
            MockFeed::priced(1, PRICE - 10),
            MockFeed::honest(2),
            MockFeed::priced(3, PRICE + 10),
        ];
        let out = resolve(&config(&SOURCES, 2), &observe_all(&feeds), NOW).unwrap();
        assert_eq!(out.value, PRICE);
        assert_eq!((out.agreeing, out.stale, out.outliers), (3, 0, 0));
    }

    #[test]
    fn single_outlier_cannot_determine_result() {
        for outlier_price in [1, PRICE / 2, PRICE * 2, MAX_VALUE] {
            let feeds = [
                MockFeed::honest(1),
                MockFeed::priced(2, PRICE + 5),
                MockFeed::priced(3, outlier_price),
            ];
            let out = resolve(&config(&SOURCES, 2), &observe_all(&feeds), NOW).unwrap();
            assert_eq!(out.value, PRICE, "outlier {outlier_price} moved the result");
            assert_eq!((out.agreeing, out.outliers), (2, 1));
        }
    }

    #[test]
    fn single_stale_source_is_ignored() {
        let feeds = [
            MockFeed::honest(1),
            MockFeed::honest(2),
            MockFeed {
                lag: 61,
                ..MockFeed::priced(3, PRICE * 3)
            },
        ];
        let out = resolve(&config(&SOURCES, 2), &observe_all(&feeds), NOW).unwrap();
        assert_eq!(out.value, PRICE);
        assert_eq!((out.agreeing, out.stale), (2, 1));
    }

    #[test]
    fn freshness_boundary_is_inclusive() {
        let feeds = [MockFeed::lagging(1, 60), MockFeed::lagging(2, 60)];
        let out = resolve(&config(&SOURCES, 2), &observe_all(&feeds), NOW).unwrap();
        assert_eq!(out.agreeing, 2);
    }

    #[test]
    fn future_observation_is_treated_as_stale() {
        let mut obs = observe_all(&[MockFeed::honest(1), MockFeed::honest(2)]);
        obs[1].observed_at = NOW + 1;
        assert_eq!(
            resolve(&config(&SOURCES, 2), &obs, NOW),
            Err(QuorumError::InsufficientFreshSources {
                fresh: 1,
                required: 2
            })
        );
    }

    #[test]
    fn lone_fresh_source_cannot_resolve() {
        let feeds = [
            MockFeed::honest(1),
            MockFeed::lagging(2, 600),
            MockFeed::lagging(3, 600),
        ];
        assert_eq!(
            resolve(&config(&SOURCES, 2), &observe_all(&feeds), NOW),
            Err(QuorumError::InsufficientFreshSources {
                fresh: 1,
                required: 2
            })
        );
    }

    #[test]
    fn insufficient_quorum_when_sources_missing() {
        assert_eq!(
            resolve(&config(&SOURCES, 2), &[], NOW),
            Err(QuorumError::InsufficientFreshSources {
                fresh: 0,
                required: 2
            })
        );
        let five = [1, 2, 3, 4, 5];
        let feeds = [MockFeed::honest(1), MockFeed::honest(2)];
        assert_eq!(
            resolve(&config(&five, 3), &observe_all(&feeds), NOW),
            Err(QuorumError::InsufficientFreshSources {
                fresh: 2,
                required: 3
            })
        );
    }

    #[test]
    fn total_disagreement_does_not_resolve() {
        let feeds = [
            MockFeed::priced(1, PRICE),
            MockFeed::priced(2, PRICE * 2),
            MockFeed::priced(3, PRICE * 4),
        ];
        assert_eq!(
            resolve(&config(&SOURCES, 2), &observe_all(&feeds), NOW),
            Err(QuorumError::InsufficientAgreement {
                agreeing: 1,
                required: 2
            })
        );
    }

    #[test]
    fn split_clusters_below_quorum_do_not_resolve() {
        // 5 sources, quorum 3: a 2-vs-2 split plus one stale source.
        let five = [1, 2, 3, 4, 5];
        let feeds = [
            MockFeed::honest(1),
            MockFeed::honest(2),
            MockFeed::priced(3, PRICE * 2),
            MockFeed::priced(4, PRICE * 2),
            MockFeed::lagging(5, 3600),
        ];
        assert_eq!(
            resolve(&config(&five, 3), &observe_all(&feeds), NOW),
            Err(QuorumError::InsufficientAgreement {
                agreeing: 2,
                required: 3
            })
        );
    }

    #[test]
    fn deviation_boundary_is_inclusive() {
        // 1% of PRICE is exactly on the boundary; one unit beyond is an outlier.
        let on_edge = PRICE + PRICE / 100;
        let feeds = [
            MockFeed::priced(1, on_edge),
            MockFeed::honest(2),
            MockFeed::priced(3, on_edge + 1),
        ];
        // Sorted: [PRICE, on_edge, on_edge + 1] → median on_edge; all within 1%.
        let out = resolve(&config(&SOURCES, 2), &observe_all(&feeds), NOW).unwrap();
        assert_eq!(out.agreeing, 3);

        let feeds = [
            MockFeed::honest(1),
            MockFeed::honest(2),
            MockFeed::priced(3, on_edge + 1),
        ];
        let out = resolve(&config(&SOURCES, 2), &observe_all(&feeds), NOW).unwrap();
        assert_eq!((out.value, out.outliers), (PRICE, 1));
    }

    #[test]
    fn source_identity_is_enforced() {
        let unknown = [MockFeed::honest(1), MockFeed::honest(9)];
        assert_eq!(
            resolve(&config(&SOURCES, 2), &observe_all(&unknown), NOW),
            Err(QuorumError::UnknownSource)
        );
        // One provider submitting twice must not count as two votes.
        let dup = [MockFeed::honest(1), MockFeed::honest(1)];
        assert_eq!(
            resolve(&config(&SOURCES, 2), &observe_all(&dup), NOW),
            Err(QuorumError::DuplicateSource)
        );
    }

    #[test]
    fn invalid_values_are_rejected() {
        for bad in [0, -1, MAX_VALUE + 1] {
            let feeds = [MockFeed::honest(1), MockFeed::priced(2, bad)];
            assert_eq!(
                resolve(&config(&SOURCES, 2), &observe_all(&feeds), NOW),
                Err(QuorumError::InvalidValue)
            );
        }
    }

    #[test]
    fn config_requires_strict_majority_of_at_least_two() {
        assert_eq!(
            config(&SOURCES, 1).validate(),
            Err(QuorumError::InvalidConfig)
        );
        assert_eq!(
            config(&SOURCES, 4).validate(),
            Err(QuorumError::InvalidConfig)
        );
        assert_eq!(
            config(&[1, 2, 3, 4], 2).validate(),
            Err(QuorumError::InvalidConfig)
        );
        assert_eq!(
            config(&[1, 1, 2], 2).validate(),
            Err(QuorumError::InvalidConfig)
        );
        assert_eq!(config(&[], 0).validate(), Err(QuorumError::InvalidConfig));
        let too_many = [0u32; MAX_SOURCES + 1];
        assert_eq!(
            config(&too_many, 9).validate(),
            Err(QuorumError::InvalidConfig)
        );
        assert_eq!(config(&SOURCES, 2).validate(), Ok(()));
        assert_eq!(config(&[1, 2, 3, 4], 3).validate(), Ok(()));
    }

    #[test]
    fn resolution_is_independent_of_observation_order() {
        let feeds = [
            MockFeed::priced(1, PRICE + 3),
            MockFeed::priced(2, PRICE - 7),
            MockFeed::priced(3, PRICE * 9),
        ];
        let mut obs = observe_all(&feeds);
        let expected = resolve(&config(&SOURCES, 2), &obs, NOW).unwrap();
        obs.reverse();
        assert_eq!(resolve(&config(&SOURCES, 2), &obs, NOW).unwrap(), expected);
        obs.swap(0, 1);
        assert_eq!(resolve(&config(&SOURCES, 2), &obs, NOW).unwrap(), expected);
    }
}
