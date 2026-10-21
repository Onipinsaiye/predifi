//! End-to-End Capacity Targets & Load Test (Issue #132)
//!
//! Run with:
//! $env:RUN_CAPACITY_TEST="1"; cargo test --test load_tests -- --nocapture

use reqwest::Client;
use serde_json::json;
use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;

const BASE_URL: &str = "http://localhost:3000/api/v1";
const TEST_ADDRESS: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA1"; // From seed.rs

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
        println!("\n--- Phase: {} ({} concurrent users) ---", phase_name, concurrency);
        run_phase(client.clone(), concurrency, duration).await;
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
                    if let Ok(res) = client.get(format!("{}/pools?status=active&limit=20", BASE_URL))
                        .header("X-Forwarded-For", &fake_ip)
                        .send().await 
                    {
                        success = res.status().is_success();
                    }
                } else if workflow_dice < 75 {
                    // Workflow 2: Prediction Prep (30%) - concurrent pool details + odds
                    let pool_id = (worker_id % 10) + 1; // Hitting seeded pools 1-10
                    let p1 = client.get(format!("{}/pools/{}", BASE_URL, pool_id)).header("X-Forwarded-For", &fake_ip).send();
                    let p2 = client.get(format!("{}/markets/{}/predictions", BASE_URL, pool_id)).header("X-Forwarded-For", &fake_ip).send();
                    
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
                    if let Ok(res) = client.post(format!("{}/indexer/prediction-placed", BASE_URL))
                        .header("X-Forwarded-For", &fake_ip)
                        .json(&payload)
                        .send().await
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
                    if let Ok(res) = client.post(format!("{}/indexer/claim", BASE_URL))
                        .header("X-Forwarded-For", &fake_ip)
                        .json(&payload)
                        .send().await
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