//! Redis integration tests for stampede-safe market reads (#60).
//!
//! Requires `REDIS_URL`; each test is skipped when it is unset. Several
//! `CoalescingCache` instances share one Redis to stand in for separate
//! backend replicas; each test uses its own namespace.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use predifi_backend::cache_coalesce::{CoalesceConfig, CoalesceError, CoalescingCache, Source};
use redis::aio::ConnectionManager;

async fn connection() -> Option<ConnectionManager> {
    let Ok(url) = std::env::var("REDIS_URL") else {
        eprintln!("REDIS_URL not set; skipping coalescing cache Redis test");
        return None;
    };
    let client = redis::Client::open(url).expect("valid REDIS_URL");
    Some(ConnectionManager::new(client).await.expect("connect to Redis"))
}

fn config() -> CoalesceConfig {
    CoalesceConfig {
        fresh_ttl: Duration::from_secs(5),
        stale_window: Duration::from_secs(5),
        lock_ttl: Duration::from_millis(600),
        refresh_timeout: Duration::from_millis(500),
        max_wait: Duration::from_secs(3),
        poll_interval: Duration::from_millis(10),
    }
}

/// `replicas` caches sharing Redis under one fresh namespace.
fn replicas(conn: &ConnectionManager, n: usize, config: CoalesceConfig) -> Vec<CoalescingCache> {
    let namespace = format!("test-{}", uuid::Uuid::new_v4());
    (0..n)
        .map(|_| CoalescingCache::redis(conn.clone(), &namespace, config))
        .collect()
}

async fn load(calls: Arc<AtomicUsize>, delay_ms: u64) -> Result<String, String> {
    let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    Ok(format!("pool-detail-v{n}"))
}

/// 200 simultaneous misses spread over 4 replicas and 3 hot pools refresh
/// each pool exactly once, and every caller gets the refreshed value within
/// the wait bound.
#[tokio::test]
async fn simultaneous_misses_across_replicas_refresh_once_per_key() {
    let Some(conn) = connection().await else { return };
    let caches = replicas(&conn, 4, config());
    let calls: Vec<_> = (0..3).map(|_| Arc::new(AtomicUsize::new(0))).collect();

    let started = Instant::now();
    let tasks: Vec<_> = (0..200)
        .map(|i| {
            let cache = caches[i % caches.len()].clone();
            let pool = i % 3;
            let calls = calls[pool].clone();
            tokio::spawn(async move {
                let key = format!("pool:{pool}");
                cache.get_or_refresh(&key, || load(calls, 100)).await
            })
        })
        .collect();
    let mut refreshed = 0;
    for task in tasks {
        let lookup = task.await.unwrap().expect("every caller gets a value");
        assert_eq!(lookup.value, "pool-detail-v1");
        if lookup.source == Source::Refreshed {
            refreshed += 1;
        }
    }
    for (pool, counter) in calls.iter().enumerate() {
        assert_eq!(counter.load(Ordering::SeqCst), 1, "pool {pool} refreshed more than once");
    }
    assert_eq!(refreshed, 3);
    assert!(started.elapsed() < config().max_wait, "{:?}", started.elapsed());
}

/// A replica killed mid-refresh leaves its lock behind; another replica
/// takes over once the lock expires instead of waiting forever.
#[tokio::test]
async fn refresh_owner_failure_recovers_after_lock_expiry() {
    let Some(conn) = connection().await else { return };
    let caches = replicas(&conn, 2, config());
    let calls = Arc::new(AtomicUsize::new(0));

    let doomed = {
        let cache = caches[0].clone();
        let calls = calls.clone();
        tokio::spawn(async move { cache.get_or_refresh("pool:9", || load(calls, 60_000)).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    doomed.abort();

    let started = Instant::now();
    let lookup = caches[1]
        .get_or_refresh("pool:9", || load(calls.clone(), 0))
        .await
        .expect("survivor recovers");
    assert_eq!(lookup.source, Source::Refreshed);
    assert_eq!(lookup.value, "pool-detail-v2");
    assert!(
        started.elapsed() < config().lock_ttl + Duration::from_millis(300),
        "recovery took {:?}",
        started.elapsed()
    );
}

/// Waiting is bounded: when the owner is slower than `max_wait`, waiters
/// give up with `WaitTimeout` and do not call the upstream themselves.
#[tokio::test]
async fn waiters_are_bounded_by_max_wait() {
    let Some(conn) = connection().await else { return };
    let mut cfg = config();
    cfg.max_wait = Duration::from_millis(150);
    let caches = replicas(&conn, 2, cfg);
    let calls = Arc::new(AtomicUsize::new(0));

    let owner = {
        let cache = caches[0].clone();
        let calls = calls.clone();
        tokio::spawn(async move { cache.get_or_refresh("pool:1", || load(calls, 400)).await })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    let started = Instant::now();
    let waited = caches[1]
        .get_or_refresh("pool:1", || load(calls.clone(), 0))
        .await;
    assert_eq!(waited, Err(CoalesceError::WaitTimeout));
    assert!(started.elapsed() < Duration::from_millis(300));
    assert_eq!(owner.await.unwrap().unwrap().source, Source::Refreshed);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

/// A slow refresh of one pool does not delay reads of another.
#[tokio::test]
async fn unrelated_keys_are_not_blocked() {
    let Some(conn) = connection().await else { return };
    let caches = replicas(&conn, 2, config());
    let calls = Arc::new(AtomicUsize::new(0));

    let slow = {
        let cache = caches[0].clone();
        let calls = calls.clone();
        tokio::spawn(async move { cache.get_or_refresh("pool:slow", || load(calls, 400)).await })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    let started = Instant::now();
    caches[1]
        .get_or_refresh("pool:fast", || load(calls.clone(), 0))
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_millis(150));
    slow.await.unwrap().unwrap();
}

/// During an upstream outage, replicas keep serving the last value until the
/// stale window ends.
#[tokio::test]
async fn stale_reads_are_bounded_during_upstream_degradation() {
    let Some(conn) = connection().await else { return };
    let mut cfg = config();
    cfg.fresh_ttl = Duration::from_millis(100);
    cfg.stale_window = Duration::from_millis(500);
    let caches = replicas(&conn, 2, cfg);

    caches[0]
        .get_or_refresh("pool:1", || async { Ok::<_, String>("v1".to_string()) })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    let degraded = caches[1]
        .get_or_refresh::<String, _, _, _>("pool:1", || async { Err("db down".to_string()) })
        .await
        .unwrap();
    assert_eq!((degraded.value.as_str(), degraded.source), ("v1", Source::Stale));

    tokio::time::sleep(Duration::from_millis(550)).await;
    let gone = caches[1]
        .get_or_refresh::<String, _, _, _>("pool:1", || async { Err("db down".to_string()) })
        .await;
    assert_eq!(gone, Err(CoalesceError::Upstream("db down".to_string())));
}
