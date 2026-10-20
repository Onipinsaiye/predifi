//! Concurrent ledger-ingestion workload against real PostgreSQL and Redis.
//!
//! Run explicitly with:
//! `cargo test --features integration-tests --test indexer_stress -- --ignored --nocapture`

use predifi_backend::{
    db::{self, PoolCreatedEvent, PredictionPlacedEvent},
    redis_cache::RedisCache,
    worker::queue::{Job, JobQueue},
};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::{postgres::Postgres, redis::Redis};
use tokio::{sync::Barrier, task::JoinHandle, time::sleep};

const FIXTURE_POOL_COUNT: i64 = 100;
const FIXTURE_PREDICTIONS_PER_POOL: i64 = 100;
const WRITER_COUNT: usize = 8;
const READER_COUNT: usize = 8;
const WORKER_COUNT: usize = 4;
const WRITES_PER_PRODUCER: usize = 250;
const READS_PER_CONSUMER: usize = 40;
const FIXTURE_POOL_ID_START: i64 = 900_000;
const FIXTURE_LEDGER_START: u64 = 50_000_000;
const QUERY_P95_LIMIT: Duration = Duration::from_millis(500);
const QUERY_P99_LIMIT: Duration = Duration::from_millis(1_500);
const BACKLOG_RECOVERY_LIMIT: Duration = Duration::from_secs(60);

#[derive(Debug, Serialize, Deserialize)]
struct StressPrediction {
    pool_id: u64,
    user_address: String,
    outcome: i32,
    amount: i64,
}

fn percentile(sorted: &[Duration], percentile: f64) -> Duration {
    let index = ((sorted.len() as f64 * percentile / 100.0).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len().saturating_sub(1));
    sorted.get(index).copied().unwrap_or_default()
}

fn fixture_pool(pool_id: u64) -> PoolCreatedEvent {
    PoolCreatedEvent {
        pool_id,
        creator: "stress-fixture-creator".to_string(),
        end_time: 2_000_000_000,
        token: "XLM".to_string(),
        category: "stress-fixture".to_string(),
        description: format!("Fixture pool {pool_id}"),
    }
}

#[tokio::test]
#[ignore = "Scheduled Docker-backed load test; run with --ignored"]
async fn concurrent_indexer_writes_reads_and_workers_stay_consistent() {
    let postgres = Postgres::default()
        .start()
        .await
        .expect("start postgres container");
    let postgres_port = postgres
        .get_host_port_ipv4(5432)
        .await
        .expect("postgres port");
    let database_url = format!("******127.0.0.1:{postgres_port}/postgres");
    let db = PgPoolOptions::new()
        .max_connections(24)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database_url)
        .await
        .expect("connect to postgres");
    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .expect("apply migrations");

    let redis = Redis::default()
        .start()
        .await
        .expect("start redis container");
    let redis_port = redis.get_host_port_ipv4(6379).await.expect("redis port");
    let cache = RedisCache::new(&format!("redis://127.0.0.1:{redis_port}")).await;
    assert!(cache.is_available(), "Redis connection must be available");
    assert!(cache.ping().await, "Redis health check must succeed");

    for offset in 0..FIXTURE_POOL_COUNT {
        db::insert_pool_from_event(&db, &fixture_pool((FIXTURE_POOL_ID_START + offset) as u64))
            .await
            .expect("insert repeatable ledger pool fixture");
    }

    let fixture_prediction_count = FIXTURE_POOL_COUNT * FIXTURE_PREDICTIONS_PER_POOL;
    sqlx::query(
        r#"
        INSERT INTO predictions (pool_id, user_address, outcome, amount)
        SELECT $1 + ((event_number - 1) / $2),
               'fixture-user-' || event_number::text,
               (event_number % 2)::integer,
               10
        FROM generate_series(1, $3) AS fixture(event_number)
        "#,
    )
    .bind(FIXTURE_POOL_ID_START)
    .bind(FIXTURE_PREDICTIONS_PER_POOL)
    .bind(fixture_prediction_count)
    .execute(&db)
    .await
    .expect("seed deterministic ledger prediction fixture");
    sqlx::query("UPDATE pools SET total_stake = $1 WHERE pool_id BETWEEN $2 AND $3")
        .bind(FIXTURE_PREDICTIONS_PER_POOL * 10)
        .bind(FIXTURE_POOL_ID_START)
        .bind(FIXTURE_POOL_ID_START + FIXTURE_POOL_COUNT - 1)
        .execute(&db)
        .await
        .expect("set fixture pool totals");

    cache.set("pools:stress", &"fixture", 60).await;
    cache.set("stats:stress", &"fixture", 60).await;

    let deadlocks_before: i64 = sqlx::query_scalar(
        "SELECT deadlocks FROM pg_stat_database WHERE datname = current_database()",
    )
    .fetch_one(&db)
    .await
    .expect("read initial deadlock count");

    let queue = JobQueue::with_defaults();
    let producers_done = Arc::new(AtomicBool::new(false));
    let maximum_backlog = Arc::new(AtomicUsize::new(0));
    let deadlocks_observed = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(WRITER_COUNT + READER_COUNT + WORKER_COUNT));
    let mut producer_handles: Vec<JoinHandle<()>> = Vec::with_capacity(WRITER_COUNT);
    let mut reader_handles: Vec<JoinHandle<Vec<Duration>>> = Vec::with_capacity(READER_COUNT);
    let mut worker_handles: Vec<JoinHandle<Result<(), String>>> = Vec::with_capacity(WORKER_COUNT);

    for producer_id in 0..WRITER_COUNT {
        let barrier = barrier.clone();
        let queue = queue.clone();
        let cache = cache.clone();
        let db = db.clone();
        let maximum_backlog = maximum_backlog.clone();
        producer_handles.push(tokio::spawn(async move {
            barrier.wait().await;

            for event_number in 0..WRITES_PER_PRODUCER {
                let ledger = FIXTURE_LEDGER_START
                    + (producer_id * WRITES_PER_PRODUCER + event_number) as u64;
                let event = StressPrediction {
                    pool_id: (FIXTURE_POOL_ID_START
                        + ((producer_id * WRITES_PER_PRODUCER + event_number) % 20) as i64)
                        as u64,
                    user_address: format!("G{:055}", ledger),
                    outcome: (event_number % 2) as i32,
                    amount: 1,
                };
                let job = Job {
                    id: format!("ledger-{ledger}-prediction"),
                    kind: "prediction_placed".to_string(),
                    payload: serde_json::to_string(&event).expect("serialize ledger fixture"),
                    attempts: 0,
                    ledger: Some(ledger),
                };
                assert!(queue.enqueue(job), "fixture job IDs must be unique");
                maximum_backlog.fetch_max(queue.health().pending_retries, Ordering::Relaxed);

                if event_number % 25 == 0 {
                    cache.invalidate_pools_cache().await;
                    cache.invalidate_stats_cache().await;
                }
            }

            db::insert_pool_from_event(&db, &fixture_pool(FIXTURE_POOL_ID_START as u64))
                .await
                .expect("replay duplicate pool-created event");
        }));
    }

    for reader_id in 0..READER_COUNT {
        let barrier = barrier.clone();
        let db = db.clone();
        let cache = cache.clone();
        reader_handles.push(tokio::spawn(async move {
            barrier.wait().await;
            let mut latencies = Vec::with_capacity(READS_PER_CONSUMER);

            for read_number in 0..READS_PER_CONSUMER {
                let started = Instant::now();
                let pools = db::get_active_pools(&db, "new", None, 20, 0)
                    .await
                    .expect("query active pools");
                assert!(!pools.is_empty(), "fixture pools should be readable");

                let pool_id = FIXTURE_POOL_ID_START
                    + ((reader_id * READS_PER_CONSUMER + read_number) % 20) as i64;
                db::get_leaderboard_extended(&db, "volume", "all", Some(pool_id), 20, 0)
                    .await
                    .expect("query concurrent leaderboard");
                db::get_protocol_stats(&db, None, Some("active"))
                    .await
                    .expect("query concurrent protocol stats");
                let _: Option<String> = cache.get("pools:stress").await;
                let _: Option<String> = cache.get("stats:stress").await;

                latencies.push(started.elapsed());
            }

            latencies
        }));
    }

    for _ in 0..WORKER_COUNT {
        let barrier = barrier.clone();
        let queue = queue.clone();
        let db = db.clone();
        let producers_done = producers_done.clone();
        let maximum_backlog = maximum_backlog.clone();
        let deadlocks_observed = deadlocks_observed.clone();
        worker_handles.push(tokio::spawn(async move {
            barrier.wait().await;
            loop {
                if let Some(job) = queue.dequeue_ready() {
                    maximum_backlog.fetch_max(queue.health().pending_retries, Ordering::Relaxed);
                    let db = db.clone();
                    let deadlocks_observed = deadlocks_observed.clone();
                    queue
                        .process_with_retry(job, move |job| {
                            let db = db.clone();
                            let deadlocks_observed = deadlocks_observed.clone();
                            async move {
                                let event: StressPrediction = serde_json::from_str(&job.payload)
                                    .map_err(|error| error.to_string())?;
                                let projection = PredictionPlacedEvent {
                                    pool_id: event.pool_id,
                                    user_address: event.user_address,
                                    outcome: event.outcome,
                                    amount: event.amount,
                                };
                                db::insert_prediction_from_event_with_pool(&db, &projection)
                                    .await
                                    .map_err(|error| {
                                        if error
                                            .as_database_error()
                                            .and_then(|database_error| database_error.code())
                                            .as_deref()
                                            == Some("40P01")
                                        {
                                            deadlocks_observed.fetch_add(1, Ordering::Relaxed);
                                        }
                                        error.to_string()
                                    })
                            }
                        })
                        .await?;
                } else if producers_done.load(Ordering::Acquire)
                    && queue.health().pending_retries == 0
                {
                    break;
                } else {
                    sleep(Duration::from_millis(1)).await;
                }
            }
            Ok(())
        }));
    }

    let workload_started = Instant::now();
    for handle in producer_handles {
        handle.await.expect("producer task must complete");
    }
    producers_done.store(true, Ordering::Release);
    let backlog_recovery_started = Instant::now();

    for handle in worker_handles {
        handle
            .await
            .expect("worker task must complete")
            .expect("worker must process every ledger event");
    }
    let backlog_recovery_time = backlog_recovery_started.elapsed();

    let mut query_latencies = Vec::with_capacity(READER_COUNT * READS_PER_CONSUMER);
    for handle in reader_handles {
        query_latencies.extend(handle.await.expect("reader task must complete"));
    }
    query_latencies.sort_unstable();

    let p50 = percentile(&query_latencies, 50.0);
    let p95 = percentile(&query_latencies, 95.0);
    let p99 = percentile(&query_latencies, 99.0);
    let queue_health = queue.health();
    let expected_prediction_count =
        fixture_prediction_count + (WRITER_COUNT * WRITES_PER_PRODUCER) as i64;
    let expected_stake =
        fixture_prediction_count * 10 + (WRITER_COUNT * WRITES_PER_PRODUCER) as i64;
    let actual_prediction_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM predictions")
        .fetch_one(&db)
        .await
        .expect("count projected predictions");
    let pool_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pools WHERE pool_id BETWEEN $1 AND $2")
            .bind(FIXTURE_POOL_ID_START)
            .bind(FIXTURE_POOL_ID_START + FIXTURE_POOL_COUNT - 1)
            .fetch_one(&db)
            .await
            .expect("count projected pools");
    let creator_pool_count: i64 =
        sqlx::query_scalar("SELECT pools_created FROM creator_stats WHERE creator = $1")
            .bind("stress-fixture-creator")
            .fetch_one(&db)
            .await
            .expect("read creator projection count");
    let projected_stake: i64 = sqlx::query_scalar("SELECT SUM(total_stake)::bigint FROM pools")
        .fetch_one(&db)
        .await
        .expect("sum projected pool stakes");
    let prediction_stake: i64 = sqlx::query_scalar("SELECT SUM(amount)::bigint FROM predictions")
        .fetch_one(&db)
        .await
        .expect("sum projected prediction stakes");
    let protocol_stats = db::get_protocol_stats(&db, None, Some("active"))
        .await
        .expect("read final protocol projection");
    let deadlocks_after: i64 = sqlx::query_scalar(
        "SELECT deadlocks FROM pg_stat_database WHERE datname = current_database()",
    )
    .fetch_one(&db)
    .await
    .expect("read final deadlock count");
    let deadlocks = (deadlocks_after - deadlocks_before).max(0) as usize;

    eprintln!("=== Concurrent indexer stress test ===");
    eprintln!("fixture_pools:          {FIXTURE_POOL_COUNT}");
    eprintln!("fixture_predictions:    {fixture_prediction_count}");
    eprintln!(
        "concurrent_writes:      {}",
        WRITER_COUNT * WRITES_PER_PRODUCER
    );
    eprintln!("concurrent_readers:     {READER_COUNT}");
    eprintln!("background_workers:     {WORKER_COUNT}");
    eprintln!(
        "maximum_backlog:        {}",
        maximum_backlog.load(Ordering::Relaxed)
    );
    eprintln!("backlog_recovery:       {backlog_recovery_time:?}");
    eprintln!("total_workload_time:    {:?}", workload_started.elapsed());
    eprintln!("query_latency_p50:      {p50:?}");
    eprintln!("query_latency_p95:      {p95:?}");
    eprintln!("query_latency_p99:      {p99:?}");
    eprintln!("sql_deadlocks:          {deadlocks}");
    eprintln!("worker_jobs_processed:  {}", queue_health.jobs_processed);
    eprintln!(
        "worker_jobs_deadlettered: {}",
        queue_health.jobs_dead_lettered
    );

    assert_eq!(
        deadlocks, 0,
        "Postgres reported a deadlock during the workload"
    );
    assert_eq!(
        deadlocks_observed.load(Ordering::Relaxed),
        0,
        "a worker encountered a deadlock"
    );
    assert_eq!(
        pool_count, FIXTURE_POOL_COUNT,
        "duplicate pool projections found"
    );
    assert_eq!(
        creator_pool_count, FIXTURE_POOL_COUNT,
        "duplicate ledger delivery must not inflate creator pool totals"
    );
    assert_eq!(
        actual_prediction_count, expected_prediction_count,
        "all fixture and concurrent prediction events must be projected once"
    );
    assert_eq!(projected_stake, expected_stake);
    assert_eq!(prediction_stake, expected_stake);
    assert_eq!(projected_stake, prediction_stake);
    assert_eq!(protocol_stats.total_bets, expected_prediction_count);
    assert_eq!(
        queue_health.jobs_processed,
        (WRITER_COUNT * WRITES_PER_PRODUCER) as u64
    );
    assert_eq!(queue_health.jobs_dead_lettered, 0);
    assert_eq!(queue_health.pending_retries, 0, "worker backlog must drain");
    assert!(
        backlog_recovery_time <= BACKLOG_RECOVERY_LIMIT,
        "backlog recovery exceeded {BACKLOG_RECOVERY_LIMIT:?}: {backlog_recovery_time:?}"
    );
    assert!(
        p95 <= QUERY_P95_LIMIT,
        "p95 exceeded {QUERY_P95_LIMIT:?}: {p95:?}"
    );
    assert!(
        p99 <= QUERY_P99_LIMIT,
        "p99 exceeded {QUERY_P99_LIMIT:?}: {p99:?}"
    );
    assert!(
        cache.ping().await,
        "Redis must remain available after the workload"
    );
    let cached_pools: Option<String> = cache.get("pools:stress").await;
    let cached_stats: Option<String> = cache.get("stats:stress").await;
    assert!(
        cached_pools.is_none(),
        "pool cache invalidation did not complete"
    );
    assert!(
        cached_stats.is_none(),
        "stats cache invalidation did not complete"
    );

    db.close().await;
    drop(cache);
    drop(redis);
    drop(postgres);
}
