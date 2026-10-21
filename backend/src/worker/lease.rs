//! Leased background jobs backed by Postgres (#65).
//!
//! Multiple backend replicas share one `background_jobs` table. A worker
//! claims a job by taking a **lease**: the row moves to `running`, records the
//! owner and an expiry, and its **fencing token** is incremented. Every later
//! write about that job — [`JobStore::extend`], [`JobStore::complete`],
//! [`JobStore::fail`] — must present the token it was issued, so:
//!
//! * two workers can never hold the same job at once (`FOR UPDATE SKIP
//!   LOCKED` hands out disjoint rows);
//! * a worker that dies, stalls, or is partitioned loses the job once its
//!   lease expires: the next claim takes it over with a higher token, and
//!   any late completion from the old holder is rejected with
//!   [`LeaseError::Lost`];
//! * redelivery is bounded: every claim counts as an attempt, and a job whose
//!   lease expires on its final attempt is failed instead of handed out again.
//!
//! ## Error classes
//!
//! Handlers return [`JobError::Retryable`] for transient problems (I/O,
//! timeouts, serialization failures, an upstream being down): the job goes
//! back to `pending` with exponential backoff until `max_attempts` is
//! reached. [`JobError::Terminal`] (bad payload, invariant violated, missing
//! entity) fails the job immediately; retrying could not succeed.
//!
//! ## Handler requirements
//!
//! Delivery is **at-least-once**. A handler may run more than once for the
//! same job (the holder crashed after committing its side effects but before
//! [`JobStore::complete`]), so handlers must be idempotent: key side effects
//! on the job id (`INSERT ... ON CONFLICT DO NOTHING`), or guard writes with
//! the fencing token (`UPDATE ... WHERE last_fencing_token < $token`) so a
//! stale holder's writes are discarded.

use std::future::Future;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use tracing::{debug, warn};

/// Default lease length; the holder extends it every third of this.
pub const DEFAULT_LEASE: Duration = Duration::from_secs(30);
/// Default maximum attempts (claims) per job.
pub const DEFAULT_MAX_ATTEMPTS: i32 = 5;

/// Lease and retry timing.
#[derive(Debug, Clone, Copy)]
pub struct LeaseConfig {
    /// How long a claim is valid without a heartbeat.
    pub lease: Duration,
    /// Backoff before the second attempt; doubles per attempt.
    pub retry_base_delay: Duration,
    /// Upper bound on backoff.
    pub retry_max_delay: Duration,
}

impl Default for LeaseConfig {
    fn default() -> Self {
        Self {
            lease: DEFAULT_LEASE,
            retry_base_delay: Duration::from_secs(1),
            retry_max_delay: Duration::from_secs(300),
        }
    }
}

impl LeaseConfig {
    /// Backoff applied after the `attempt`-th attempt failed (1-indexed).
    pub fn retry_delay(&self, attempt: i32) -> Duration {
        let shift = attempt.saturating_sub(1).clamp(0, 20) as u32;
        self.retry_base_delay
            .saturating_mul(2u32.saturating_pow(shift))
            .min(self.retry_max_delay)
    }
}

/// A job to enqueue.
#[derive(Debug, Clone)]
pub struct NewJob {
    pub queue: String,
    pub kind: String,
    pub payload: serde_json::Value,
    /// Producer-side de-duplication key; a second enqueue with the same key is
    /// ignored.
    pub dedupe_key: Option<String>,
    pub max_attempts: i32,
}

impl NewJob {
    pub fn new(
        queue: impl Into<String>,
        kind: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            queue: queue.into(),
            kind: kind.into(),
            payload,
            dedupe_key: None,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
        }
    }

    pub fn dedupe_key(mut self, key: impl Into<String>) -> Self {
        self.dedupe_key = Some(key.into());
        self
    }

    pub fn max_attempts(mut self, max_attempts: i32) -> Self {
        self.max_attempts = max_attempts.max(1);
        self
    }
}

/// A job held under lease. `fencing_token` identifies this claim.
#[derive(Debug, Clone, PartialEq)]
pub struct LeasedJob {
    pub id: i64,
    pub queue: String,
    pub kind: String,
    pub payload: serde_json::Value,
    /// Attempt number of this claim (1 for the first delivery).
    pub attempt: i32,
    pub max_attempts: i32,
    pub fencing_token: i64,
    pub lease_expires_at: DateTime<Utc>,
}

/// Failure reported by a job handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobError {
    /// Transient; the job is retried with backoff while attempts remain.
    Retryable(String),
    /// Permanent; the job is failed immediately.
    Terminal(String),
}

impl JobError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable(_))
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Retryable(m) | Self::Terminal(m) => m,
        }
    }
}

impl From<sqlx::Error> for JobError {
    /// Connection problems and serialization/deadlock aborts are transient;
    /// decode and missing-row errors indicate a bug or bad data.
    fn from(error: sqlx::Error) -> Self {
        let retryable = match &error {
            sqlx::Error::Io(_)
            | sqlx::Error::PoolTimedOut
            | sqlx::Error::PoolClosed
            | sqlx::Error::Tls(_)
            | sqlx::Error::Protocol(_)
            | sqlx::Error::WorkerCrashed => true,
            sqlx::Error::Database(db) => {
                matches!(db.code().as_deref(), Some("40001" | "40P01" | "57P01"))
            }
            _ => false,
        };
        if retryable {
            Self::Retryable(error.to_string())
        } else {
            Self::Terminal(error.to_string())
        }
    }
}

/// The lease presented no longer owns the job, or the database failed.
#[derive(Debug)]
pub enum LeaseError {
    /// The fencing token is stale: the lease expired and the job was
    /// reclaimed, or it was already finished.
    Lost,
    Database(sqlx::Error),
}

impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lost => f.write_str("job lease lost (stale fencing token)"),
            Self::Database(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for LeaseError {}

impl From<sqlx::Error> for LeaseError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

/// Where a failed job went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailOutcome {
    /// Back to `pending`, available after backoff.
    Retrying,
    /// Terminally `failed`.
    Failed,
}

/// Result of [`JobStore::run_one`].
#[derive(Debug)]
pub struct RunReport {
    pub job_id: i64,
    pub fencing_token: i64,
    /// `Ok(None)` on success, `Ok(Some(_))` when the handler failed.
    pub outcome: Result<Option<FailOutcome>, LeaseError>,
}

/// Aborts the wrapped task when dropped, so a killed worker's heartbeat stops
/// with it.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Postgres-backed leased job queue.
#[derive(Clone)]
pub struct JobStore {
    pool: PgPool,
    config: LeaseConfig,
}

impl JobStore {
    pub fn new(pool: PgPool, config: LeaseConfig) -> Self {
        Self { pool, config }
    }

    pub fn config(&self) -> LeaseConfig {
        self.config
    }

    /// Enqueue a job. Returns `None` when `dedupe_key` was already enqueued.
    pub async fn enqueue(&self, job: &NewJob) -> Result<Option<i64>, sqlx::Error> {
        let row = sqlx::query(
            r#"
            INSERT INTO background_jobs (queue, kind, dedupe_key, payload, max_attempts)
            VALUES ($1, $2, $3, $4::jsonb, $5)
            ON CONFLICT (dedupe_key) DO NOTHING
            RETURNING id
            "#,
        )
        .bind(&job.queue)
        .bind(&job.kind)
        .bind(&job.dedupe_key)
        .bind(job.payload.to_string())
        .bind(job.max_attempts)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| r.try_get("id")).transpose()
    }

    /// Fail running jobs in `queue` whose lease expired on their final
    /// attempt. Called by [`claim`](Self::claim); returns rows failed.
    pub async fn fail_exhausted_leases(&self, queue: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            r#"
            UPDATE background_jobs
            SET status = 'failed',
                last_error = 'lease expired on final attempt',
                lease_owner = NULL,
                lease_expires_at = NULL,
                completed_at = NOW(),
                updated_at = NOW()
            WHERE queue = $1
              AND status = 'running'
              AND lease_expires_at < NOW()
              AND attempts >= max_attempts
            "#,
        )
        .bind(queue)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Lease the next due job in `queue` for `worker_id`.
    ///
    /// Due jobs are pending jobs past `available_at` and running jobs whose
    /// lease expired with attempts remaining. Each claim increments the
    /// fencing token and the attempt count.
    pub async fn claim(
        &self,
        queue: &str,
        worker_id: &str,
    ) -> Result<Option<LeasedJob>, sqlx::Error> {
        let failed = self.fail_exhausted_leases(queue).await?;
        if failed > 0 {
            warn!(
                queue,
                failed, "failed jobs whose lease expired on their final attempt"
            );
        }

        let row = sqlx::query(
            r#"
            UPDATE background_jobs AS j
            SET status = 'running',
                lease_owner = $2,
                lease_expires_at = NOW() + ($3::bigint * INTERVAL '1 millisecond'),
                fencing_token = j.fencing_token + 1,
                attempts = j.attempts + 1,
                updated_at = NOW()
            WHERE j.id = (
                SELECT id FROM background_jobs
                WHERE queue = $1
                  AND (
                        (status = 'pending' AND available_at <= NOW())
                     OR (status = 'running' AND lease_expires_at < NOW() AND attempts < max_attempts)
                  )
                ORDER BY available_at, id
                LIMIT 1
                FOR UPDATE SKIP LOCKED
            )
            RETURNING j.id, j.queue, j.kind, j.payload::text AS payload, j.attempts,
                      j.max_attempts, j.fencing_token, j.lease_expires_at
            "#,
        )
        .bind(queue)
        .bind(worker_id)
        .bind(duration_ms(self.config.lease))
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else { return Ok(None) };
        let payload: String = row.try_get("payload")?;
        let job = LeasedJob {
            id: row.try_get("id")?,
            queue: row.try_get("queue")?,
            kind: row.try_get("kind")?,
            payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
            attempt: row.try_get("attempts")?,
            max_attempts: row.try_get("max_attempts")?,
            fencing_token: row.try_get("fencing_token")?,
            lease_expires_at: row.try_get("lease_expires_at")?,
        };
        debug!(
            job_id = job.id,
            token = job.fencing_token,
            attempt = job.attempt,
            worker_id,
            "claimed job"
        );
        Ok(Some(job))
    }

    /// Extend the lease (heartbeat).
    pub async fn extend(&self, job: &LeasedJob) -> Result<(), LeaseError> {
        let result = sqlx::query(
            r#"
            UPDATE background_jobs
            SET lease_expires_at = NOW() + ($3::bigint * INTERVAL '1 millisecond'),
                updated_at = NOW()
            WHERE id = $1 AND fencing_token = $2 AND status = 'running'
            "#,
        )
        .bind(job.id)
        .bind(job.fencing_token)
        .bind(duration_ms(self.config.lease))
        .execute(&self.pool)
        .await?;
        fenced(result.rows_affected())
    }

    /// Record success. Fails with [`LeaseError::Lost`] for a stale token.
    pub async fn complete(&self, job: &LeasedJob) -> Result<(), LeaseError> {
        let result = sqlx::query(
            r#"
            UPDATE background_jobs
            SET status = 'succeeded',
                lease_owner = NULL,
                lease_expires_at = NULL,
                last_error = NULL,
                completed_at = NOW(),
                updated_at = NOW()
            WHERE id = $1 AND fencing_token = $2 AND status = 'running'
            "#,
        )
        .bind(job.id)
        .bind(job.fencing_token)
        .execute(&self.pool)
        .await?;
        fenced(result.rows_affected())
    }

    /// Record a handler failure: retry with backoff or fail terminally.
    pub async fn fail(&self, job: &LeasedJob, error: &JobError) -> Result<FailOutcome, LeaseError> {
        let outcome = if error.is_retryable() && job.attempt < job.max_attempts {
            FailOutcome::Retrying
        } else {
            FailOutcome::Failed
        };
        let result = match outcome {
            FailOutcome::Retrying => {
                sqlx::query(
                    r#"
                    UPDATE background_jobs
                    SET status = 'pending',
                        last_error = $3,
                        lease_owner = NULL,
                        lease_expires_at = NULL,
                        available_at = NOW() + ($4::bigint * INTERVAL '1 millisecond'),
                        updated_at = NOW()
                    WHERE id = $1 AND fencing_token = $2 AND status = 'running'
                    "#,
                )
                .bind(job.id)
                .bind(job.fencing_token)
                .bind(error.message())
                .bind(duration_ms(self.config.retry_delay(job.attempt)))
                .execute(&self.pool)
                .await?
            }
            FailOutcome::Failed => {
                sqlx::query(
                    r#"
                    UPDATE background_jobs
                    SET status = 'failed',
                        last_error = $3,
                        lease_owner = NULL,
                        lease_expires_at = NULL,
                        completed_at = NOW(),
                        updated_at = NOW()
                    WHERE id = $1 AND fencing_token = $2 AND status = 'running'
                    "#,
                )
                .bind(job.id)
                .bind(job.fencing_token)
                .bind(error.message())
                .execute(&self.pool)
                .await?
            }
        };
        fenced(result.rows_affected()).map(|()| outcome)
    }

    /// Claim one job from `queue`, run `handler` while heartbeating the
    /// lease, and record the outcome. Returns `None` when nothing is due.
    ///
    /// Dropping the returned future (e.g. the worker task is aborted) stops
    /// the heartbeat, so the lease expires and another worker reclaims the
    /// job.
    pub async fn run_one<F, Fut>(
        &self,
        queue: &str,
        worker_id: &str,
        handler: F,
    ) -> Result<Option<RunReport>, sqlx::Error>
    where
        F: FnOnce(LeasedJob) -> Fut,
        Fut: Future<Output = Result<(), JobError>>,
    {
        let Some(job) = self.claim(queue, worker_id).await? else {
            return Ok(None);
        };

        let heartbeat = {
            let store = self.clone();
            let job = job.clone();
            let interval = (self.config.lease / 3).max(Duration::from_millis(10));
            AbortOnDrop(tokio::spawn(async move {
                loop {
                    tokio::time::sleep(interval).await;
                    if let Err(e) = store.extend(&job).await {
                        warn!(job_id = job.id, error = %e, "lease heartbeat failed");
                        if matches!(e, LeaseError::Lost) {
                            return;
                        }
                    }
                }
            }))
        };

        let result = handler(job.clone()).await;
        drop(heartbeat);

        let outcome = match result {
            Ok(()) => self.complete(&job).await.map(|()| None),
            Err(error) => self.fail(&job, &error).await.map(Some),
        };
        if let Err(LeaseError::Lost) = &outcome {
            warn!(
                job_id = job.id,
                token = job.fencing_token,
                "job outcome discarded: lease lost"
            );
        }
        Ok(Some(RunReport {
            job_id: job.id,
            fencing_token: job.fencing_token,
            outcome,
        }))
    }
}

fn fenced(rows_affected: u64) -> Result<(), LeaseError> {
    if rows_affected == 1 {
        Ok(())
    } else {
        Err(LeaseError::Lost)
    }
}

fn duration_ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_delay_doubles_and_caps() {
        let config = LeaseConfig {
            lease: Duration::from_secs(1),
            retry_base_delay: Duration::from_millis(100),
            retry_max_delay: Duration::from_millis(500),
        };
        assert_eq!(config.retry_delay(1), Duration::from_millis(100));
        assert_eq!(config.retry_delay(2), Duration::from_millis(200));
        assert_eq!(config.retry_delay(3), Duration::from_millis(400));
        assert_eq!(config.retry_delay(4), Duration::from_millis(500));
        assert_eq!(config.retry_delay(i32::MAX), Duration::from_millis(500));
    }

    #[test]
    fn sqlx_errors_are_classified() {
        assert!(JobError::from(sqlx::Error::PoolTimedOut).is_retryable());
        assert!(JobError::from(sqlx::Error::Io(std::io::Error::other("reset"))).is_retryable());
        assert!(!JobError::from(sqlx::Error::RowNotFound).is_retryable());
        assert!(!JobError::from(sqlx::Error::ColumnNotFound("x".into())).is_retryable());
    }

    #[test]
    fn new_job_clamps_max_attempts() {
        let job = NewJob::new("q", "k", serde_json::json!({})).max_attempts(0);
        assert_eq!(job.max_attempts, 1);
    }
}
