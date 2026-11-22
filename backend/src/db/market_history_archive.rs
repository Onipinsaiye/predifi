//! Hot/cold retention tiering for market-history samples (#314).
//!
//! `market_history_samples` (see `022_create_market_history_samples.sql`)
//! is the hot tier the bounded query API in [`super::market_history`] reads
//! from first. [`archive_market_history`] moves a pool's old hot rows into
//! a single checksummed row in `market_history_archives` (cold tier): the
//! rows are serialized to a canonical JSON array and hashed with SHA-256,
//! so [`restore_market_history_archive`] (and the transparent cold-tier
//! fallback in [`lookup_archived_samples`]) can detect corruption before
//! ever trusting an archived payload. This reuses Postgres storage the same
//! way `portfolio_exports` already stores its CSV artifacts inline — it is
//! a simple checksummed blob format, not a new storage engine.
//!
//! Because [`super::market_history::get_market_history`] merges hot and
//! cold rows into the exact same [`super::market_history::MarketValueSample`]
//! shape, callers never observe which tier (or both) a given sample came
//! from.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};

use super::market_history::BucketWidth;

/// One sample row as stored inside an archive's JSON payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct ArchivedSampleRow {
    bucket_start: DateTime<Utc>,
    value: i64,
    checkpoint_ledger: i64,
}

/// Canonical serialization of a set of sample rows: a plain JSON array,
/// sorted by `bucket_start`, so the same logical row set always produces
/// byte-identical payload text (and therefore the same checksum).
fn canonical_payload(rows: &[ArchivedSampleRow]) -> String {
    let mut sorted = rows.to_vec();
    sorted.sort_by_key(|r| r.bucket_start);
    serde_json::to_string(&sorted).expect("archived sample rows always serialize")
}

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Hand-rolled hex encoding (mirroring the hand-rolled base64url codec in
/// `crate::db::portfolio_ledger`), so a checksum digest doesn't need a new
/// `hex` dependency.
fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX_DIGITS[(b >> 4) as usize] as char);
        out.push(HEX_DIGITS[(b & 0x0f) as usize] as char);
    }
    out
}

/// SHA-256 hex digest of `payload`.
fn checksum(payload: &str) -> String {
    let digest = Sha256::digest(payload.as_bytes());
    to_hex(&digest)
}

/// Why an archive/restore operation failed.
#[derive(Debug)]
pub enum ArchiveError {
    /// No hot rows matched the archive request (nothing to move).
    NothingToArchive,
    /// The requested archive does not exist.
    NotFound,
    /// The stored payload's checksum no longer matches its content —
    /// the archive is corrupted and must never be restored as if valid.
    ChecksumMismatch {
        archive_id: i64,
    },
    /// The stored payload is not valid JSON for the expected row shape.
    Corrupted {
        archive_id: i64,
        reason: String,
    },
    Database(sqlx::Error),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveError::NothingToArchive => write!(f, "no hot rows matched the archive request"),
            ArchiveError::NotFound => write!(f, "archive not found"),
            ArchiveError::ChecksumMismatch { archive_id } => {
                write!(f, "archive {archive_id} failed checksum verification")
            }
            ArchiveError::Corrupted { archive_id, reason } => {
                write!(f, "archive {archive_id} payload is corrupted: {reason}")
            }
            ArchiveError::Database(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for ArchiveError {}

impl From<sqlx::Error> for ArchiveError {
    fn from(e: sqlx::Error) -> Self {
        ArchiveError::Database(e)
    }
}

#[derive(FromRow)]
struct ArchiveRow {
    id: i64,
    pool_id: i64,
    #[allow(dead_code)]
    bucket_width: String,
    checksum: String,
    payload: String,
}

/// Move every hot-tier row for `(pool_id, width)` with `bucket_start <
/// cutoff` into a single checksummed cold-tier archive, atomically (the
/// read, insert and delete all happen in one transaction, so a crash
/// midway never leaves a row duplicated across tiers or dropped
/// entirely). Returns the new archive's id.
pub async fn archive_market_history(
    pool: &PgPool,
    pool_id: i64,
    width: BucketWidth,
    cutoff: DateTime<Utc>,
) -> Result<i64, ArchiveError> {
    let mut tx = pool.begin().await?;

    let rows: Vec<(DateTime<Utc>, i64, i64)> = sqlx::query_as(
        r#"
        SELECT bucket_start, value, checkpoint_ledger
        FROM market_history_samples
        WHERE pool_id = $1 AND bucket_width = $2 AND bucket_start < $3
        ORDER BY bucket_start ASC
        "#,
    )
    .bind(pool_id)
    .bind(width.as_str())
    .bind(cutoff)
    .fetch_all(&mut *tx)
    .await?;

    if rows.is_empty() {
        return Err(ArchiveError::NothingToArchive);
    }

    let archived_rows: Vec<ArchivedSampleRow> = rows
        .iter()
        .map(
            |(bucket_start, value, checkpoint_ledger)| ArchivedSampleRow {
                bucket_start: *bucket_start,
                value: *value,
                checkpoint_ledger: *checkpoint_ledger,
            },
        )
        .collect();
    let bucket_start_min = archived_rows.iter().map(|r| r.bucket_start).min().unwrap();
    let bucket_start_max = archived_rows.iter().map(|r| r.bucket_start).max().unwrap();
    let payload = canonical_payload(&archived_rows);
    let sum = checksum(&payload);

    let archive_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO market_history_archives
            (pool_id, bucket_width, bucket_start_min, bucket_start_max, row_count, checksum, payload, archived_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
        RETURNING id
        "#,
    )
    .bind(pool_id)
    .bind(width.as_str())
    .bind(bucket_start_min)
    .bind(bucket_start_max)
    .bind(archived_rows.len() as i32)
    .bind(&sum)
    .bind(&payload)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        r#"DELETE FROM market_history_samples WHERE pool_id = $1 AND bucket_width = $2 AND bucket_start < $3"#,
    )
    .bind(pool_id)
    .bind(width.as_str())
    .bind(cutoff)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(archive_id)
}

/// Restore an archive's rows back into the hot tier, verifying the
/// checksum first. A corrupted payload is rejected with
/// [`ArchiveError::ChecksumMismatch`] rather than silently restored.
///
/// On success, the archive row is removed (its rows now live in the hot
/// tier again) and the number of restored rows is returned.
pub async fn restore_market_history_archive(
    pool: &PgPool,
    archive_id: i64,
) -> Result<u64, ArchiveError> {
    let row: Option<ArchiveRow> = sqlx::query_as(
        r#"SELECT id, pool_id, bucket_width, checksum, payload FROM market_history_archives WHERE id = $1"#,
    )
    .bind(archive_id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Err(ArchiveError::NotFound);
    };

    let samples = decode_verified_payload(&row)?;

    let mut tx = pool.begin().await?;
    for sample in &samples {
        sqlx::query(
            r#"
            INSERT INTO market_history_samples
                (pool_id, bucket_width, bucket_start, value, checkpoint_ledger, computed_at)
            VALUES ($1, $2, $3, $4, $5, NOW())
            ON CONFLICT (pool_id, bucket_width, bucket_start) DO UPDATE
                SET value             = EXCLUDED.value,
                    checkpoint_ledger = EXCLUDED.checkpoint_ledger,
                    computed_at       = EXCLUDED.computed_at
            "#,
        )
        .bind(row.pool_id)
        .bind(&row.bucket_width)
        .bind(sample.bucket_start)
        .bind(sample.value)
        .bind(sample.checkpoint_ledger)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("DELETE FROM market_history_archives WHERE id = $1")
        .bind(archive_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(samples.len() as u64)
}

fn decode_verified_payload(row: &ArchiveRow) -> Result<Vec<ArchivedSampleRow>, ArchiveError> {
    let actual = checksum(&row.payload);
    if actual != row.checksum {
        return Err(ArchiveError::ChecksumMismatch { archive_id: row.id });
    }
    serde_json::from_str(&row.payload).map_err(|e| ArchiveError::Corrupted {
        archive_id: row.id,
        reason: e.to_string(),
    })
}

/// Look up archived samples for `(pool_id, width)` whose archive overlaps
/// `[from, to)`, verifying each candidate archive's checksum before
/// trusting its contents. A corrupted archive is logged and skipped rather
/// than surfaced as a hard query failure — the bounded query API degrades
/// to "these particular buckets are unavailable", not a 500 for the whole
/// page.
pub(super) async fn lookup_archived_samples(
    pool: &PgPool,
    pool_id: i64,
    width: BucketWidth,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<(DateTime<Utc>, i64, i64)>, sqlx::Error> {
    let rows: Vec<ArchiveRow> = sqlx::query_as(
        r#"
        SELECT id, pool_id, bucket_width, checksum, payload
        FROM market_history_archives
        WHERE pool_id = $1 AND bucket_width = $2
          AND bucket_start_min < $4 AND bucket_start_max >= $3
        "#,
    )
    .bind(pool_id)
    .bind(width.as_str())
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::new();
    for row in rows {
        let samples = match decode_verified_payload(&row) {
            Ok(samples) => samples,
            Err(e) => {
                tracing::error!(archive_id = row.id, error = %e, "skipping corrupted market history archive");
                continue;
            }
        };
        for s in samples {
            if s.bucket_start >= from && s.bucket_start < to {
                out.push((s.bucket_start, s.value, s.checkpoint_ledger));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(secs: i64, value: i64) -> ArchivedSampleRow {
        ArchivedSampleRow {
            bucket_start: DateTime::<Utc>::from_timestamp(secs, 0).unwrap(),
            value,
            checkpoint_ledger: 10,
        }
    }

    #[test]
    fn canonical_payload_is_sorted_and_deterministic() {
        let rows = vec![sample(7200, 30), sample(0, 10), sample(3600, 20)];
        let payload = canonical_payload(&rows);
        let reparsed: Vec<ArchivedSampleRow> = serde_json::from_str(&payload).unwrap();
        assert_eq!(
            reparsed,
            vec![sample(0, 10), sample(3600, 20), sample(7200, 30)]
        );

        // Same logical rows, different input order, must produce the exact
        // same payload text (and therefore the same checksum).
        let rows_reordered = vec![sample(3600, 20), sample(7200, 30), sample(0, 10)];
        assert_eq!(payload, canonical_payload(&rows_reordered));
    }

    #[test]
    fn checksum_detects_any_payload_mutation() {
        let payload = canonical_payload(&[sample(0, 10)]);
        let sum = checksum(&payload);

        let tampered = payload.replace("10", "999999");
        assert_ne!(
            checksum(&tampered),
            sum,
            "a mutated payload must not keep the original checksum"
        );
    }

    #[test]
    fn decode_verified_payload_rejects_checksum_mismatch() {
        let payload = canonical_payload(&[sample(0, 10)]);
        let row = ArchiveRow {
            id: 1,
            pool_id: 1,
            bucket_width: "1h".to_string(),
            checksum: "0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            payload,
        };
        let err = decode_verified_payload(&row).unwrap_err();
        assert!(matches!(
            err,
            ArchiveError::ChecksumMismatch { archive_id: 1 }
        ));
    }

    #[test]
    fn decode_verified_payload_round_trips_with_a_correct_checksum() {
        let rows = vec![sample(0, 10), sample(3600, 20)];
        let payload = canonical_payload(&rows);
        let sum = checksum(&payload);
        let row = ArchiveRow {
            id: 2,
            pool_id: 1,
            bucket_width: "1h".to_string(),
            checksum: sum,
            payload,
        };
        let decoded = decode_verified_payload(&row).expect("valid checksum must decode");
        assert_eq!(decoded, rows);
    }

    #[test]
    fn hex_encoding_matches_known_vectors() {
        assert_eq!(to_hex(&[0x00, 0xff, 0x0a]), "00ff0a");
        assert_eq!(to_hex(&[]), "");
    }
}
