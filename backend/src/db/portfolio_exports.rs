//! Bounded asynchronous portfolio exports (#312).
//!
//! A large export must not hold an HTTP connection open or exhaust DB
//! resources. Export jobs are created here and processed by
//! [`run_next_export`], which leases the job through the existing
//! `background_jobs` table (`kind = `[`EXPORT_KIND`]`), streams rows from
//! `portfolio_activity_ledger` in bounded batches against a single
//! committed indexer checkpoint (never mixing in events the checkpoint
//! hasn't advanced past — the same constraint
//! [`crate::db::portfolio_snapshots`] enforces), and stops once either the
//! row limit or the time limit is reached. The finished artifact (CSV text)
//! is stored with an `expires_at`; an expired artifact can no longer be
//! downloaded.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool};

use super::checkpoints::{load_checkpoint, CheckpointStatus, DEFAULT_LISTENER_ID};

/// `background_jobs.kind` used for portfolio export jobs.
pub const EXPORT_KIND: &str = "portfolio_export";

/// Default/maximum row and time bounds enforced on every export.
pub const DEFAULT_ROW_LIMIT: i64 = 10_000;
pub const MAX_ROW_LIMIT: i64 = 50_000;
pub const DEFAULT_TIME_LIMIT_SECS: i64 = 60;
pub const MAX_TIME_LIMIT_SECS: i64 = 300;
/// How long a finished artifact stays downloadable.
pub const ARTIFACT_TTL_SECS: i64 = 24 * 3600;
/// Row batch size used while streaming an export, so no single query holds
/// an unbounded result set in memory.
const EXPORT_BATCH_SIZE: i64 = 500;

/// Lifecycle status of an export, including the derived `expired` state
/// (a `succeeded` export whose `expires_at` has passed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    /// Derived, not stored: `succeeded` plus `expires_at` in the past.
    Expired,
}

#[derive(Debug, Clone, FromRow)]
struct ExportRow {
    id: i64,
    job_id: i64,
    user_address: String,
    status: String,
    checkpoint_ledger: Option<i64>,
    row_limit: i32,
    time_limit_secs: i32,
    rows_written: i32,
    expires_at: Option<DateTime<Utc>>,
    error: Option<String>,
    created_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
}

/// Public, client-facing view of an export's status and progress.
#[derive(Debug, Clone, Serialize)]
pub struct ExportStatusView {
    pub id: i64,
    pub status: ExportStatus,
    pub rows_written: i64,
    pub row_limit: i64,
    pub time_limit_secs: i64,
    pub checkpoint_ledger: Option<i64>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    /// `true` only when the artifact exists and has not expired.
    pub download_available: bool,
}

fn effective_status(row: &ExportRow, now: DateTime<Utc>) -> ExportStatus {
    match row.status.as_str() {
        "pending" => ExportStatus::Pending,
        "running" => ExportStatus::Running,
        "failed" => ExportStatus::Failed,
        "cancelled" => ExportStatus::Cancelled,
        "succeeded" => {
            if row.expires_at.is_some_and(|exp| exp < now) {
                ExportStatus::Expired
            } else {
                ExportStatus::Succeeded
            }
        }
        _ => ExportStatus::Failed,
    }
}

fn to_view(row: &ExportRow, now: DateTime<Utc>) -> ExportStatusView {
    let status = effective_status(row, now);
    ExportStatusView {
        id: row.id,
        status,
        rows_written: row.rows_written as i64,
        row_limit: row.row_limit as i64,
        time_limit_secs: row.time_limit_secs as i64,
        checkpoint_ledger: row.checkpoint_ledger,
        error: row.error.clone(),
        created_at: row.created_at,
        completed_at: row.completed_at,
        download_available: matches!(status, ExportStatus::Succeeded),
    }
}

/// Create a new export job for `wallet_address`, bounded by `row_limit` /
/// `time_limit_secs` (clamped to `[1, MAX_*]`, defaulting to `DEFAULT_*`
/// when not supplied).
pub async fn create_export(
    pool: &PgPool,
    wallet_address: &str,
    row_limit: Option<i64>,
    time_limit_secs: Option<i64>,
) -> Result<ExportStatusView, sqlx::Error> {
    let row_limit = row_limit.unwrap_or(DEFAULT_ROW_LIMIT).clamp(1, MAX_ROW_LIMIT);
    let time_limit_secs = time_limit_secs
        .unwrap_or(DEFAULT_TIME_LIMIT_SECS)
        .clamp(1, MAX_TIME_LIMIT_SECS);

    let mut tx = pool.begin().await?;

    let job_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO background_jobs (queue, kind, payload, max_attempts)
        VALUES ('portfolio_exports', $1, $2, 1)
        RETURNING id
        "#,
    )
    .bind(EXPORT_KIND)
    .bind(serde_json::json!({
        "user_address": wallet_address,
        "row_limit": row_limit,
        "time_limit_secs": time_limit_secs,
    }))
    .fetch_one(&mut *tx)
    .await?;

    let row = sqlx::query_as::<_, ExportRow>(
        r#"
        INSERT INTO portfolio_exports (job_id, user_address, status, row_limit, time_limit_secs)
        VALUES ($1, $2, 'pending', $3, $4)
        RETURNING id, job_id, user_address, status, checkpoint_ledger, row_limit,
                  time_limit_secs, rows_written, expires_at, error, created_at, completed_at
        "#,
    )
    .bind(job_id)
    .bind(wallet_address)
    .bind(row_limit as i32)
    .bind(time_limit_secs as i32)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(to_view(&row, Utc::now()))
}

/// Fetch an export's owner (for authorization) and status view.
pub async fn get_export_status(
    pool: &PgPool,
    export_id: i64,
) -> Result<Option<(String, ExportStatusView)>, sqlx::Error> {
    let row = sqlx::query_as::<_, ExportRow>(
        r#"
        SELECT id, job_id, user_address, status, checkpoint_ledger, row_limit,
               time_limit_secs, rows_written, expires_at, error, created_at, completed_at
        FROM portfolio_exports
        WHERE id = $1
        "#,
    )
    .bind(export_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
        let view = to_view(&r, Utc::now());
        (r.user_address, view)
    }))
}

/// Outcome of fetching an export's artifact for download.
pub enum ArtifactResult {
    Available { owner: String, content: String },
    NotReady,
    Expired,
}

/// Fetch an export's downloadable artifact, enforcing expiry: an expired
/// artifact is never returned, even if the content still exists on disk.
pub async fn get_export_artifact(
    pool: &PgPool,
    export_id: i64,
) -> Result<Option<ArtifactResult>, sqlx::Error> {
    let row = sqlx::query_as::<_, ExportRow>(
        r#"
        SELECT id, job_id, user_address, status, checkpoint_ledger, row_limit,
               time_limit_secs, rows_written, expires_at, error, created_at, completed_at
        FROM portfolio_exports
        WHERE id = $1
        "#,
    )
    .bind(export_id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let result = match effective_status(&row, Utc::now()) {
        ExportStatus::Succeeded => {
            let content: Option<String> =
                sqlx::query_scalar("SELECT artifact_content FROM portfolio_exports WHERE id = $1")
                    .bind(export_id)
                    .fetch_one(pool)
                    .await?;
            match content {
                Some(content) => ArtifactResult::Available {
                    owner: row.user_address,
                    content,
                },
                None => ArtifactResult::NotReady,
            }
        }
        ExportStatus::Expired => ArtifactResult::Expired,
        _ => ArtifactResult::NotReady,
    };

    Ok(Some(result))
}

/// Outcome of a cancellation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    Cancelled,
    NotFound,
    Forbidden,
    /// The export already reached a terminal state (succeeded/failed/
    /// cancelled/expired) and can no longer be cancelled.
    AlreadyTerminal,
}

/// Cancel a pending or running export on behalf of `requester`. Only the
/// owning wallet may cancel its own export.
///
/// A cancelled job is moved to a terminal `cancelled` state in both
/// `portfolio_exports` and `background_jobs`, so [`run_next_export`] never
/// picks it up (its claim query only selects `status = 'pending'` jobs).
pub async fn cancel_export(
    pool: &PgPool,
    export_id: i64,
    requester: &str,
) -> Result<CancelOutcome, sqlx::Error> {
    let Some((owner, view)) = get_export_status(pool, export_id).await? else {
        return Ok(CancelOutcome::NotFound);
    };
    if owner != requester {
        return Ok(CancelOutcome::Forbidden);
    }
    if !matches!(view.status, ExportStatus::Pending | ExportStatus::Running) {
        return Ok(CancelOutcome::AlreadyTerminal);
    }

    let mut tx = pool.begin().await?;
    let updated = sqlx::query(
        r#"
        UPDATE portfolio_exports
        SET status = 'cancelled', updated_at = NOW(), completed_at = NOW()
        WHERE id = $1 AND status IN ('pending', 'running')
        "#,
    )
    .bind(export_id)
    .execute(&mut *tx)
    .await?;

    if updated.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(CancelOutcome::AlreadyTerminal);
    }

    sqlx::query(
        r#"
        UPDATE background_jobs
        SET status = 'cancelled', updated_at = NOW(), completed_at = NOW()
        WHERE id = (SELECT job_id FROM portfolio_exports WHERE id = $1)
          AND status IN ('pending', 'running')
        "#,
    )
    .bind(export_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(CancelOutcome::Cancelled)
}

/// Claim and run the next pending export job, bounded by its row/time
/// limits and by the committed indexer checkpoint. Returns `Ok(None)` when
/// there was no pending job to claim.
pub async fn run_next_export(pool: &PgPool) -> Result<Option<i64>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let claimed: Option<(i64, i64, i32, i32, String)> = sqlx::query_as(
        r#"
        SELECT e.id, e.job_id, e.row_limit, e.time_limit_secs, e.user_address
        FROM portfolio_exports e
        JOIN background_jobs j ON j.id = e.job_id
        WHERE e.status = 'pending' AND j.status = 'pending'
        ORDER BY e.id
        LIMIT 1
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .fetch_optional(&mut *tx)
    .await?;

    let Some((export_id, job_id, row_limit, time_limit_secs, wallet_address)) = claimed else {
        tx.rollback().await?;
        return Ok(None);
    };

    sqlx::query(
        "UPDATE portfolio_exports SET status = 'running', started_at = NOW(), updated_at = NOW() WHERE id = $1",
    )
    .bind(export_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        UPDATE background_jobs
        SET status = 'running',
            lease_owner = 'portfolio_export_worker',
            lease_expires_at = NOW() + ($1::bigint * INTERVAL '1 second'),
            fencing_token = fencing_token + 1,
            attempts = attempts + 1
        WHERE id = $2
        "#,
    )
    .bind(time_limit_secs as i64)
    .bind(job_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    match export_rows_bounded(pool, &wallet_address, row_limit as i64, time_limit_secs as i64).await
    {
        Ok((content, rows_written, checkpoint_ledger)) => {
            // Re-check status: a cancellation may have landed while this
            // export was running. Never overwrite a terminal cancellation
            // with a late success.
            sqlx::query(
                r#"
                UPDATE portfolio_exports
                SET status = 'succeeded',
                    artifact_content = $1,
                    rows_written = $2,
                    checkpoint_ledger = $3,
                    expires_at = NOW() + ($4::bigint * INTERVAL '1 second'),
                    completed_at = NOW(),
                    updated_at = NOW()
                WHERE id = $5 AND status = 'running'
                "#,
            )
            .bind(content)
            .bind(rows_written as i32)
            .bind(checkpoint_ledger)
            .bind(ARTIFACT_TTL_SECS)
            .bind(export_id)
            .execute(pool)
            .await?;
            sqlx::query(
                "UPDATE background_jobs SET status = 'succeeded', completed_at = NOW(), updated_at = NOW() WHERE id = $1 AND status = 'running'",
            )
            .bind(job_id)
            .execute(pool)
            .await?;
        }
        Err(message) => {
            sqlx::query(
                "UPDATE portfolio_exports SET status = 'failed', error = $1, updated_at = NOW() WHERE id = $2 AND status = 'running'",
            )
            .bind(&message)
            .bind(export_id)
            .execute(pool)
            .await?;
            sqlx::query(
                "UPDATE background_jobs SET status = 'failed', last_error = $1, updated_at = NOW() WHERE id = $2 AND status = 'running'",
            )
            .bind(&message)
            .bind(job_id)
            .execute(pool)
            .await?;
        }
    }

    Ok(Some(export_id))
}

/// Stream `wallet_address`'s activity into a bounded CSV artifact.
///
/// Reads are checkpoint-gated (`ledger <= committed_checkpoint_ledger`)
/// exactly like [`crate::db::portfolio_snapshots`], so an export never
/// mixes in events from an indexer batch that hasn't fully committed.
/// Stops as soon as either `row_limit` rows have been written or
/// `time_limit_secs` has elapsed — whichever comes first — rather than
/// running unbounded.
async fn export_rows_bounded(
    pool: &PgPool,
    wallet_address: &str,
    row_limit: i64,
    time_limit_secs: i64,
) -> Result<(String, i64, Option<i64>), String> {
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(time_limit_secs.max(1) as u64);

    let checkpoint_ledger = match load_checkpoint(pool, DEFAULT_LISTENER_ID).await {
        CheckpointStatus::Valid(ledger) => ledger as i64,
        CheckpointStatus::Missing => {
            return Err("listener checkpoint not yet established".to_string())
        }
        CheckpointStatus::Malformed(reason) => {
            return Err(format!("listener checkpoint malformed: {reason}"))
        }
    };

    let mut content =
        String::from("ledger,tx_hash,event_index,event_type,pool_id,outcome,amount,created_at\n");
    let mut rows_written: i64 = 0;
    let mut after_id: i64 = 0;

    loop {
        if tokio::time::Instant::now() >= deadline || rows_written >= row_limit {
            break;
        }
        let remaining = (row_limit - rows_written).min(EXPORT_BATCH_SIZE);

        let batch: Vec<(
            i64,
            i64,
            String,
            i32,
            String,
            Option<i64>,
            Option<i32>,
            Option<i64>,
            DateTime<Utc>,
        )> = sqlx::query_as(
            r#"
            SELECT id, ledger, tx_hash, event_index, event_type, pool_id, outcome, amount, created_at
            FROM portfolio_activity_ledger
            WHERE user_address = $1 AND ledger <= $2 AND id > $3
            ORDER BY id
            LIMIT $4
            "#,
        )
        .bind(wallet_address)
        .bind(checkpoint_ledger)
        .bind(after_id)
        .bind(remaining)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

        if batch.is_empty() {
            break;
        }

        for (id, ledger, tx_hash, event_index, event_type, pool_id, outcome, amount, created_at) in
            &batch
        {
            content.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                ledger,
                tx_hash,
                event_index,
                event_type,
                pool_id.map(|v| v.to_string()).unwrap_or_default(),
                outcome.map(|v| v.to_string()).unwrap_or_default(),
                amount.map(|v| v.to_string()).unwrap_or_default(),
                created_at.to_rfc3339(),
            ));
            rows_written += 1;
            after_id = *id;
        }
    }

    Ok((content, rows_written, Some(checkpoint_ledger)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_artifact_is_not_downloadable_even_when_status_says_succeeded() {
        let row = ExportRow {
            id: 1,
            job_id: 1,
            user_address: "GWALLET".to_string(),
            status: "succeeded".to_string(),
            checkpoint_ledger: Some(10),
            row_limit: 100,
            time_limit_secs: 60,
            rows_written: 5,
            expires_at: Some(Utc::now() - chrono::Duration::seconds(1)),
            error: None,
            created_at: Utc::now(),
            completed_at: Some(Utc::now()),
        };
        let view = to_view(&row, Utc::now());
        assert_eq!(view.status, ExportStatus::Expired);
        assert!(!view.download_available);
    }

    #[test]
    fn unexpired_succeeded_artifact_is_downloadable() {
        let row = ExportRow {
            id: 1,
            job_id: 1,
            user_address: "GWALLET".to_string(),
            status: "succeeded".to_string(),
            checkpoint_ledger: Some(10),
            row_limit: 100,
            time_limit_secs: 60,
            rows_written: 5,
            expires_at: Some(Utc::now() + chrono::Duration::seconds(60)),
            error: None,
            created_at: Utc::now(),
            completed_at: Some(Utc::now()),
        };
        let view = to_view(&row, Utc::now());
        assert_eq!(view.status, ExportStatus::Succeeded);
        assert!(view.download_available);
    }

    #[test]
    fn pending_and_running_are_not_downloadable() {
        for status in ["pending", "running"] {
            let row = ExportRow {
                id: 1,
                job_id: 1,
                user_address: "GWALLET".to_string(),
                status: status.to_string(),
                checkpoint_ledger: None,
                row_limit: 100,
                time_limit_secs: 60,
                rows_written: 0,
                expires_at: None,
                error: None,
                created_at: Utc::now(),
                completed_at: None,
            };
            let view = to_view(&row, Utc::now());
            assert!(!view.download_available, "status {status}");
        }
    }
}

#[cfg(all(test, feature = "integration-tests"))]
mod db_tests {
    use super::*;
    use crate::db::checkpoints::save_checkpoint;
    use sqlx::PgPool;

    async fn seed_activity(pool: &PgPool, wallet: &str, rows: i64) {
        for i in 0..rows {
            sqlx::query(
                r#"
                INSERT INTO portfolio_activity_ledger
                    (network, contract_id, ledger, tx_hash, event_index, event_type,
                     user_address, pool_id, amount)
                VALUES ('testnet', 'CTEST', 1, $1, $2, 'prediction_placed', $3, 1, 10)
                "#,
            )
            .bind(format!("tx-{i}"))
            .bind(i as i32)
            .bind(wallet)
            .execute(pool)
            .await
            .expect("seed activity row");
        }
    }

    /// Bounded-limits: the export must stop at `row_limit` rows even though
    /// more rows exist, never run unbounded.
    #[sqlx::test]
    async fn export_enforces_the_row_limit(pool: PgPool) {
        let wallet = "GROWLIMITTESTWALLET00000000000000000000000000000000000";
        seed_activity(&pool, wallet, 20).await;
        save_checkpoint(&pool, DEFAULT_LISTENER_ID, 1)
            .await
            .expect("save checkpoint");

        let (_, rows_written, checkpoint) = export_rows_bounded(&pool, wallet, 5, 60)
            .await
            .expect("bounded export");
        assert_eq!(rows_written, 5, "must stop exactly at the row limit");
        assert_eq!(checkpoint, Some(1));
    }

    /// Cancelling a pending export moves it to a terminal state and keeps
    /// the worker from ever picking it up.
    #[sqlx::test]
    async fn cancel_moves_job_to_terminal_state_and_worker_skips_it(pool: PgPool) {
        let wallet = "GCANCELTESTWALLET0000000000000000000000000000000000000";
        let created = create_export(&pool, wallet, Some(10), Some(30))
            .await
            .expect("create export");

        let outcome = cancel_export(&pool, created.id, wallet)
            .await
            .expect("cancel export");
        assert_eq!(outcome, CancelOutcome::Cancelled);

        let (_, view) = get_export_status(&pool, created.id)
            .await
            .expect("fetch status")
            .expect("export exists");
        assert_eq!(view.status, ExportStatus::Cancelled);

        // The worker's claim query only selects pending jobs, so a
        // cancelled job is never picked up again.
        let claimed = run_next_export(&pool).await.expect("run worker");
        assert_ne!(claimed, Some(created.id));
    }

    #[sqlx::test]
    async fn cancel_by_a_different_wallet_is_forbidden(pool: PgPool) {
        let owner = "GOWNERTESTWALLET00000000000000000000000000000000000000";
        let created = create_export(&pool, owner, None, None)
            .await
            .expect("create export");

        let outcome = cancel_export(&pool, created.id, "GSOMEONEELSE00000000000000000000000000000000000000000")
            .await
            .expect("cancel attempt");
        assert_eq!(outcome, CancelOutcome::Forbidden);
    }

    #[sqlx::test]
    async fn download_before_completion_returns_not_ready(pool: PgPool) {
        let wallet = "GNOTREADYTESTWALLET000000000000000000000000000000000000";
        let created = create_export(&pool, wallet, None, None)
            .await
            .expect("create export");

        let artifact = get_export_artifact(&pool, created.id)
            .await
            .expect("fetch artifact");
        assert!(matches!(artifact, Some(ArtifactResult::NotReady)));
    }
}
