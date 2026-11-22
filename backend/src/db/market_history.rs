//! Historical market-value time series, sourced from canonical indexed
//! activity (#313), with a hot/cold retention tier for old samples (#314).
//!
//! ## Contract
//!
//! * **Bucket widths** are exactly one of [`BucketWidth`]: `1m`, `5m`, `1h`,
//!   `1d` — matching the cadence at which `prediction_placed` activity is
//!   already indexed (per-event, timestamped) rather than an arbitrary
//!   resolution invented for charting.
//! * **Timestamp semantics**: every sample's `bucket_start` is the
//!   *beginning* of its interval, always UTC, floored to an exact multiple
//!   of the bucket width since the Unix epoch (see [`floor_to_bucket`]) —
//!   there is no local-time conversion anywhere in this module, so bucket
//!   boundaries never shift around a daylight-saving transition in any
//!   caller's timezone. `value` is the pool's cumulative staked total as of
//!   the *end* of that interval (`bucket_start + width`).
//! * **Missing-sample behavior**: *carry-forward*. A bucket with no new
//!   activity reports the same cumulative value as the previous bucket —
//!   never omitted, never null. This falls directly out of the
//!   cumulative-sum semantics above; see the `sparse_intervals_*` tests.
//! * **Maximum query range**: [`MAX_BUCKETS_PER_QUERY`] buckets. A request
//!   whose `[from, to)` would produce more buckets than that is rejected
//!   with [`MarketHistoryError::RangeTooLarge`] — never silently truncated.
//! * Every sample carries the `checkpoint_ledger` it was derived from (the
//!   same committed-checkpoint cursor convention used by
//!   [`super::portfolio_snapshots`]) and its `value` is serialized as an
//!   exact decimal string, never a float.
//!
//! ## Source of truth
//!
//! Samples are aggregated from `portfolio_activity_ledger`
//! (`event_type = 'prediction_placed'`), the same canonical, checkpoint-gated
//! projection table backing the wallet ledger/snapshot/export APIs
//! (#309-#312) — this module introduces no separate price feed or
//! ingestion path.
//!
//! ## Hot/cold tiering (#314)
//!
//! Computed buckets are cached in `market_history_samples` (hot tier) so
//! repeat queries don't re-aggregate the ledger. [`super::market_history_archive::archive_market_history`]
//! moves old hot rows into a checksummed blob in `market_history_archives`
//! (cold tier); [`get_market_history`] transparently falls back to that
//! tier for any bucket missing from the hot tier, so callers see the exact
//! same response shape regardless of which tier (or both) served the data.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool};

use super::checkpoints::{load_checkpoint, CheckpointStatus, DEFAULT_LISTENER_ID};
use super::market_history_archive::lookup_archived_samples;

/// Explicit, supported bucket widths — matching this codebase's existing
/// per-event indexing cadence, not an arbitrary resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BucketWidth {
    OneMinute,
    FiveMinutes,
    OneHour,
    OneDay,
}

impl BucketWidth {
    /// Parse the wire form (`"1m"`, `"5m"`, `"1h"`, `"1d"`). Returns `None`
    /// for anything else — callers must treat an unrecognized width as a
    /// client error, never guess the closest supported one.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "1m" => Some(Self::OneMinute),
            "5m" => Some(Self::FiveMinutes),
            "1h" => Some(Self::OneHour),
            "1d" => Some(Self::OneDay),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OneMinute => "1m",
            Self::FiveMinutes => "5m",
            Self::OneHour => "1h",
            Self::OneDay => "1d",
        }
    }

    pub fn seconds(&self) -> i64 {
        match self {
            Self::OneMinute => 60,
            Self::FiveMinutes => 5 * 60,
            Self::OneHour => 3_600,
            Self::OneDay => 86_400,
        }
    }
}

/// Largest number of buckets a single query may request. Chosen so a
/// maximum-width query (`1d`) still covers multiple years while a
/// minimum-width query (`1m`) is bounded to a little under 1.5 days —
/// callers needing a wider `1m` view must page across multiple requests
/// rather than this endpoint silently truncating or running unbounded.
pub const MAX_BUCKETS_PER_QUERY: i64 = 2_000;

/// Floor `ts` to the start of its bucket, entirely in UTC epoch-second
/// arithmetic. This never touches local time, so a daylight-saving
/// transition in any timezone can never shift a bucket boundary.
pub fn floor_to_bucket(ts: DateTime<Utc>, width: BucketWidth) -> DateTime<Utc> {
    let width_secs = width.seconds();
    let secs = ts.timestamp();
    let floored = secs.div_euclid(width_secs) * width_secs;
    DateTime::<Utc>::from_timestamp(floored, 0).expect("floored unix timestamp is always valid")
}

/// Number of buckets `[from, to)` at `width` would produce (rounding the
/// partial leading/trailing bucket up), used to enforce
/// [`MAX_BUCKETS_PER_QUERY`] without materializing the bucket list.
pub fn bucket_count(from: DateTime<Utc>, to: DateTime<Utc>, width: BucketWidth) -> i64 {
    let width_secs = width.seconds();
    let total_secs = (to - floor_to_bucket(from, width)).num_seconds().max(0);
    (total_secs + width_secs - 1) / width_secs
}

/// The ordered list of bucket-start instants covering `[from, to)` at
/// `width`.
fn expected_bucket_starts(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    width: BucketWidth,
) -> Vec<DateTime<Utc>> {
    let step = ChronoDuration::seconds(width.seconds());
    let mut start = floor_to_bucket(from, width);
    let mut out = Vec::new();
    while start < to {
        out.push(start);
        start += step;
    }
    out
}

/// One raw `prediction_placed` activity row feeding the aggregation.
#[derive(Debug, Clone, Copy)]
pub struct RawStakeEvent {
    pub created_at: DateTime<Utc>,
    pub amount: i64,
}

/// Deterministically bucket `events` into cumulative-stake samples covering
/// `[from, to)`, carrying `baseline` (the cumulative total strictly before
/// `floor_to_bucket(from, width)`) forward.
///
/// `events` must be sorted ascending by `created_at` and already filtered
/// to `[floor_to_bucket(from, width), to)` — the caller (the DB-backed
/// aggregation in [`get_market_history`], or a test fixture) is responsible
/// for that filtering, which keeps this function pure and DB-free so it is
/// fully unit-testable (sparse intervals, DST-boundary buckets, hand
/// computed fixtures) without a database.
pub fn bucket_cumulative_stake(
    baseline: i64,
    events: &[RawStakeEvent],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    width: BucketWidth,
) -> Vec<(DateTime<Utc>, i64)> {
    let step = ChronoDuration::seconds(width.seconds());
    let mut bucket_start = floor_to_bucket(from, width);
    let mut running = baseline;
    let mut idx = 0usize;
    let mut out = Vec::new();

    while bucket_start < to {
        let bucket_end = bucket_start + step;
        while idx < events.len() && events[idx].created_at < bucket_end {
            running = running.saturating_add(events[idx].amount);
            idx += 1;
        }
        out.push((bucket_start, running));
        bucket_start = bucket_end;
    }
    out
}

/// One historical market-value sample.
#[derive(Debug, Clone, Serialize)]
pub struct MarketValueSample {
    /// Start of this bucket's interval, UTC.
    pub bucket_start: DateTime<Utc>,
    /// Cumulative value as of the end of this interval, as an exact
    /// decimal string (never a float).
    pub value: String,
    /// Committed listener checkpoint this sample was derived from.
    pub checkpoint_ledger: i64,
}

/// A page of ordered historical market-value samples.
#[derive(Debug, Clone, Serialize)]
pub struct MarketHistoryPage {
    pub pool_id: i64,
    pub bucket_width: String,
    pub samples: Vec<MarketValueSample>,
}

/// Why a market-history query could not be served.
#[derive(Debug)]
pub enum MarketHistoryError {
    /// `from >= to`.
    InvalidRange,
    /// The requested range would produce more buckets than
    /// [`MAX_BUCKETS_PER_QUERY`]; rejected rather than silently truncated.
    RangeTooLarge {
        max_buckets: i64,
        requested_buckets: i64,
    },
    /// The listener checkpoint is missing or malformed, so there is no safe
    /// ledger boundary to aggregate against.
    CheckpointNotReady(String),
    Database(sqlx::Error),
}

impl std::fmt::Display for MarketHistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarketHistoryError::InvalidRange => write!(f, "from must be before to"),
            MarketHistoryError::RangeTooLarge {
                max_buckets,
                requested_buckets,
            } => write!(
                f,
                "requested range spans {requested_buckets} buckets, exceeding the maximum of {max_buckets}"
            ),
            MarketHistoryError::CheckpointNotReady(reason) => {
                write!(f, "checkpoint not ready: {reason}")
            }
            MarketHistoryError::Database(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for MarketHistoryError {}

impl From<sqlx::Error> for MarketHistoryError {
    fn from(e: sqlx::Error) -> Self {
        MarketHistoryError::Database(e)
    }
}

#[derive(FromRow)]
struct HotRow {
    bucket_start: DateTime<Utc>,
    value: i64,
    checkpoint_ledger: i64,
}

/// Fetch a bounded, ordered page of historical market-value samples for
/// `pool_id` over `[from, to)`, bucketed at `width`.
///
/// Reads transparently span the hot tier (`market_history_samples`), the
/// cold tier (`market_history_archives`, see
/// [`super::market_history_archive`]), and — for buckets neither tier has
/// yet — a fresh aggregation straight from `portfolio_activity_ledger`,
/// which is then cached back into the hot tier. The response shape is
/// identical no matter which of these served a given sample.
pub async fn get_market_history(
    pool: &PgPool,
    pool_id: i64,
    width: BucketWidth,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<MarketHistoryPage, MarketHistoryError> {
    if from >= to {
        return Err(MarketHistoryError::InvalidRange);
    }
    let requested_buckets = bucket_count(from, to, width);
    if requested_buckets > MAX_BUCKETS_PER_QUERY {
        return Err(MarketHistoryError::RangeTooLarge {
            max_buckets: MAX_BUCKETS_PER_QUERY,
            requested_buckets,
        });
    }

    let checkpoint_ledger = match load_checkpoint(pool, DEFAULT_LISTENER_ID).await {
        CheckpointStatus::Valid(ledger) => ledger as i64,
        CheckpointStatus::Missing => {
            return Err(MarketHistoryError::CheckpointNotReady(
                "listener checkpoint not yet established".to_string(),
            ));
        }
        CheckpointStatus::Malformed(reason) => {
            return Err(MarketHistoryError::CheckpointNotReady(reason));
        }
    };

    let floor_from = floor_to_bucket(from, width);
    let all_bucket_starts = expected_bucket_starts(from, to, width);

    // 1. Hot tier.
    let hot_rows: Vec<HotRow> = sqlx::query_as(
        r#"
        SELECT bucket_start, value, checkpoint_ledger
        FROM market_history_samples
        WHERE pool_id = $1 AND bucket_width = $2 AND bucket_start >= $3 AND bucket_start < $4
        ORDER BY bucket_start ASC
        "#,
    )
    .bind(pool_id)
    .bind(width.as_str())
    .bind(floor_from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    let mut by_bucket: BTreeMap<DateTime<Utc>, (i64, i64)> = hot_rows
        .into_iter()
        .map(|r| (r.bucket_start, (r.value, r.checkpoint_ledger)))
        .collect();

    // 2. Cold tier, for anything the hot tier doesn't have.
    let missing_after_hot: Vec<DateTime<Utc>> = all_bucket_starts
        .iter()
        .copied()
        .filter(|b| !by_bucket.contains_key(b))
        .collect();
    if !missing_after_hot.is_empty() {
        let archived = lookup_archived_samples(pool, pool_id, width, floor_from, to).await?;
        for (bucket_start, value, ckpt) in archived {
            by_bucket.entry(bucket_start).or_insert((value, ckpt));
        }
    }

    // 3. Anything still missing has never been computed: aggregate fresh
    //    from the ledger and cache the result into the hot tier.
    let still_missing: Vec<DateTime<Utc>> = all_bucket_starts
        .iter()
        .copied()
        .filter(|b| !by_bucket.contains_key(b))
        .collect();
    if !still_missing.is_empty() {
        let baseline: i64 = sqlx::query_scalar(
            r#"
            SELECT COALESCE(SUM(amount), 0)::BIGINT
            FROM portfolio_activity_ledger
            WHERE pool_id = $1 AND event_type = 'prediction_placed'
              AND ledger <= $2 AND created_at < $3
            "#,
        )
        .bind(pool_id)
        .bind(checkpoint_ledger)
        .bind(floor_from)
        .fetch_one(pool)
        .await?;

        let raw_rows: Vec<(DateTime<Utc>, i64)> = sqlx::query_as(
            r#"
            SELECT created_at, amount
            FROM portfolio_activity_ledger
            WHERE pool_id = $1 AND event_type = 'prediction_placed'
              AND ledger <= $2 AND created_at >= $3 AND created_at < $4
            ORDER BY created_at ASC
            "#,
        )
        .bind(pool_id)
        .bind(checkpoint_ledger)
        .bind(floor_from)
        .bind(to)
        .fetch_all(pool)
        .await?;

        let events: Vec<RawStakeEvent> = raw_rows
            .into_iter()
            .map(|(created_at, amount)| RawStakeEvent { created_at, amount })
            .collect();
        let computed = bucket_cumulative_stake(baseline, &events, from, to, width);

        for (bucket_start, value) in &computed {
            by_bucket
                .entry(*bucket_start)
                .or_insert((*value, checkpoint_ledger));
        }

        materialize_hot_rows(pool, pool_id, width, &computed, checkpoint_ledger).await?;
    }

    let mut samples: Vec<MarketValueSample> = by_bucket
        .into_iter()
        .filter(|(bucket_start, _)| *bucket_start >= floor_from && *bucket_start < to)
        .map(|(bucket_start, (value, ckpt))| MarketValueSample {
            bucket_start,
            value: value.to_string(),
            checkpoint_ledger: ckpt,
        })
        .collect();
    samples.sort_by_key(|s| s.bucket_start);

    Ok(MarketHistoryPage {
        pool_id,
        bucket_width: width.as_str().to_string(),
        samples,
    })
}

/// Cache freshly computed buckets into the hot tier. Best-effort in the
/// sense that it never overwrites a hot row with a result derived from an
/// *older* checkpoint than what is already stored (a stale, slow caller
/// must never clobber a fresher cached value).
async fn materialize_hot_rows(
    pool: &PgPool,
    pool_id: i64,
    width: BucketWidth,
    computed: &[(DateTime<Utc>, i64)],
    checkpoint_ledger: i64,
) -> Result<(), sqlx::Error> {
    for (bucket_start, value) in computed {
        sqlx::query(
            r#"
            INSERT INTO market_history_samples
                (pool_id, bucket_width, bucket_start, value, checkpoint_ledger, computed_at)
            VALUES ($1, $2, $3, $4, $5, NOW())
            ON CONFLICT (pool_id, bucket_width, bucket_start) DO UPDATE
                SET value             = EXCLUDED.value,
                    checkpoint_ledger = EXCLUDED.checkpoint_ledger,
                    computed_at       = EXCLUDED.computed_at
            WHERE EXCLUDED.checkpoint_ledger >= market_history_samples.checkpoint_ledger
            "#,
        )
        .bind(pool_id)
        .bind(width.as_str())
        .bind(bucket_start)
        .bind(value)
        .bind(checkpoint_ledger)
        .execute(pool)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
    }

    #[test]
    fn bucket_width_round_trips_through_parse_and_as_str() {
        for width in [
            BucketWidth::OneMinute,
            BucketWidth::FiveMinutes,
            BucketWidth::OneHour,
            BucketWidth::OneDay,
        ] {
            assert_eq!(BucketWidth::parse(width.as_str()), Some(width));
        }
        assert_eq!(BucketWidth::parse("2h"), None);
        assert_eq!(BucketWidth::parse(""), None);
    }

    #[test]
    fn floor_to_bucket_truncates_to_exact_multiples_of_the_epoch() {
        let t = ts(2024, 1, 1, 0, 0, 37);
        assert_eq!(
            floor_to_bucket(t, BucketWidth::OneMinute),
            ts(2024, 1, 1, 0, 0, 0)
        );
        let t2 = ts(2024, 1, 1, 0, 4, 0);
        assert_eq!(
            floor_to_bucket(t2, BucketWidth::FiveMinutes),
            ts(2024, 1, 1, 0, 0, 0)
        );
        let t3 = ts(2024, 1, 1, 23, 59, 59);
        assert_eq!(
            floor_to_bucket(t3, BucketWidth::OneHour),
            ts(2024, 1, 1, 23, 0, 0)
        );
        assert_eq!(
            floor_to_bucket(t3, BucketWidth::OneDay),
            ts(2024, 1, 1, 0, 0, 0)
        );
    }

    #[test]
    fn bucket_count_rounds_up_partial_buckets_and_rejects_huge_ranges() {
        let from = ts(2024, 1, 1, 0, 0, 0);
        let to = ts(2024, 1, 1, 0, 0, 1); // 1 second into a 1h bucket.
        assert_eq!(bucket_count(from, to, BucketWidth::OneHour), 1);

        let to_wide = from + ChronoDuration::days(3000); // way beyond MAX_BUCKETS_PER_QUERY at 1d.
        assert!(bucket_count(from, to_wide, BucketWidth::OneDay) > MAX_BUCKETS_PER_QUERY);
    }

    // ── Deterministic aggregation against raw fixtures ──────────────────────

    #[test]
    fn deterministic_aggregation_matches_hand_computed_buckets() {
        let from = ts(2024, 6, 1, 0, 0, 0);
        let to = ts(2024, 6, 1, 3, 0, 0);
        let events = [
            RawStakeEvent {
                created_at: ts(2024, 6, 1, 0, 30, 0),
                amount: 100,
            },
            RawStakeEvent {
                created_at: ts(2024, 6, 1, 0, 45, 0),
                amount: 50,
            },
            RawStakeEvent {
                created_at: ts(2024, 6, 1, 2, 10, 0),
                amount: 25,
            },
        ];
        let buckets = bucket_cumulative_stake(0, &events, from, to, BucketWidth::OneHour);
        assert_eq!(
            buckets,
            vec![
                (ts(2024, 6, 1, 0, 0, 0), 150), // both 00:xx events land here
                (ts(2024, 6, 1, 1, 0, 0), 150), // no activity: carries forward
                (ts(2024, 6, 1, 2, 0, 0), 175), // the 02:10 event lands here
            ]
        );
    }

    #[test]
    fn nonzero_baseline_carries_into_the_first_bucket() {
        let from = ts(2024, 6, 1, 0, 0, 0);
        let to = ts(2024, 6, 1, 1, 0, 0);
        let buckets = bucket_cumulative_stake(1_000, &[], from, to, BucketWidth::OneHour);
        assert_eq!(buckets, vec![(ts(2024, 6, 1, 0, 0, 0), 1_000)]);
    }

    // ── Sparse intervals (missing-sample behavior = carry-forward) ──────────

    #[test]
    fn sparse_intervals_carry_the_last_known_value_forward() {
        let from = ts(2024, 6, 1, 0, 0, 0);
        let to = ts(2024, 6, 1, 6, 0, 0);
        // A single event at hour 1; hours 2-5 have no activity at all.
        let events = [RawStakeEvent {
            created_at: ts(2024, 6, 1, 1, 30, 0),
            amount: 42,
        }];
        let buckets = bucket_cumulative_stake(10, &events, from, to, BucketWidth::OneHour);
        let values: Vec<i64> = buckets.iter().map(|(_, v)| *v).collect();
        assert_eq!(values, vec![10, 52, 52, 52, 52, 52]);
        // Every bucket present, none omitted or null (carry-forward, not omission).
        assert_eq!(buckets.len(), 6);
    }

    // ── UTC bucketing is unaffected by local DST transitions ────────────────

    #[test]
    fn bucket_boundaries_are_stable_across_a_local_dst_transition() {
        // 2024-03-10 02:00 America/New_York (UTC-5 standard time) is the
        // instant US Eastern clocks spring forward to 03:00 (UTC-4). In UTC
        // this is simply 2024-03-10T07:00:00Z with no discontinuity at all.
        // If bucketing ever converted to local time before flooring, this
        // boundary would be skipped or duplicated; in pure UTC epoch math it
        // is just another hour.
        let from = ts(2024, 3, 10, 6, 0, 0); // 01:00 EST
        let to = ts(2024, 3, 10, 9, 0, 0); // 05:00 EDT
        let events = [
            RawStakeEvent {
                created_at: ts(2024, 3, 10, 6, 30, 0),
                amount: 10,
            }, // pre-transition
            RawStakeEvent {
                created_at: ts(2024, 3, 10, 7, 30, 0),
                amount: 20,
            }, // the transition hour, in UTC
            RawStakeEvent {
                created_at: ts(2024, 3, 10, 8, 30, 0),
                amount: 30,
            }, // post-transition
        ];
        let buckets = bucket_cumulative_stake(0, &events, from, to, BucketWidth::OneHour);
        assert_eq!(
            buckets,
            vec![
                (ts(2024, 3, 10, 6, 0, 0), 10),
                (ts(2024, 3, 10, 7, 0, 0), 30),
                (ts(2024, 3, 10, 8, 0, 0), 60),
            ],
            "UTC-hour buckets must be exactly 3 one-hour buckets, unaffected by the US DST transition"
        );
        // Every boundary is an exact multiple of 3600 seconds since the
        // epoch — proof bucketing never consulted a local calendar.
        for (bucket_start, _) in &buckets {
            assert_eq!(bucket_start.timestamp() % 3600, 0);
        }
    }

    #[test]
    fn empty_range_single_bucket_uses_baseline_only() {
        let from = ts(2024, 1, 1, 0, 0, 0);
        let to = ts(2024, 1, 1, 0, 1, 0);
        let buckets = bucket_cumulative_stake(5, &[], from, to, BucketWidth::OneMinute);
        assert_eq!(buckets, vec![(ts(2024, 1, 1, 0, 0, 0), 5)]);
    }
}

/// DB-backed tests for the full hot-tier aggregation and the hot/cold
/// archival contract (#314): query results for a fixed range must be
/// identical whether served purely from the hot tier, purely from the cold
/// tier, or split across both.
#[cfg(all(test, feature = "integration-tests"))]
mod db_tests {
    use super::super::market_history_archive::{
        archive_market_history, restore_market_history_archive,
    };
    use super::*;
    use crate::db::checkpoints::save_checkpoint;
    use sqlx::PgPool;

    async fn insert_prediction_placed(
        pool: &PgPool,
        pool_id: i64,
        ledger: i64,
        tx_hash: &str,
        created_at: DateTime<Utc>,
        amount: i64,
    ) {
        sqlx::query(
            r#"
            INSERT INTO portfolio_activity_ledger
                (network, contract_id, ledger, tx_hash, event_index, event_type,
                 user_address, pool_id, amount, created_at)
            VALUES ('testnet', 'CTEST', $1, $2, 0, 'prediction_placed', 'GSOME', $3, $4, $5)
            "#,
        )
        .bind(ledger)
        .bind(tx_hash)
        .bind(pool_id)
        .bind(amount)
        .bind(created_at)
        .execute(pool)
        .await
        .expect("insert prediction_placed activity row");
    }

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(secs, 0).unwrap()
    }

    /// Query results for the same `[from, to)` range must be identical
    /// before archival (served purely from the hot tier) and after the
    /// older half of the range has migrated to the cold tier (served from
    /// hot + cold combined).
    #[sqlx::test]
    async fn query_results_are_identical_before_archival_and_after_tier_migration(pool: PgPool) {
        let pool_id = 4242;
        let base = 1_700_000_000i64; // arbitrary fixed epoch anchor
        for i in 0..6 {
            insert_prediction_placed(
                &pool,
                pool_id,
                1,
                &format!("tx-{i}"),
                ts(base + i * 3600),
                100,
            )
            .await;
        }
        save_checkpoint(&pool, DEFAULT_LISTENER_ID, 1)
            .await
            .expect("save checkpoint");

        let from = floor_to_bucket(ts(base), BucketWidth::OneHour);
        let to = from + ChronoDuration::hours(8);

        let before = get_market_history(&pool, pool_id, BucketWidth::OneHour, from, to)
            .await
            .expect("query before archival");

        // Archive everything strictly before the midpoint of the range.
        let cutoff = from + ChronoDuration::hours(4);
        archive_market_history(&pool, pool_id, BucketWidth::OneHour, cutoff)
            .await
            .expect("archive old hot rows");

        let after = get_market_history(&pool, pool_id, BucketWidth::OneHour, from, to)
            .await
            .expect("query after tier migration");

        assert_eq!(before.samples.len(), after.samples.len());
        for (b, a) in before.samples.iter().zip(after.samples.iter()) {
            assert_eq!(b.bucket_start, a.bucket_start);
            assert_eq!(
                b.value, a.value,
                "value must be identical across tiers at {:?}",
                b.bucket_start
            );
        }
    }

    /// Round-trip: archive a set of hot rows, then restore them, and verify
    /// the restored values are byte-for-byte/value-for-value identical to
    /// the originals (including the archive's checksum verifying cleanly).
    #[sqlx::test]
    async fn archive_then_restore_round_trips_exactly(pool: PgPool) {
        let pool_id = 777;
        let base = 1_700_000_000i64;
        for i in 0..3 {
            insert_prediction_placed(
                &pool,
                pool_id,
                1,
                &format!("tx-{i}"),
                ts(base + i * 3600),
                50,
            )
            .await;
        }
        save_checkpoint(&pool, DEFAULT_LISTENER_ID, 1)
            .await
            .expect("save checkpoint");

        let from = floor_to_bucket(ts(base), BucketWidth::OneHour);
        let to = from + ChronoDuration::hours(3);
        let original = get_market_history(&pool, pool_id, BucketWidth::OneHour, from, to)
            .await
            .expect("materialize hot rows");

        let archive_id = archive_market_history(&pool, pool_id, BucketWidth::OneHour, to)
            .await
            .expect("archive all hot rows for this pool");

        // Hot tier is now empty for this range.
        let hot_rows: Vec<(DateTime<Utc>,)> =
            sqlx::query_as("SELECT bucket_start FROM market_history_samples WHERE pool_id = $1")
                .bind(pool_id)
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(
            hot_rows.is_empty(),
            "rows must have moved out of the hot tier"
        );

        let restored_count = restore_market_history_archive(&pool, archive_id)
            .await
            .expect("restore archive");
        assert_eq!(restored_count, original.samples.len() as u64);

        let after_restore = get_market_history(&pool, pool_id, BucketWidth::OneHour, from, to)
            .await
            .expect("query after restore");
        assert_eq!(after_restore.samples.len(), original.samples.len());
        for (orig, restored) in original.samples.iter().zip(after_restore.samples.iter()) {
            assert_eq!(orig.bucket_start, restored.bucket_start);
            assert_eq!(orig.value, restored.value);
        }

        // The archive itself is gone after a successful restore.
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM market_history_archives WHERE id = $1")
                .bind(archive_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, 0);
    }

    /// A corrupted archive (checksum no longer matches the stored payload)
    /// must be rejected by restore, never silently restored.
    #[sqlx::test]
    async fn restore_rejects_a_corrupted_archive(pool: PgPool) {
        let pool_id = 888;
        insert_prediction_placed(&pool, pool_id, 1, "tx-0", ts(1_700_000_000), 10).await;
        save_checkpoint(&pool, DEFAULT_LISTENER_ID, 1)
            .await
            .expect("save checkpoint");

        let from = floor_to_bucket(ts(1_700_000_000), BucketWidth::OneHour);
        let to = from + ChronoDuration::hours(1);
        get_market_history(&pool, pool_id, BucketWidth::OneHour, from, to)
            .await
            .expect("materialize hot row");

        let archive_id = archive_market_history(&pool, pool_id, BucketWidth::OneHour, to)
            .await
            .expect("archive hot row");

        sqlx::query("UPDATE market_history_archives SET payload = '[]' WHERE id = $1")
            .bind(archive_id)
            .execute(&pool)
            .await
            .unwrap();

        let err = restore_market_history_archive(&pool, archive_id)
            .await
            .expect_err("tampered payload must fail checksum verification");
        assert!(matches!(
            err,
            super::super::market_history_archive::ArchiveError::ChecksumMismatch { .. }
        ));
    }
}
