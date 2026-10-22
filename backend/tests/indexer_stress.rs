//! Concurrent ledger-ingestion workload against real PostgreSQL and Redis.
//!
//! Run explicitly with:
//! `cargo test --features integration-tests --test indexer_stress -- --ignored --nocapture`

use predifi_backend::{
    db::{self, PoolCreatedEvent, PredictionPlacedEvent},
    redis_cache::RedisCache,
    rpc::{
        self, MockRpcProvider, ProviderHealth, RpcBudget, RpcError, RpcPriority, RpcRequest,
        RpcResponse, RpcRouter,
    },
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
const RPC_MAX_ATTEMPTS: usize = 3;
const RPC_BUDGET_PER_WINDOW: u32 = 5;
const RPC_WINDOW: Duration = Duration::from_secs(1);
const RPC_HEALTH_FAILURE_THRESHOLD: u32 = 2;
const RPC_HEALTH_COOLDOWN: Duration = Duration::from_millis(250);
const RPC_MAX_RETRY_DELAY: Duration = Duration::from_millis(50);

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

/// Builds a router with a deterministic provider ordering and shared budget so
/// fault-injection scenarios are reproducible.
fn fault_injection_router(providers: Vec<MockRpcProvider>) -> RpcRouter {
    let budget = RpcBudget::new(RPC_BUDGET_PER_WINDOW, RPC_WINDOW);
    RpcRouter::builder()
        .budget(budget)
        .max_attempts(RPC_MAX_ATTEMPTS)
        .max_retry_delay(RPC_MAX_RETRY_DELAY)
        .health_failure_threshold(RPC_HEALTH_FAILURE_THRESHOLD)
        .health_cooldown(RPC_HEALTH_COOLDOWN)
        .providers(providers)
        .build()
        .expect("build fault-injection RPC router")
}

fn sample_request(idempotency_key: &str, priority: RpcPriority) -> RpcRequest {
    RpcRequest::new("getLatestLedger", serde_json::json!({}))
        .with_priority(priority)
        .with_idempotency_key(idempotency_key)
}

#[tokio::test]
#[ignore = "Scheduled fault-injection test; run with --ignored"]
async fn rpc_throttling_produces_bounded_retries_without_duplicate_submission() {
    let primary = MockRpcProvider::new("primary")
        .with_throttle_after(1)
        .with_throttle_retry_after(Duration::from_millis(5));
    let fallback = MockRpcProvider::new("fallback");
    let router = fault_injection_router(vec![primary.clone(), fallback.clone()]);

    let request = sample_request("tx-throttle-1", RpcPriority::UserTransaction);
    let outcome = router
        .execute(request.clone())
        .await
        .expect("router must eventually succeed via fallback");

    assert_eq!(outcome.provider, "fallback", "fallback ordering must be honored");
    assert!(
        outcome.attempts <= RPC_MAX_ATTEMPTS,
        "retries must be bounded: {} > {RPC_MAX_ATTEMPTS}",
        outcome.attempts
    );
    assert_eq!(
        primary.throttled_requests(),
        1,
        "primary should observe exactly one throttled attempt"
    );
    assert_eq!(
        fallback.successful_requests(),
        1,
        "fallback must serve the request exactly once"
    );

    // Replaying the same idempotency key must not submit the transaction twice.
    let replay = router
        .execute(request)
        .await
        .expect("idempotent replay must succeed");
    assert_eq!(
        replay.provider, "fallback",
        "replay must reuse the recorded provider"
    );
    assert_eq!(
        fallback.successful_requests(),
        1,
        "idempotency key must prevent duplicate transaction submission"
    );
    assert_eq!(
        router.submission_count("tx-throttle-1"),
        1,
        "router must record exactly one submission per idempotency key"
    );
}

#[tokio::test]
#[ignore = "Scheduled fault-injection test; run with --ignored"]
async fn rpc_provider_outage_transitions_health_and_fails_over() {
    let primary = MockRpcProvider::new("primary").with_outage();
    let secondary = MockRpcProvider::new("secondary");
    let router = fault_injection_router(vec![primary.clone(), secondary.clone()]);

    let first = router
        .execute(sample_request("tx-outage-1", RpcPriority::Indexer))
        .await
        .expect("router must fail over to a healthy provider");
    assert_eq!(first.provider, "secondary");
    assert!(
        first.attempts <= RPC_MAX_ATTEMPTS,
        "outage retries must be bounded"
    );

    let health = router.provider_health("primary");
    assert_eq!(
        health,
        ProviderHealth::Unhealthy,
        "primary must be marked unhealthy after repeated failures"
    );
    assert_eq!(
        router.provider_health("secondary"),
        ProviderHealth::Healthy
    );

    // While the primary is cooling down, subsequent requests must skip it.
    let second = router
        .execute(sample_request("tx-outage-2", RpcPriority::Indexer))
        .await
        .expect("router must keep serving during cooldown");
    assert_eq!(second.provider, "secondary");
    assert_eq!(
        primary.attempted_requests(),
        RPC_HEALTH_FAILURE_THRESHOLD as usize,
        "unhealthy provider must not be retried beyond the failure threshold"
    );

    // After the cooldown elapses the primary is probed again and recovers.
    primary.recover();
    sleep(RPC_HEALTH_COOLDOWN + Duration::from_millis(25)).await;
    let third = router
        .execute(sample_request("tx-outage-3", RpcPriority::Indexer))
        .await
        .expect("router must recover the primary provider");
    assert_eq!(third.provider, "primary", "recovered provider must be preferred");
    assert_eq!(
        router.provider_health("primary"),
        ProviderHealth::Healthy,
        "health must transition back to healthy after a successful probe"
    );
}

#[tokio::test]
#[ignore = "Scheduled fault-injection test; run with --ignored"]
async fn rpc_budget_exhaustion_returns_bounded_error() {
    let primary = MockRpcProvider::new("primary").with_throttle_after(0);
    let router = fault_injection_router(vec![primary.clone()]);

    let mut errors = 0;
    for attempt in 0..(RPC_BUDGET_PER_WINDOW as usize + 2) {
        let request = sample_request(&format!("tx-budget-{attempt}"), RpcPriority::Indexer);
        match router.execute(request).await {
            Ok(_) => {}
            Err(RpcError::BudgetExhausted { .. }) => errors += 1,
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    assert!(
        errors > 0,
        "exhausting the per-network budget must surface a bounded error"
    );
    assert!(
        primary.attempted_requests() <= RPC_BUDGET_PER_WINDOW as usize,
        "provider must not receive more requests than the budget allows"
    );
}

#[tokio::test]
#[ignore = "Scheduled fault-injection test; run with --ignored"]
async fn rpc_provider_disagreement_is_detected_and_not_resubmitted() {
    let primary = MockRpcProvider::new("primary")
        .with_response(RpcResponse::ledger(100, "hash-a"));
    let secondary = MockRpcProvider::new("secondary")
        .with_response(RpcResponse::ledger(100, "hash-b"));
    let router = fault_injection_router(vec![primary.clone(), secondary.clone()]);

    let request = sample_request("tx-disagree-1", RpcPriority::UserTransaction);
    let outcome = router
        .execute(request.clone())
        .await
        .expect("router must return the primary response");

    assert_eq!(outcome.provider, "primary");
    assert!(
        outcome.disagreement.is_some(),
        "router must flag disagreement between providers"
    );
    assert_eq!(
        outcome.disagreement.as_ref().map(|d| d.providers.clone()),
        Some(vec!["primary".to_string(), "secondary".to_string()])
    );

    // Disagreement must not trigger a duplicate transaction submission.
    let replay = router
        .execute(request)
        .await
        .expect("idempotent replay must succeed");
    assert_eq!(replay.provider, "primary");
    assert_eq!(
        router.submission_count("tx-disagree-1"),
        1,
        "disagreement handling must not resubmit the transaction"
    );
    assert_eq!(primary.successful_requests(), 1);
    assert_eq!(secondary.successful_requests(), 1);
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
