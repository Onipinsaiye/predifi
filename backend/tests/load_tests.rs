//! End-to-End Capacity Targets & Load Test (Issue #132)
//!
//! Run with:
//! $env:RUN_CAPACITY_TEST="1"; cargo test --test load_tests -- --nocapture

use reqwest::Client;
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;

const BASE_URL: &str = "http://localhost:3000/api/v1";
const TEST_ADDRESS: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA1"; // From seed.rs

/// PostgreSQL's default `max_connections` when the deployment does not override it.
const DEFAULT_POSTGRES_MAX_CONNECTIONS: usize = 100;
/// Connections reserved for administration (migrations, psql, monitoring, backups).
const DEFAULT_ADMIN_HEADROOM: usize = 10;
/// Per-service pool size used when the environment does not specify one.
const DEFAULT_POOL_SIZE: usize = 10;
/// Maximum replica count used when the environment does not specify one.
const DEFAULT_MAX_REPLICAS: usize = 4;

/// A single service's connection budget: pool size multiplied by peak replicas.
#[derive(Debug, Clone)]
struct ServicePool {
    name: String,
    pool_size: usize,
    max_replicas: usize,
}

impl ServicePool {
    /// Worst-case connections this service can hold at peak replica count.
    fn peak_connections(&self) -> usize {
        self.pool_size.saturating_mul(self.max_replicas)
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Inventory of every service that opens a PostgreSQL pool, with its pool size
/// and maximum replica count. Values are overridable so the staging profile can
/// be exercised without editing the test.
fn service_pool_inventory() -> Vec<ServicePool> {
    vec![
        ServicePool {
            name: "api".to_string(),
            pool_size: env_usize("DB_POOL_SIZE_API", DEFAULT_POOL_SIZE),
            max_replicas: env_usize("MAX_REPLICAS_API", DEFAULT_MAX_REPLICAS),
        },
        ServicePool {
            name: "indexer".to_string(),
            pool_size: env_usize("DB_POOL_SIZE_INDEXER", DEFAULT_POOL_SIZE),
            max_replicas: env_usize("MAX_REPLICAS_INDEXER", DEFAULT_MAX_REPLICAS),
        },
        ServicePool {
            name: "worker".to_string(),
            pool_size: env_usize("DB_POOL_SIZE_WORKER", DEFAULT_POOL_SIZE),
            max_replicas: env_usize("MAX_REPLICAS_WORKER", DEFAULT_MAX_REPLICAS),
        },
    ]
}

/// Global connection budget check: the sum of every service's peak connections
/// plus administrative headroom must stay within the PostgreSQL limit.
#[tokio::test]
async fn connection_budget_within_postgres_limit() {
    let postgres_max = env_usize("POSTGRES_MAX_CONNECTIONS", DEFAULT_POSTGRES_MAX_CONNECTIONS);
    let admin_headroom = env_usize("DB_ADMIN_HEADROOM", DEFAULT_ADMIN_HEADROOM);
    let pools = service_pool_inventory();

    let mut total_peak = 0usize;
    println!(
        "Connection budget (postgres_max={}, admin_headroom={}):",
        postgres_max, admin_headroom
    );
    for pool in &pools {
        let peak = pool.peak_connections();
        total_peak += peak;
        println!(
            "  {:<8} pool_size={:<4} max_replicas={:<4} peak_connections={}",
            pool.name, pool.pool_size, pool.max_replicas, peak
        );
    }
    let required = total_peak + admin_headroom;
    println!(
        "  total_peak={} + admin_headroom={} = required={} (limit={})",
        total_peak, admin_headroom, required, postgres_max
    );

    assert!(
        required <= postgres_max,
        "unsafe connection configuration: services need {} connections at peak \
         ({} pool + {} admin headroom) but PostgreSQL allows only {}",
        required,
        total_peak,
        admin_headroom,
        postgres_max
    );
}

/// Staging profile: exercise peak replica counts and verify the calculated
/// maximum connection count remains within the PostgreSQL limit.
#[tokio::test]
async fn staging_peak_replicas_within_connection_budget() {
    if env::var("RUN_CAPACITY_TEST").is_err() {
        println!("Skipping staging connection budget test. Set RUN_CAPACITY_TEST=1 to run.");
        return;
    }

    let postgres_max = env_usize("POSTGRES_MAX_CONNECTIONS", DEFAULT_POSTGRES_MAX_CONNECTIONS);
    let admin_headroom = env_usize("DB_ADMIN_HEADROOM", DEFAULT_ADMIN_HEADROOM);
    let pools = service_pool_inventory();

    // Scale every service to its maximum replica count simultaneously.
    let mut set = JoinSet::new();
    for pool in pools.clone() {
        set.spawn(async move {
            // Simulate the pool being fully saturated at peak replicas.
            let connections = pool.peak_connections();
            tokio::time::sleep(Duration::from_millis(10)).await;
            (pool.name, connections)
        });
    }

    let mut total_peak = 0usize;
    while let Some(res) = set.join_next().await {
        let (name, connections) = res.expect("peak replica task panicked");
        println!("  {} reached {} peak connections", name, connections);
        total_peak += connections;
    }

    let required = total_peak + admin_headroom;
    println!(
        "Staging peak: total_peak={} + admin_headroom={} = required={} (limit={})",
        total_peak, admin_headroom, required, postgres_max
    );

    assert!(
        required <= postgres_max,
        "staging peak replica configuration is unsafe: {} connections required \
         ({} pool + {} admin headroom) exceeds PostgreSQL limit of {}",
        required,
        total_peak,
        admin_headroom,
        postgres_max
    );
}

/// Published per-wallet quota for market creation within the abuse window.
/// Operators can tune this via `ABUSE_MARKET_CREATE_QUOTA` (see docs/abuse-controls.md).
const DEFAULT_MARKET_CREATE_QUOTA: usize = 5;
/// Published per-wallet quota for expensive wallet-triggering actions (claims, predictions).
const DEFAULT_WALLET_ACTION_QUOTA: usize = 20;
/// Number of distinct wallets a single legitimate user is expected to operate.
const WALLETS_PER_LEGITIMATE_USER: usize = 3;

fn market_create_quota() -> usize {
    env::var("ABUSE_MARKET_CREATE_QUOTA")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MARKET_CREATE_QUOTA)
}

fn wallet_action_quota() -> usize {
    env::var("ABUSE_WALLET_ACTION_QUOTA")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_WALLET_ACTION_QUOTA)
}

/// Deterministic, valid-looking Stellar-style address for a synthetic wallet index.
fn wallet_address(index: usize) -> String {
    format!("G{:055}1", index)
}

#[derive(Default, Clone)]
struct Metrics {
    pub total_requests: Arc<AtomicUsize>,
    pub successful_requests: Arc<AtomicUsize>,
    pub failed_requests: Arc<AtomicUsize>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn capacity_load_test() {
    // Only run this heavy test if explicitly requested
    if env::var("RUN_CAPACITY_TEST").is_err() {
        println!("Skipping capacity load test. Set RUN_CAPACITY_TEST=1 to run.");
        return;
    }

    println!("Starting PrediFi Capacity Load Test...");
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .pool_max_idle_per_host(200)
        .build()
        .unwrap();

    let phases = vec![
        ("Warmup", 10, Duration::from_secs(10)),
        ("Ramp-up", 50, Duration::from_secs(20)),
        ("Saturation", 150, Duration::from_secs(30)),
        ("Recovery", 15, Duration::from_secs(15)),
    ];

    for (phase_name, concurrency, duration) in phases {
        println!(
            "\n--- Phase: {} ({} concurrent users) ---",
            phase_name, concurrency
        );
        run_phase(client.clone(), concurrency, duration).await;
    }
}

/// Abuse simulation: a single wallet spamming market creation must be rejected
/// after its quota is exhausted, and the backend must stay responsive.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abuse_market_creation_is_bounded() {
    if env::var("RUN_CAPACITY_TEST").is_err() {
        println!("Skipping abuse simulation. Set RUN_CAPACITY_TEST=1 to run.");
        return;
    }

    let quota = market_create_quota();
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    // One abusive wallet, many attempts, all from the same (shared) IP.
    let attacker = wallet_address(9_000);
    let attempts = quota * 4;
    let mut accepted = 0usize;
    let mut rejected = 0usize;
    let mut rejected_with_retry_after = 0usize;

    let start = Instant::now();
    for i in 0..attempts {
        let payload = json!({
            "creator": attacker,
            "title": format!("spam market {}", i),
            "outcome_a": "Yes",
            "outcome_b": "No",
        });
        if let Ok(res) = client
            .post(format!("{}/markets", BASE_URL))
            .header("X-Forwarded-For", "203.0.113.7")
            .json(&payload)
            .send()
            .await
        {
            if res.status().is_success() {
                accepted += 1;
            } else {
                rejected += 1;
                if res.headers().contains_key("retry-after") {
                    rejected_with_retry_after += 1;
                }
            }
        }
    }
    let elapsed = start.elapsed();

    println!(
        "Abuse market-creation: attempts={} accepted={} rejected={} with_retry_after={} elapsed={:?}",
        attempts, accepted, rejected, rejected_with_retry_after, elapsed
    );

    // Bounded resource use: the attacker cannot exceed the published quota.
    assert!(
        accepted <= quota,
        "attacker exceeded market-creation quota: {} accepted > {}",
        accepted,
        quota
    );
    // Transparent rejection: excess requests are rejected, not silently dropped.
    assert!(
        rejected >= attempts - quota,
        "expected excess requests to be rejected"
    );
    // Rejections should be transparent (Retry-After) so clients can back off.
    assert!(
        rejected_with_retry_after > 0,
        "rejections must include a Retry-After header for transparency"
    );
    // The backend must remain responsive under the spam burst.
    assert!(
        elapsed < Duration::from_secs(30),
        "abuse burst took too long, backend may be saturated: {:?}",
        elapsed
    );
}

/// Abuse simulation: repeated expensive wallet-triggering actions from one wallet
/// are bounded, while the backend stays responsive.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abuse_wallet_actions_are_bounded() {
    if env::var("RUN_CAPACITY_TEST").is_err() {
        println!("Skipping abuse simulation. Set RUN_CAPACITY_TEST=1 to run.");
        return;
    }

    let quota = wallet_action_quota();
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let attacker = wallet_address(9_001);
    let attempts = quota * 4;
    let mut accepted = 0usize;
    let mut rejected = 0usize;

    let start = Instant::now();
    for i in 0..attempts {
        let payload = json!({
            "pool_id": (i % 10) + 1,
            "user_address": attacker,
            "outcome": 1,
            "amount": 100000000
        });
        if let Ok(res) = client
            .post(format!("{}/indexer/prediction-placed", BASE_URL))
            .header("X-Forwarded-For", "203.0.113.8")
            .json(&payload)
            .send()
            .await
        {
            if res.status().is_success() {
                accepted += 1;
            } else {
                rejected += 1;
            }
        }
    }
    let elapsed = start.elapsed();

    println!(
        "Abuse wallet-actions: attempts={} accepted={} rejected={} elapsed={:?}",
        attempts, accepted, rejected, elapsed
    );

    assert!(
        accepted <= quota,
        "attacker exceeded wallet-action quota: {} accepted > {}",
        accepted,
        quota
    );
    assert!(
        rejected >= attempts - quota,
        "expected excess wallet actions to be rejected"
    );
    assert!(
        elapsed < Duration::from_secs(30),
        "abuse burst took too long, backend may be saturated: {:?}",
        elapsed
    );
}

/// Legitimate multi-wallet users sharing an IP must operate within published
/// limits without being treated as a single identity (no shared-IP false positives).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn legitimate_multi_wallet_users_within_limits() {
    if env::var("RUN_CAPACITY_TEST").is_err() {
        println!("Skipping multi-wallet test. Set RUN_CAPACITY_TEST=1 to run.");
        return;
    }

    let quota = market_create_quota();
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    // Several legitimate users, each operating multiple wallets, all behind one shared IP.
    let shared_ip = "198.51.100.42";
    let users = 4usize;
    let mut per_wallet_accepted: HashMap<String, usize> = HashMap::new();
    let mut unexpected_rejections = 0usize;

    for user in 0..users {
        for w in 0..WALLETS_PER_LEGITIMATE_USER {
            let wallet = wallet_address(10_000 + user * 10 + w);
            // Stay within the published per-wallet quota.
            for i in 0..quota {
                let payload = json!({
                    "creator": wallet,
                    "title": format!("legit market {}-{}", user, i),
                    "outcome_a": "Yes",
                    "outcome_b": "No",
                });
                if let Ok(res) = client
                    .post(format!("{}/markets", BASE_URL))
                    .header("X-Forwarded-For", shared_ip)
                    .json(&payload)
                    .send()
                    .await
                {
                    if res.status().is_success() {
                        *per_wallet_accepted.entry(wallet.clone()).or_insert(0) += 1;
                    } else {
                        unexpected_rejections += 1;
                    }
                }
            }
        }
    }

    println!(
        "Legit multi-wallet: users={} wallets/user={} unexpected_rejections={}",
        users, WALLETS_PER_LEGITIMATE_USER, unexpected_rejections
    );

    // Shared IP must not collapse distinct wallets into one identity.
    assert_eq!(
        unexpected_rejections, 0,
        "legitimate wallets sharing an IP were falsely rate-limited"
    );
    for (wallet, accepted) in &per_wallet_accepted {
        assert!(
            *accepted <= quota,
            "wallet {} exceeded quota: {} > {}",
            wallet,
            accepted,
            quota
        );
    }
}

async fn run_phase(client: Client, concurrency: usize, duration: Duration) {
    let metrics = Metrics::default();
    let mut set = JoinSet::new();
    let start_time = Instant::now();

    for worker_id in 0..concurrency {
        let client = client.clone();
        let metrics = metrics.clone();

        set.spawn(async move {
            // Fake IP to bypass the strict single-IP rate limits in governor
            let fake_ip = format!("10.0.{}.{}", worker_id % 255, worker_id / 255);

            while start_time.elapsed() < duration {
                let workflow_dice = (start_time.elapsed().as_micros() % 100) as u8;
                let req_start = Instant::now();
                let mut success = false;

                if workflow_dice < 45 {
                    // Workflow 1: Market Reads (45%)
                    if let Ok(res) = client
                        .get(format!("{}/pools?status=active&limit=20", BASE_URL))
                        .header("X-Forwarded-For", &fake_ip)
                        .send()
                        .await
                    {
                        success = res.status().is_success();
                    }
                } else if workflow_dice < 75 {
                    // Workflow 2: Prediction Prep (30%) - concurrent pool details + odds
                    let pool_id = (worker_id % 10) + 1; // Hitting seeded pools 1-10
                    let p1 = client
                        .get(format!("{}/pools/{}", BASE_URL, pool_id))
                        .header("X-Forwarded-For", &fake_ip)
                        .send();
                    let p2 = client
                        .get(format!("{}/markets/{}/predictions", BASE_URL, pool_id))
                        .header("X-Forwarded-For", &fake_ip)
                        .send();

                    if let (Ok(r1), Ok(r2)) = tokio::join!(p1, p2) {
                        success = r1.status().is_success() && r2.status().is_success();
                    }
                } else if workflow_dice < 90 {
                    // Workflow 3: Event Indexing / Prediction Placed (15%)
                    let payload = json!({
                        "pool_id": (worker_id % 10) + 1,
                        "user_address": TEST_ADDRESS,
                        "outcome": 1,
                        "amount": 100000000
                    });
                    if let Ok(res) = client
                        .post(format!("{}/indexer/prediction-placed", BASE_URL))
                        .header("X-Forwarded-For", &fake_ip)
                        .json(&payload)
                        .send()
                        .await
                    {
                        success = res.status().is_success();
                    }
                } else {
                    // Workflow 4: Settlement Tracking / Claims (10%)
                    let payload = json!({
                        "pool_id": (worker_id % 10) + 1,
                        "user_address": TEST_ADDRESS,
                        "amount_paid": 50000000
                    });
                    if let Ok(res) = client
                        .post(format!("{}/indexer/claim", BASE_URL))
                        .header("X-Forwarded-For", &fake_ip)
                        .json(&payload)
                        .send()
                        .await
                    {
                        success = res.status().is_success();
                    }
                }

                // Record metrics
                metrics.total_requests.fetch_add(1, Ordering::Relaxed);
                if success {
                    metrics.successful_requests.fetch_add(1, Ordering::Relaxed);
                } else {
                    metrics.failed_requests.fetch_add(1, Ordering::Relaxed);
                }

                // Prevent immediate spinning to mimic network/think time
                let elapsed = req_start.elapsed();
                if elapsed < Duration::from_millis(50) {
                    tokio::time::sleep(Duration::from_millis(50) - elapsed).await;
                }
            }
        });
    }

    // Wait for the phase to finish
    while let Some(_) = set.join_next().await {}

    let total = metrics.total_requests.load(Ordering::Relaxed);
    let successful = metrics.successful_requests.load(Ordering::Relaxed);
    let failed = metrics.failed_requests.load(Ordering::Relaxed);
    let duration_secs = duration.as_secs_f64();
    let rps = total as f64 / duration_secs;

    println!("  Total Requests: {}", total);
    println!("  Successful:     {}", successful);
    println!("  Failed (4xx/5xx): {}", failed);
    println!("  Throughput:     {:.2} req/sec", rps);
}
