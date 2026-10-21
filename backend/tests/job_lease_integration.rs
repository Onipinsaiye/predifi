//! Postgres integration tests for leased background jobs (#65).
//!
//! Requires `DATABASE_URL`; each test is skipped when it is unset. Every test
//! uses its own queue name, so tests can run concurrently against one
//! database.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use predifi_backend::worker::lease::{
    FailOutcome, JobError, JobStore, LeaseConfig, LeaseError, NewJob,
};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

async fn pool() -> Option<PgPool> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL not set; skipping job lease integration test");
        return None;
    };
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    // Tests start in parallel; serialise the DDL so concurrent
    // `CREATE TABLE IF NOT EXISTS` calls do not race.
    let migration = format!(
        "BEGIN; SELECT pg_advisory_xact_lock(4813); {} COMMIT;",
        include_str!("../migrations/013_create_background_jobs.sql")
    );
    sqlx::raw_sql(&migration)
        .execute(&pool)
        .await
        .expect("apply background_jobs migration");
    Some(pool)
}

fn queue_name() -> String {
    format!("test-{}", uuid::Uuid::new_v4())
}

fn config(lease_ms: u64) -> LeaseConfig {
    LeaseConfig {
        lease: Duration::from_millis(lease_ms),
        retry_base_delay: Duration::ZERO,
        retry_max_delay: Duration::ZERO,
    }
}

async fn job_row(pool: &PgPool, id: i64) -> (String, i32, i64, Option<String>) {
    let row = sqlx::query(
        "SELECT status, attempts, fencing_token, last_error FROM background_jobs WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("job row");
    (
        row.get("status"),
        row.get("attempts"),
        row.get("fencing_token"),
        row.get("last_error"),
    )
}

/// Competing workers drain a queue; every job is completed exactly once.
#[tokio::test]
async fn competing_workers_complete_each_job_once() {
    let Some(pool) = pool().await else { return };
    let store = JobStore::new(pool.clone(), config(5_000));
    let queue = queue_name();

    let mut ids = Vec::new();
    for i in 0..60 {
        let id = store
            .enqueue(&NewJob::new(&queue, "test", serde_json::json!({ "n": i })))
            .await
            .expect("enqueue")
            .expect("new job");
        ids.push(id);
    }

    let executions: Arc<Mutex<HashMap<i64, usize>>> = Arc::default();
    let mut workers = Vec::new();
    for w in 0..8 {
        let store = store.clone();
        let queue = queue.clone();
        let executions = executions.clone();
        workers.push(tokio::spawn(async move {
            let worker_id = format!("worker-{w}");
            loop {
                let executions = executions.clone();
                let report = store
                    .run_one(&queue, &worker_id, |job| async move {
                        *executions.lock().unwrap().entry(job.id).or_default() += 1;
                        tokio::time::sleep(Duration::from_millis(2)).await;
                        Ok(())
                    })
                    .await
                    .expect("run_one");
                match report {
                    Some(report) => assert!(report.outcome.is_ok(), "{report:?}"),
                    None => break,
                }
            }
        }));
    }
    for worker in workers {
        worker.await.expect("worker panicked");
    }

    let executions = executions.lock().unwrap().clone();
    assert_eq!(executions.len(), ids.len());
    for id in &ids {
        assert_eq!(executions.get(id), Some(&1), "job {id} executed more than once");
        let (status, attempts, token, _) = job_row(&pool, *id).await;
        assert_eq!((status.as_str(), attempts, token), ("succeeded", 1, 1));
    }
}

/// Killing the lease holder: the job is reclaimed after the lease expires,
/// the old holder's late writes are fenced off, and exactly one completion
/// is recorded.
#[tokio::test]
async fn killed_lease_holder_is_fenced_and_job_redelivered() {
    let Some(pool) = pool().await else { return };
    let store = JobStore::new(pool.clone(), config(300));
    let queue = queue_name();
    let id = store
        .enqueue(&NewJob::new(&queue, "test", serde_json::json!({})).max_attempts(3))
        .await
        .unwrap()
        .unwrap();

    // Worker A claims the job, then its task is killed mid-handler.
    let (claimed_tx, claimed_rx) = tokio::sync::oneshot::channel();
    let doomed = {
        let store = store.clone();
        let queue = queue.clone();
        tokio::spawn(async move {
            store
                .run_one(&queue, "worker-a", |job| async move {
                    let _ = claimed_tx.send(job);
                    std::future::pending::<Result<(), JobError>>().await
                })
                .await
        })
    };
    let stale = claimed_rx.await.expect("worker A claimed the job");
    assert_eq!((stale.id, stale.fencing_token, stale.attempt), (id, 1, 1));
    doomed.abort();
    let _ = doomed.await;

    // While A's lease is live nobody else can take the job.
    assert!(store.claim(&queue, "worker-b").await.unwrap().is_none());

    tokio::time::sleep(Duration::from_millis(450)).await;
    let fresh = store
        .claim(&queue, "worker-b")
        .await
        .unwrap()
        .expect("expired lease is reclaimable");
    assert_eq!((fresh.id, fresh.fencing_token, fresh.attempt), (id, 2, 2));

    // A comes back from the dead: every write it attempts is rejected.
    assert!(matches!(store.complete(&stale).await, Err(LeaseError::Lost)));
    assert!(matches!(store.extend(&stale).await, Err(LeaseError::Lost)));
    assert!(matches!(
        store.fail(&stale, &JobError::Terminal("late".into())).await,
        Err(LeaseError::Lost)
    ));

    store.complete(&fresh).await.expect("current holder completes");
    assert!(matches!(store.complete(&fresh).await, Err(LeaseError::Lost)));
    let (status, attempts, token, _) = job_row(&pool, id).await;
    assert_eq!((status.as_str(), attempts, token), ("succeeded", 2, 2));
}

/// The heartbeat keeps a long-running job leased past its initial lease.
#[tokio::test]
async fn heartbeat_keeps_long_job_leased() {
    let Some(pool) = pool().await else { return };
    let store = JobStore::new(pool.clone(), config(200));
    let queue = queue_name();
    let id = store
        .enqueue(&NewJob::new(&queue, "test", serde_json::json!({})))
        .await
        .unwrap()
        .unwrap();

    let runner = {
        let store = store.clone();
        let queue = queue.clone();
        tokio::spawn(async move {
            store
                .run_one(&queue, "worker-a", |_| async {
                    tokio::time::sleep(Duration::from_millis(700)).await;
                    Ok(())
                })
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(450)).await;
    assert!(
        store.claim(&queue, "worker-b").await.unwrap().is_none(),
        "a heartbeating lease must not be reclaimed"
    );
    let report = runner.await.unwrap().unwrap().expect("job ran");
    assert!(report.outcome.is_ok());
    let (status, attempts, _, _) = job_row(&pool, id).await;
    assert_eq!((status.as_str(), attempts), ("succeeded", 1));
}

/// A job whose holder keeps dying is redelivered at most `max_attempts`
/// times, then failed.
#[tokio::test]
async fn redelivery_is_bounded_by_max_attempts() {
    let Some(pool) = pool().await else { return };
    let store = JobStore::new(pool.clone(), config(150));
    let queue = queue_name();
    let id = store
        .enqueue(&NewJob::new(&queue, "test", serde_json::json!({})).max_attempts(2))
        .await
        .unwrap()
        .unwrap();

    for attempt in 1..=2 {
        let job = store.claim(&queue, "crashy").await.unwrap().expect("claim");
        assert_eq!(job.attempt, attempt);
        // Never completes; let the lease lapse.
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(store.claim(&queue, "crashy").await.unwrap().is_none());
    let (status, attempts, _, last_error) = job_row(&pool, id).await;
    assert_eq!((status.as_str(), attempts), ("failed", 2));
    assert_eq!(last_error.as_deref(), Some("lease expired on final attempt"));
}

/// Retryable errors go back to pending; terminal errors fail immediately.
#[tokio::test]
async fn retryable_and_terminal_errors() {
    let Some(pool) = pool().await else { return };
    let store = JobStore::new(pool.clone(), config(5_000));
    let queue = queue_name();
    let retry_id = store
        .enqueue(&NewJob::new(&queue, "retry", serde_json::json!({})).max_attempts(2))
        .await
        .unwrap()
        .unwrap();

    let report = store
        .run_one(&queue, "w", |_| async { Err(JobError::Retryable("upstream down".into())) })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(report.outcome.unwrap(), Some(FailOutcome::Retrying));
    assert_eq!(job_row(&pool, retry_id).await.0, "pending");

    // Second (final) attempt: retryable errors no longer retry.
    let report = store
        .run_one(&queue, "w", |_| async { Err(JobError::Retryable("still down".into())) })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(report.outcome.unwrap(), Some(FailOutcome::Failed));
    assert_eq!(job_row(&pool, retry_id).await.0, "failed");

    let terminal_id = store
        .enqueue(&NewJob::new(&queue, "terminal", serde_json::json!({})))
        .await
        .unwrap()
        .unwrap();
    let report = store
        .run_one(&queue, "w", |_| async { Err(JobError::Terminal("bad payload".into())) })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(report.outcome.unwrap(), Some(FailOutcome::Failed));
    let (status, attempts, _, last_error) = job_row(&pool, terminal_id).await;
    assert_eq!((status.as_str(), attempts), ("failed", 1));
    assert_eq!(last_error.as_deref(), Some("bad payload"));
}

/// Producers can enqueue the same logical job twice without duplicating it.
#[tokio::test]
async fn dedupe_key_suppresses_duplicate_enqueue() {
    let Some(pool) = pool().await else { return };
    let store = JobStore::new(pool, config(5_000));
    let queue = queue_name();
    let key = format!("evt-{}", uuid::Uuid::new_v4());
    let job = NewJob::new(&queue, "test", serde_json::json!({})).dedupe_key(&key);
    assert!(store.enqueue(&job).await.unwrap().is_some());
    assert!(store.enqueue(&job).await.unwrap().is_none());
}
