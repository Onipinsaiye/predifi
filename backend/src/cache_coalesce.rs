//! Stampede-safe read-through cache for hot market reads (#60).
//!
//! When a popular pool's cache entry expires, every concurrent request would
//! otherwise miss at once and hit Postgres together. [`CoalescingCache`]
//! makes exactly one caller per key — across all replicas — refresh the
//! value, while everyone else either reads the previous value or waits a
//! bounded time for the refresh.
//!
//! ## Protocol
//!
//! Each entry is stored with a *fresh-until* timestamp and a storage TTL of
//! `fresh_ttl + stale_window`. A read:
//!
//! 1. returns the value if it is **fresh**;
//! 2. otherwise tries to take **refresh ownership** of the key: a per-process
//!    mutex (coalesces callers inside one replica) plus a Redis lock
//!    `SET <key>:lock <token> NX PX lock_ttl` (coalesces replicas). The owner
//!    runs the loader under `refresh_timeout`, writes the new value, and
//!    releases the lock with a compare-and-delete on its token;
//! 3. a caller that is not the owner returns the **stale** value immediately
//!    if one exists — staleness is bounded by `stale_window` because the
//!    entry is evicted from storage after it;
//! 4. on a cold miss (no value at all) it polls for the owner's result for at
//!    most `max_wait`, retrying ownership on every poll, then gives up with
//!    [`CoalesceError::WaitTimeout`].
//!
//! ## Failure handling
//!
//! * **Owner returns an error or times out** — the lock is released at once
//!   so a waiter can take over; the owner itself falls back to the stale
//!   value when there is one (degraded upstream), otherwise reports the error.
//! * **Owner dies** (process killed, task aborted) — its lock expires after
//!   `lock_ttl` and the next poller takes ownership. Keep
//!   `refresh_timeout < lock_ttl` so a live owner never outlives its lock.
//! * **Redis unavailable** — reads miss and ownership falls back to the
//!   per-process mutex, so each replica still refreshes a key only once.
//!
//! Locks are per key; nothing is held across an `.await` that is shared
//! between keys, so a slow refresh never delays unrelated keys.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use redis::aio::ConnectionManager;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// Timing knobs for a [`CoalescingCache`].
#[derive(Debug, Clone, Copy)]
pub struct CoalesceConfig {
    /// How long a refreshed value is served without refreshing.
    pub fresh_ttl: Duration,
    /// How long past `fresh_ttl` a value may still be served while a refresh
    /// is in flight or the upstream is failing.
    pub stale_window: Duration,
    /// Expiry of refresh ownership; bounds recovery after an owner dies.
    pub lock_ttl: Duration,
    /// Upper bound on one loader call.
    pub refresh_timeout: Duration,
    /// Upper bound on how long a caller waits on a cold miss.
    pub max_wait: Duration,
    /// Poll interval while waiting.
    pub poll_interval: Duration,
}

impl Default for CoalesceConfig {
    fn default() -> Self {
        Self {
            fresh_ttl: Duration::from_secs(10),
            stale_window: Duration::from_secs(60),
            lock_ttl: Duration::from_secs(5),
            refresh_timeout: Duration::from_secs(3),
            max_wait: Duration::from_secs(4),
            poll_interval: Duration::from_millis(25),
        }
    }
}

/// Where a returned value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A fresh cached value.
    Fresh,
    /// A value past its fresh TTL, inside the stale window.
    Stale,
    /// Loaded by this caller as refresh owner.
    Refreshed,
}

/// A value and its provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct Lookup<T> {
    pub value: T,
    pub source: Source,
}

/// Why no value could be returned.
#[derive(Debug, PartialEq)]
pub enum CoalesceError<E> {
    /// The loader failed and no stale value was available.
    Upstream(E),
    /// The loader exceeded `refresh_timeout` and no stale value was available.
    RefreshTimeout,
    /// Another caller owned the refresh and did not finish within `max_wait`.
    WaitTimeout,
}

#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    fresh_until_ms: u64,
    value: T,
}

type MemoryEntries = Arc<Mutex<HashMap<String, (String, Instant)>>>;

#[derive(Clone)]
enum Backend {
    Redis(ConnectionManager),
    /// Process-local storage with the same expiry semantics; used when Redis
    /// is not configured and in tests.
    Memory(MemoryEntries),
}

const RELEASE_SCRIPT: &str = r#"
if redis.call('GET', KEYS[1]) == ARGV[1] then
    return redis.call('DEL', KEYS[1])
end
return 0
"#;

impl Backend {
    async fn get(&self, key: &str) -> Result<Option<String>, redis::RedisError> {
        match self {
            Self::Redis(conn) => {
                let mut conn = conn.clone();
                redis::cmd("GET").arg(key).query_async(&mut conn).await
            }
            Self::Memory(entries) => {
                let mut entries = entries.lock().expect("coalesce memory lock");
                match entries.get(key) {
                    Some((_, expires)) if *expires <= Instant::now() => {
                        entries.remove(key);
                        Ok(None)
                    }
                    Some((value, _)) => Ok(Some(value.clone())),
                    None => Ok(None),
                }
            }
        }
    }

    async fn set(&self, key: &str, value: &str, ttl: Duration) -> Result<(), redis::RedisError> {
        match self {
            Self::Redis(conn) => {
                let mut conn = conn.clone();
                redis::cmd("SET")
                    .arg(key)
                    .arg(value)
                    .arg("PX")
                    .arg(ttl_ms(ttl))
                    .query_async(&mut conn)
                    .await
            }
            Self::Memory(entries) => {
                entries
                    .lock()
                    .expect("coalesce memory lock")
                    .insert(key.to_string(), (value.to_string(), Instant::now() + ttl));
                Ok(())
            }
        }
    }

    /// `SET key value NX PX ttl`; `true` when the key was set.
    async fn set_nx(
        &self,
        key: &str,
        value: &str,
        ttl: Duration,
    ) -> Result<bool, redis::RedisError> {
        match self {
            Self::Redis(conn) => {
                let mut conn = conn.clone();
                let reply: Option<String> = redis::cmd("SET")
                    .arg(key)
                    .arg(value)
                    .arg("NX")
                    .arg("PX")
                    .arg(ttl_ms(ttl))
                    .query_async(&mut conn)
                    .await?;
                Ok(reply.is_some())
            }
            Self::Memory(entries) => {
                let mut entries = entries.lock().expect("coalesce memory lock");
                let now = Instant::now();
                if entries.get(key).is_some_and(|(_, expires)| *expires > now) {
                    return Ok(false);
                }
                entries.insert(key.to_string(), (value.to_string(), now + ttl));
                Ok(true)
            }
        }
    }

    /// Delete `key` only if it still holds `value`.
    async fn delete_if(&self, key: &str, value: &str) -> Result<(), redis::RedisError> {
        match self {
            Self::Redis(conn) => {
                let mut conn = conn.clone();
                let _: i64 = redis::Script::new(RELEASE_SCRIPT)
                    .key(key)
                    .arg(value)
                    .invoke_async(&mut conn)
                    .await?;
                Ok(())
            }
            Self::Memory(entries) => {
                let mut entries = entries.lock().expect("coalesce memory lock");
                if entries.get(key).is_some_and(|(v, _)| v == value) {
                    entries.remove(key);
                }
                Ok(())
            }
        }
    }

    async fn delete(&self, key: &str) -> Result<(), redis::RedisError> {
        match self {
            Self::Redis(conn) => {
                let mut conn = conn.clone();
                redis::cmd("DEL").arg(key).query_async(&mut conn).await
            }
            Self::Memory(entries) => {
                entries.lock().expect("coalesce memory lock").remove(key);
                Ok(())
            }
        }
    }
}

fn ttl_ms(ttl: Duration) -> u64 {
    u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX).max(1)
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Refresh ownership of one key. Release with [`CoalescingCache::release`];
/// if dropped without release (owner died), the distributed lock simply
/// expires after `lock_ttl`.
struct Ownership {
    _local: tokio::sync::OwnedMutexGuard<()>,
    /// `None` when Redis was unreachable and only the local lock is held.
    token: Option<String>,
}

/// Stampede-safe read-through cache. Cheap to clone; clones share state.
#[derive(Clone)]
pub struct CoalescingCache {
    backend: Backend,
    namespace: Arc<str>,
    config: CoalesceConfig,
    local_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl CoalescingCache {
    /// Cache shared across replicas through Redis.
    pub fn redis(conn: ConnectionManager, namespace: &str, config: CoalesceConfig) -> Self {
        Self::with_backend(Backend::Redis(conn), namespace, config)
    }

    /// Process-local cache with identical semantics.
    pub fn memory(namespace: &str, config: CoalesceConfig) -> Self {
        Self::with_backend(Backend::Memory(Arc::default()), namespace, config)
    }

    /// Use Redis when `redis` is connected, otherwise a process-local cache.
    pub fn from_redis_cache(
        redis: &crate::redis_cache::RedisCache,
        namespace: &str,
        config: CoalesceConfig,
    ) -> Self {
        match redis.connection_manager() {
            Some(conn) => Self::redis(conn, namespace, config),
            None => Self::memory(namespace, config),
        }
    }

    fn with_backend(backend: Backend, namespace: &str, config: CoalesceConfig) -> Self {
        Self {
            backend,
            namespace: Arc::from(namespace),
            config,
            local_locks: Arc::default(),
        }
    }

    pub fn config(&self) -> CoalesceConfig {
        self.config
    }

    fn value_key(&self, key: &str) -> String {
        format!(
            "{}:coalesce:{}:{key}",
            crate::redis_cache::CACHE_NAMESPACE,
            self.namespace
        )
    }

    fn lock_key(&self, key: &str) -> String {
        format!("{}:lock", self.value_key(key))
    }

    /// Read the entry: `Some((value, is_fresh))`. Backend errors read as a miss.
    async fn read<T: DeserializeOwned>(&self, key: &str) -> Option<(T, bool)> {
        let raw = match self.backend.get(&self.value_key(key)).await {
            Ok(raw) => raw?,
            Err(e) => {
                warn!(key, error = %e, "coalescing cache read failed; treating as miss");
                return None;
            }
        };
        let envelope: Envelope<T> = serde_json::from_str(&raw).ok()?;
        Some((envelope.value, envelope.fresh_until_ms > unix_ms()))
    }

    async fn write<T: Serialize>(&self, key: &str, value: &T) {
        let envelope = Envelope {
            fresh_until_ms: unix_ms() + ttl_ms(self.config.fresh_ttl),
            value,
        };
        let Ok(raw) = serde_json::to_string(&envelope) else {
            return;
        };
        let ttl = self.config.fresh_ttl + self.config.stale_window;
        if let Err(e) = self.backend.set(&self.value_key(key), &raw, ttl).await {
            warn!(key, error = %e, "coalescing cache write failed");
        }
    }

    /// Remove `key` so the next read refreshes it (call after mutations).
    pub async fn invalidate(&self, key: &str) {
        if let Err(e) = self.backend.delete(&self.value_key(key)).await {
            warn!(key, error = %e, "coalescing cache invalidation failed");
        }
    }

    fn local_lock(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.local_locks
            .lock()
            .expect("local lock map")
            .entry(key.to_string())
            .or_default()
            .clone()
    }

    /// Try to become the refresh owner for `key` without waiting.
    async fn try_acquire(&self, key: &str) -> Option<Ownership> {
        let local = self.local_lock(key).try_lock_owned().ok()?;
        let token = uuid::Uuid::new_v4().to_string();
        match self
            .backend
            .set_nx(&self.lock_key(key), &token, self.config.lock_ttl)
            .await
        {
            Ok(true) => Some(Ownership {
                _local: local,
                token: Some(token),
            }),
            Ok(false) => None,
            Err(e) => {
                warn!(key, error = %e, "refresh lock unavailable; coalescing within this process only");
                Some(Ownership {
                    _local: local,
                    token: None,
                })
            }
        }
    }

    async fn release(&self, key: &str, ownership: Ownership) {
        if let Some(token) = &ownership.token {
            if let Err(e) = self.backend.delete_if(&self.lock_key(key), token).await {
                warn!(key, error = %e, "failed to release refresh lock; it will expire");
            }
        }
        drop(ownership);
        let mut locks = self.local_locks.lock().expect("local lock map");
        // Only the map holds the mutex now: nobody is using it.
        if locks.get(key).is_some_and(|m| Arc::strong_count(m) == 1) {
            locks.remove(key);
        }
    }

    /// Return the cached value for `key`, refreshing it with `loader` when
    /// needed, following the protocol in the module docs. `loader` runs at
    /// most once per call, and only when this caller owns the refresh.
    pub async fn get_or_refresh<T, E, F, Fut>(
        &self,
        key: &str,
        loader: F,
    ) -> Result<Lookup<T>, CoalesceError<E>>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let deadline = Instant::now() + self.config.max_wait;
        let mut loader = Some(loader);

        loop {
            let cached = self.read::<T>(key).await;
            let stale = match cached {
                Some((value, true)) => {
                    return Ok(Lookup {
                        value,
                        source: Source::Fresh,
                    })
                }
                Some((value, false)) => Some(value),
                None => None,
            };

            if let Some(ownership) = self.try_acquire(key).await {
                // Another owner may have finished between our read and lock.
                if let Some((value, true)) = self.read::<T>(key).await {
                    self.release(key, ownership).await;
                    return Ok(Lookup {
                        value,
                        source: Source::Fresh,
                    });
                }
                let load = loader.take().expect("loader runs at most once");
                let outcome = tokio::time::timeout(self.config.refresh_timeout, load()).await;
                let result = match outcome {
                    Ok(Ok(value)) => {
                        self.write(key, &value).await;
                        Ok(Lookup {
                            value,
                            source: Source::Refreshed,
                        })
                    }
                    Ok(Err(e)) => Err(CoalesceError::Upstream(e)),
                    Err(_) => Err(CoalesceError::RefreshTimeout),
                };
                self.release(key, ownership).await;
                return match (result, stale) {
                    (Err(_), Some(value)) => {
                        debug!(key, "refresh failed; serving stale value");
                        Ok(Lookup {
                            value,
                            source: Source::Stale,
                        })
                    }
                    (result, _) => result,
                };
            }

            if let Some(value) = stale {
                return Ok(Lookup {
                    value,
                    source: Source::Stale,
                });
            }

            let now = Instant::now();
            if now >= deadline {
                return Err(CoalesceError::WaitTimeout);
            }
            tokio::time::sleep(self.config.poll_interval.min(deadline - now)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn config() -> CoalesceConfig {
        CoalesceConfig {
            fresh_ttl: Duration::from_secs(10),
            stale_window: Duration::from_secs(10),
            lock_ttl: Duration::from_millis(500),
            refresh_timeout: Duration::from_millis(400),
            max_wait: Duration::from_secs(2),
            poll_interval: Duration::from_millis(5),
        }
    }

    async fn slow_load(calls: Arc<AtomicUsize>, value: u32, delay_ms: u64) -> Result<u32, String> {
        calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        Ok(value)
    }

    #[tokio::test]
    async fn simultaneous_misses_refresh_once_per_key() {
        let cache = CoalescingCache::memory("t", config());
        let calls = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..64)
            .map(|i| {
                let cache = cache.clone();
                let calls = calls.clone();
                let key = if i % 2 == 0 { "pool:1" } else { "pool:2" };
                tokio::spawn(
                    async move { cache.get_or_refresh(key, || slow_load(calls, 7, 50)).await },
                )
            })
            .collect();
        let mut refreshed = 0;
        for task in tasks {
            let lookup = task.await.unwrap().expect("value");
            assert_eq!(lookup.value, 7);
            if lookup.source == Source::Refreshed {
                refreshed += 1;
            }
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2, "one refresh per key");
        assert_eq!(refreshed, 2);
    }

    #[tokio::test]
    async fn slow_refresh_does_not_block_unrelated_keys() {
        let cache = CoalescingCache::memory("t", config());
        let calls = Arc::new(AtomicUsize::new(0));
        let slow = {
            let cache = cache.clone();
            let calls = calls.clone();
            tokio::spawn(async move {
                cache
                    .get_or_refresh("slow", || slow_load(calls, 1, 300))
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        let started = Instant::now();
        let fast = cache
            .get_or_refresh("fast", || slow_load(calls.clone(), 2, 0))
            .await
            .unwrap();
        assert_eq!(fast.value, 2);
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(slow.await.unwrap().unwrap().value, 1);
    }

    #[tokio::test]
    async fn waiters_give_up_after_max_wait() {
        let mut cfg = config();
        cfg.max_wait = Duration::from_millis(100);
        cfg.refresh_timeout = Duration::from_millis(450);
        let cache = CoalescingCache::memory("t", cfg);
        let calls = Arc::new(AtomicUsize::new(0));
        let owner = {
            let cache = cache.clone();
            let calls = calls.clone();
            tokio::spawn(
                async move { cache.get_or_refresh("k", || slow_load(calls, 1, 400)).await },
            )
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        let started = Instant::now();
        let waited = cache
            .get_or_refresh("k", || slow_load(calls.clone(), 2, 0))
            .await;
        assert_eq!(waited, Err(CoalesceError::WaitTimeout));
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(100) && elapsed < Duration::from_millis(250),
            "{elapsed:?}"
        );
        assert_eq!(owner.await.unwrap().unwrap().source, Source::Refreshed);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn waiter_takes_over_after_owner_dies() {
        let cache = CoalescingCache::memory("t", config());
        let calls = Arc::new(AtomicUsize::new(0));
        let owner = {
            let cache = cache.clone();
            let calls = calls.clone();
            tokio::spawn(async move {
                cache
                    .get_or_refresh("k", || slow_load(calls, 1, 10_000))
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        owner.abort(); // owner killed while holding the refresh lock
        let started = Instant::now();
        let lookup = cache
            .get_or_refresh("k", || slow_load(calls.clone(), 2, 0))
            .await
            .expect("recovered after lock expiry");
        assert_eq!((lookup.value, lookup.source), (2, Source::Refreshed));
        // Recovery is bounded by the lock TTL.
        assert!(started.elapsed() < config().lock_ttl + Duration::from_millis(200));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn failed_owner_releases_lock_immediately() {
        let cache = CoalescingCache::memory("t", config());
        let owner = {
            let cache = cache.clone();
            tokio::spawn(async move {
                cache
                    .get_or_refresh::<u32, _, _, _>("k", || async {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        Err("db down".to_string())
                    })
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        let started = Instant::now();
        let lookup = cache
            .get_or_refresh("k", || async { Ok::<_, String>(5u32) })
            .await
            .unwrap();
        assert_eq!((lookup.value, lookup.source), (5, Source::Refreshed));
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "no wait for lock expiry"
        );
        assert_eq!(
            owner.await.unwrap(),
            Err(CoalesceError::Upstream("db down".into()))
        );
    }

    #[tokio::test]
    async fn stale_value_is_served_during_refresh_and_degradation() {
        let mut cfg = config();
        cfg.fresh_ttl = Duration::from_millis(50);
        cfg.stale_window = Duration::from_millis(300);
        let cache = CoalescingCache::memory("t", cfg);
        cache
            .get_or_refresh("k", || async { Ok::<_, String>(1u32) })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;

        // A slow refresh is in flight; other callers get the stale value at once.
        let refresher = {
            let cache = cache.clone();
            tokio::spawn(async move {
                cache
                    .get_or_refresh("k", || async {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        Err::<u32, _>("upstream degraded".to_string())
                    })
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        let started = Instant::now();
        let during = cache
            .get_or_refresh("k", || async { Ok::<_, String>(99u32) })
            .await
            .unwrap();
        assert_eq!((during.value, during.source), (1, Source::Stale));
        assert!(started.elapsed() < Duration::from_millis(50));
        // The failing owner falls back to the stale value too.
        let owner = refresher.await.unwrap().unwrap();
        assert_eq!((owner.value, owner.source), (1, Source::Stale));

        // Past the stale window the value is gone and errors surface.
        tokio::time::sleep(Duration::from_millis(350)).await;
        let expired = cache
            .get_or_refresh::<u32, _, _, _>("k", || async { Err("still down".to_string()) })
            .await;
        assert_eq!(expired, Err(CoalesceError::Upstream("still down".into())));
    }

    #[tokio::test]
    async fn hung_loader_times_out_and_releases() {
        let mut cfg = config();
        cfg.refresh_timeout = Duration::from_millis(50);
        let cache = CoalescingCache::memory("t", cfg);
        let timed_out = cache
            .get_or_refresh::<u32, String, _, _>("k", std::future::pending)
            .await;
        assert_eq!(timed_out, Err(CoalesceError::RefreshTimeout));
        let next = cache
            .get_or_refresh("k", || async { Ok::<_, String>(3u32) })
            .await
            .unwrap();
        assert_eq!(next.source, Source::Refreshed);
    }

    #[tokio::test]
    async fn invalidate_forces_refresh() {
        let cache = CoalescingCache::memory("t", config());
        cache
            .get_or_refresh("k", || async { Ok::<_, String>(1u32) })
            .await
            .unwrap();
        cache.invalidate("k").await;
        let lookup = cache
            .get_or_refresh("k", || async { Ok::<_, String>(2u32) })
            .await
            .unwrap();
        assert_eq!((lookup.value, lookup.source), (2, Source::Refreshed));
    }
}
