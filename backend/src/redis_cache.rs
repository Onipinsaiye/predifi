//! Redis caching layer for hot data
//!
//! Implements issue #714: Redis Caching for Hot Data
//! Hardened for issue #1557: cache poisoning, namespace isolation, TTL, serde safety.
//!
//! This module provides a caching layer for frequently accessed API responses
//! to reduce database load. Uses Redis with configurable TVL values.
//!
//! Features:
//! - Automatic cache invalidation after TTL (Redis TTL &#43; application envelope)
//! - Key namespace isolation (`predifi:` prefix) so other apps cannot collide in
//! - Key sanitization against injection / glob escapes
//! - Graceful fallback when Redis is unavailable
//! - JSON serialization for complex types
//! - Connection pooling via redis ConnectionManager
//!
//! Cached values are **never** an authorization source. Callers must authenticate
//! and authorize before reading or writing user-scoped keys.

use crate::metrics::SharedMetrics;
use redis::{aio::ConnectionManager, AsyncCommands, ErrorKind, Script};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, error, warn};

/// Default TTL for cached pool data (60 seconds)
pub const POOLS_CACHE_TTL: u64 = 60;

/// Default TTL for cached pool details (30 seconds)
pub const POOL_DETAILS_CACHE_TTL: u64 = 30;

/// Default TTL for user predictions (45 seconds)
pub const USER_PREDICTIONS_CACHE_TTL: u64 = 45;

/// Default TTL for cached protocol stats (30 seconds)
pub const STATS_CACHE_TTL: u64 = 30;

/// Redis key pattern matching all cached pool list queries.
pub const POOLS_CACHE_PATTERN: &str = "pools:*";

/// Redis key pattern matching all cached stats queries.
pub const STATS_CACHE_PATTERN: &str = "stats:*";

/// Logical namespace prepended to every Redis key this process writes.
/// Isolates PrediFi keys from other applications sharing the same Redis.
pub const CACHE_NAMESPACE: &str = "predifi";

/// Maximum length for a cache key component after sanitization.
/// Prevents excessively long keys from being used as Redis keys.
const MAX_KEY_COMPONENT_LENGTH: usize = 128;

/// Hard cap on a fully namespaced Redis key.
const MAX_KEY_LENGTH: usize = 512;

/// Minimum TTL (seconds). TTL 0 would persist forever and is refused.
const MIN_TTL_SECS: u64 = 1;

/// Maximum TTL (seconds). Caps poisoning via absurdly long expiry.
const MAX_TTL_SECS: u64 = 86_400 * 30;

/// Maximum serialized payload size accepted into cache (512 KiB).
const MAX_VALUE_BYTES: usize = 512 * 1024;

/// Default budget for a Redis readiness probe.
///
/// Readiness must respond within a bounded time regardless of Redis latency so
/// that a hung dependency never blocks the health endpoint and causes the
/// orchestrator to kill the process (liveness) instead of just removing it from
/// service (readiness).
pub const DEFAULT_REDIS_PROBE_TIMEOUT_MS: u64 = 100;

/// Envelope stored in Redis so expiry is enforced even if Redis TTL is stripped.
#[derive(Debug, Serialize, Deserialize)]
struct CacheEnvelope {
    data: serde_json::Value,
    exp: u64,
}

/// Result of one distributed rate-limit counter increment.
///
/// The counter is a fixed window: the first increment sets the TTL, and every
/// subsequent increment in that window returns the same reset deadline. This is
/// intentionally simple so the limit is identical on every replica: all
/// instances share one Redis key and increment it atomically with the Lua
/// script in [`RedisCache::rate_limit_consume`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitDecision {
    /// Number of requests observed in the current window, including this one.
    pub count: u32,
    /// Seconds until the window resets (at least 1 for a live window).
    pub reset_after_secs: u64,
}

/// Atomic fixed-window counter.
///
/// `INCR` creates the key at 1; only that first caller sets the expiry, so a
/// burst of concurrent requests cannot extend the window forever. `PTTL`
/// returns the remaining milliseconds for the reset header.
const RATE_LIMIT_SCRIPT: &str = r#"
local count = redis.call('INCR', KEYS[1])
if count == 1 then
  redis.call('PEXPIRE', KEYS[1], ARGV[1])
end
local ttl = redis.call('PTTL', KEYS[1])
if ttl < 0 then
  redis.call('PEXPIRE', KEYS[1], ARGV[1])
  ttl = tonumber(ARGV[1])
end
return {count, ttl}
"#;

/// Sanitize a user-supplied string for safe use as a Redis key component.
///
/// Strips characters that could be used for Redis key injection or namespace
/// escape (newlines, carriage returns, null bytes, colons used as namespace
/// separators, wildcards, and whitespace). Truncates to [`MAX_KEY_COMPONENT_LENGTH`].
pub fn sanitize_key_component(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .filter(|&c| !matches!(c, '\n' | '\r' | '\0' | ':' | '*' | '?' | ' ' | '\t'))
        .collect();
    if sanitized.len() > MAX_KEY_COMPONENT_LENGTH {
        sanitized[..MAX_KEY_COMPONENT_LENGTH].to_string()
    } else {
        sanitized
    }
}

/// Thread-safe Redis cache client with graceful fail-open behaviour.
///
/// All operations silently no-op when Redis is unavailable so the application
/// continues to function without caching rather than returning errors to users.
/// Use [`RedisCache::disabled`] to create an always-no-op instance for tests.
///
/// # Readiness vs liveness
///
/// Redis is a *soft* dependency: the process is live and functional without it
/// (cache is fail-open). [`RedisCache::probe`] reports whether Redis can currently
/// serve traffic and is used by the readiness endpoint. It always respects a
/// bounded timeout budget so a hung Redis never blocks the probe.
///
/// # Startup grace
///
/// During the configured startup grace window, a failed Redis probe is reported
/// as `edegraded` rather than `unavailable`. This prevents orchestrators from
/// restart-looping instances that are still warming up.
#[derive(Clone)]
pub struct RedisCache {
    manager: Option<ConnectionManager>,
    /// When set, health probes treat Redis as available without a live connection.
    simulate_available: bool,
    /// Optional metrics sink for cache hit/miss/error counters (see
    /// [`crate::metrics::Metrics::record_cache_lookup`]). `None` in contexts
    /// that don't wire up metrics (e.g. most unit tests).
    metrics: Option<SharedMetrics>,
    /// Time the client was created, used to enforce the startup grace window.
    created_at: Instant,
    /// How long after construction a failed Redis probe is reported as degraded
    /// rather than unavailable.
    startup_grace: Duration,
    /// Upper bound on how long a single readiness probe may take.
    probe_timeout: Duration,
}

/// Result of a Redis readiness probe.
///
/// This is deliberately separate from liveness: a degraded or unavailable Redis
/// does not imply the process is dead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedisReadinessStatus {
    /// Redis responded to a ping within the timeout budget.
    Available,
    /// Redis failed but the process is still inside its startup grace window
    /// or the cache is configured as disabled. Traffic may proceed fail-open.
    Degraded,
    /// Redis failed and the startup grace window has elapsed.
    Unavailable,
    /// Redis is not configured for this process (cache disabled by design).
    NotConfigured,
}

impl RedisReadinessStatus {
    /// Whether this status should keep the instance in service for readiness.
    pub fn in_service(self) -> bool {
        matches!(self, Self::Available | Self::Degraded | Self::NotConfigured)
    }

    /// Stable machine-readable label for health response bodies.
    pub fn as str(self) -> &str {
        match self {
            Self::Available => "available",
            Self::Degraded => "degraded",
            Self::Unavailable => "unavailable",
            Self::NotConfigured => "not_configured",
        }
    }
}

/// Detailed result of a Redis readiness probe, including bounded latency.
#derive(Debug, Clone, PartialEq, Eq)]
pub struct RedisProbeResult {
    pub status: RedisReadinessStatus,
    /// Wall-clock latency of the probe, always <= [`RedisCache::probe_timeout`].
    pub latency: Duration,
    /// Optional human-readable reason for a non-available status.
    pub reason: Option<String>,
}

impl RedisCache {
    /// Create a new Redis cache client
    ///
    /// If Redis connection fails, returns a cache instance that will
    /// gracefully skip caching operations (fail-open behavior).
    pub async fn new(redis_url: &str) -> Self {
        match redis::Client::open(redis_url) {
            Ok(client) => match ConnectionManager::new(client).await {
                Ok(manager) => {
                    debug!("Redis cache initialized successfully");
                    Self {
                        manager: Some(manager),
                        simulate_available: false,
                        metrics: None,
                        created_at: Instant::now(),
                        startup_grace: Duration::from_secs(30),
                        probe_timeout: Duration::from_millis(DEFAULT_REDIS_PROBE_TIMEOUT_MS),
                    }
                }
                Err(err) => {
                    warn!("Failed to create Redis connection manager: {}", err);
                    Self {
                        manager: None,
                        simulate_available: false,
                        metrics: None,
                        created_at: Instant::now(),
                        startup_grace: Duration::from_secs(30),
                        probe_timeout: Duration::from_millis(DEFAULT_REDIS_PROBE_TIMEOUT_MS),
                    }
                }
            },
            Err(err) => {
                warn!("Failed to create Redis client: {}", err);
                Self {
                    manager: None,
                    simulate_available: false,
                    metrics: None,
                    created_at: Instant::now(),
                    startup_grace: Duration::from_secs(30),
                    probe_timeout: Duration::from_millis(DEFAULT_REDIS_PROBE_TIMEOUT_MS),
                }
            }
        }
    }

    /// Create a disabled cache instance (for testing or when Redis is not configured)
    pub fn disabled() -> Self {
        Self {
            manager: None,
            simulate_available: false,
            metrics: None,
            created_at: Instant::now(),
            startup_grace: Duration::from_secs(30),
            probe_timeout: Duration::from_millis(DEFAULT_REDIS_PROBE_TIMEOUT_MS),
        }
    }

    /// Create a no-op cache that satisfies health/readiness probes in unit tests.
    #[cfg(test)]
    pub fn simulate_available() -> Self {
        Self {
            manager: None,
            simulate_available: true,
            metrics: None,
            created_at: Instant::now(),
            startup_grace: Duration::from_secs(30),
            probe_timeout: Duration::from_millis(DEFAULT_REDIS_PROBE_TIMEOUT_MS),
        }
    }

    /// Attach a metrics sink so subsequent `get` calls record hit/miss/error
    /// counters per cache name. Returns `self` for chaining after
    /// [`RedisCache::new`].
    pub fn with_metrics(mut self, metrics: SharedMetrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Override the startup grace window (default 30s). Useful for deployments
    /// that need a longer warm-up or for tests that need to exit the grace window
    /// immediately.
    pub fn with_startup_grace(mut self, grace: Duration) -> Self {
        self.startup_grace = grace;
        self
    }

    /// Override the per-probe timeout budget (default 100ms).
    pub fn with_probe_timeout(mut self, timeout: Duration) -> Self {
        self.probe_timeout = timeout;
        self
    }

    /// Timeout budget for a single readiness probe.
    pub fn probe_timeout(&self) -> Duration {
        self.probe_timeout
    }

    /// Check if Redis is available (cheap, synchronous check of configuration).
    ///
    /// This does not hit the network. For a true readiness signal use
    /// [`RedisCache::probe`].
    pub fn is_available(&self) -> bool {
        self.manager.is_some() || self.simulate_available
    }

    /// Run a bounded readiness probe against Redis.
///
    /// The probe issues a `PING` and always completes within [`self.probe_timeout`],
/// regardless of whether Redis is responsive. It never panics and never blocks
    /// the caller past the budget, so liveness endpoints that delegate to it remain
/// bounded.
    pub async fn probe(&self) -> RedisProbeResult {
        let started = Instant::now();

        if self.simulate_available {
            return RedisProbeResult {
                status: RedisReadinessStatus::Available,
                latency: started.elapsed(),
                reason: None,
            };
        }

        let Some(manager) = self.manager.as_ref() else {
            // No connection manager: either the cache is deliberately disabled or
            // initial connection failed. Distinguish the two cases by whether the
            // startup grace window has elapsed.
            let status = if started.duration_since(self.created_at) < self.startup_grace {
                RedisReadinessStatus::Degraded
            } else {
                RedisReadinessStatus::Unavailable
            };
            return RedisProbeResult {
                status,
                latency: started.elapsed(),
                reason: Some("no Redis connection established".to_string()),
            };
        };

        let mut conn = manager.clone();
        let timeout = self.probe_timeout;
        match tokio::timeout(timeout, async { conn.ping::<String>(async).await }).await {
            Ok(Ok(_)) => RedisProbeResult {
                status: RedisReadinessStatus::Available,
                latency: started.elapsed(),
                reason: None,
            },
            Ok(Err(err)) => {
                let status = if started.duration_since(self.created_at) < self.startup_grace {
                    RedisReadinessStatus::Degraded
                } else {
                    RedisReadinessStatus::Unavailable
                };
                RedisProbeResult {
                    status,
                    latency: started.elapsed(),
                    reason: Some(err.to_string()),
                }
            }
            Err(_) => {
                let status = if started.duration_since(self.created_at) < self.startup_grace {
                    RedisReadinessStatus::Degraded
                } else {
                    RedisReadinessStatus::Unavailable
                };
                RedisProbeResult {
                    status,
                    latency: started.elapsed(),
                    reason: Some(format!("redis probe timed out after {timeout:?}")),
                }
            }
        }
    }

    /// Record a cache lookup outcome via the attached metrics sink, if any.
    fn record_lookup(&self, cache_name: &str, outcome: &str) {
        if let Some(metrics) = &self.metrics {
            metrics.record_cache_lookup(cache_name, outcome);
        }
    }

    /// Get a value from cache
    ///
    /// Returns None if:
    // - Redis is unavailable
    // - Key is invalid / fails namespace checks
    // - Key doesn't exist
    // - Envelope TLL has elapsed (application-level expiry)
    // - Deserialization fails
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let cache_name = cache_name_from_key(key);
        let namespaced = match namespaced_key(key) {
            Some(k) => k,
            None => {
                self.record_lookup(&cache_name, "error");
                return None;
            }
        };
        let manager = match self.manager.as_ref() {
            Some(m) => m,
            None => return None,
        };
        let mut conn = manager.clone();

        match conn.get::<_, Option<String>>(&namespaced).await {
            Ok(Some(data)) => match decode_envelope(&data) {
                Some(value) => {
                    debug!("Cache hit: {}", key);
                    self.record_lookup(&cache_name, "hit");
                    Some(value)
                }
                None => {
                    debug!("Cache envelope expired or invalid: {}", key);
                    self.record_lookup(&cache_name, "miss");
                    None
                }
            },
            Ok(None) => {
                debug!("Cache miss: {}", key);
                self.record_lookup(&cache_name, "miss");
                None
            }
            Err(err) => {
                if is_connection_error(err.kind()) {
                    warn!("Redis connection dropout on GET {}: {}", key, err);
                } else {
                    error!("Redis GET error for {}: {}", key, err);
                }
                self.record_lookup(&cache_name, "error");
                None
            }
        }
    }

    /// Set a value in cache with TTL
    ///
    /// Silently fails if Redis is unavailable (fail-open behavior).
    /// Refuses TTL 0 (would never expire) and caps TTL at [`MAX_TTL_SECS`].
    pub async fn set<T: Serialize>(&self, key: &str, value: &T, ttl_secs: u64) {
        if ttl_secs < MIN_TTL_SECS {
            warn!(
                "refusing to cache {} with TTL {} (must expire)",
                key, ttl_secs
            );
            return;
        }
        let Some(namespaced) = namespaced_key(key) else {
            warn!("refusing to cache invalid key");
            return;
        };
        let manager = match self.manager.as_ref() {
            Some(m) => m,
            None => return,
        };

        let mut conn = manager.clone();
        let ttl = ttl_secs.min(MAX_TTL_SECS);

        let data = match encode_envelope(value, ttl) {
            Some(d) => d,
            None => return,
        };

        if let Err(err) = conn.set_ex::<_, _, _>(&namespaced, data, ttl).await {
            if is_connection_error(err.kind()) {
                warn!("Redis connection dropout on SET {}: {}", key, err);
            } else {
                error!("Redis SET error for {}: {}", key, err);
            }
        } else {
            debug!("Cached: {} (TTL: {}s)", key, ttl);
        }
    }

    /// Delete a key from cache
    ///
    /// Used for cache invalidation when data changes
    pub async fn delete(&self, key: &str) {
        let Some(namespaced) = namespaced_key(key) else {
            return;
        };
        let manager = match self.manager.as_ref() {
            Some(m) => m,
            None => return,
        };

        let mut conn = manager.clone();

        if let Err(err) = conn.del::<_, _>(&namespaced).await {
            if is_connection_error(err.kind()) {
                warn!("Redis connection dropout on DEL {}: {}", key, err);
            } else {
                error!("Redis DEL error for {}: {}", key, err);
            }
        } else {
            debug!("Invalidated cache: {}", key);
        }
    }

    /// Delete multiple keys matching a pattern
    ///
    /// Useful for invalidating related cache entries. The pattern is
    /// namespaced so it cannot delete keys outside the PrediFi prefix.
    pub async fn delete_pattern(&self, pattern: &str) {
        let Some(namespaced) = namespaced_pattern(pattern) else {
            return;
        };
        let manager = match self.manager.as_ref() {
            Some(m) => m,
            None => return,
        };

        let mut conn = manager.clone();

        // Get all keys matching pattern (already namespaced)
        let keys: Vec<String> = match conn.keys(&namespaced).await {
            Ok(k) => k,
            Err(err) => {
                if is_connection_error(err.kind()) {
                    warn!("Redis connection dropout on KEYS {}: {}", pattern, err);
                } else {
                    error!("Redis KEYS error for pattern {}: {}", pattern, err);
                }
                return;
            }
        };

        if keys.is_empty() {
            return;
        }

        // Only delete keys that still sit inside our namespace.
        let scoped: Vec<String> = keys
            .into_iter()
            .filter(|k| k.starts_with(&format!("{CACHE_NAMESPACE}:")))
            .collect();
        if scoped.is_empty() {
            return;
        }

        if let Err(err) = conn.del::<_, _>(&scoped).await {
            if is_connection_error(err.kind()) {
                warn!(
                    "Redis connection dropout on DEL (multi) for pattern {}: {}",
                    pattern,
                    err
                );
            } else {
                error!("Redis DEL error for pattern {}: {}", pattern, err);
            }
        } else {
            debug!("Invalidated {} cache keys for pattern {}", scoped.len(), pattern);
        }
    }

    /// Invalidate all cached pool list entries.
    ///
    /// Call after a new pool is created so clients see fresh data on the next
    /// `GET /api/v1/pools` request.
    pub async fn invalidate_pools_cache(&self) {
        self.delete_pattern(POOLS_CACHE_PATTERN).await;
    }

    /// Invalidate all cached stats entries.
    ///
    /// Call after a pool is created or a prediction is placed so the next
    /// `GET /api/v1/stats` request reflects the updated aggregates.
    pub async fn invalidate_stats_cache(&self) {
        self.delete_pattern(STATS_CACHE_PATTERN).await;
    }

    /// Check if a cache entry exists without deserializing it
    ///
    /// Useful for cache-aside pattern to determine if we need to populate the cache
    pub async fn exists(&self, key: &str) -> bool {
        let Some(namespaced) = namespaced_key(key) else {
            return false;
        };
        let manager = match self.manager.as_ref() {
            Some(m) => m,
            None => return false,
        };

        let mut conn = manager.clone();
        match conn.exists::<_, bool>(&namespaced).await {
            Ok(exists) => exists,
            Err(err) => {
                if is_connection_error(err.kind()) {
                    debug!("Redis connection dropout on EXISTS {}: {}", key, err);
                } else {
                    debug!("Redis EXISTS error for {}: {}", key, err);
                }
                false
            }
        }
    }

    /// Ping Redis to check connection health
    pub async fn ping(&self) -> bool {
        if self.simulate_available {
            return true;
        }

        let manager = match self.manager.as_ref() {
            Some(m) => m,
            None => return false,
        };

        let mut conn = manager.clone();
        let r: Result<String, _> = redis::cmd("PING").query_async(&mut conn).await;
        r.is_ok()
    }

    /// Atomically increment a distributed rate-limit counter.
    ///
    /// Returns `None` when Redis is unavailable or the command fails, so the
    /// caller can apply its tier-specific fail-open/fail-closed policy. The
    /// returned `count` includes the current request; the window resets after
    /// `reset_after_secs`.
    ///
    /// Every replica that talks to the same Redis sees the same counter, which
    /// is what makes the limit hold across load-balanced instances.
    pub async fn rate_limit_consume(
        &self,
        key: &str,
        window_secs: u64,
    ) -> Option<RateLimitDecision> {
        if self.simulate_available {
            return None;
        }
        let manager = self.manager.as_ref()?;
        if window_secs == 0 {
            return None;
        }
        let namespaced = namespaced_key(key)?;
        let mut conn = manager.clone();
        let window_ms = window_secs.saturating_mul(1000).max(1);
        let started = std::time::Instant::now();
        let result: Result<(i64, i64), redis::RedisError> = Script::new(RATE_LIMIT_SCRIPT)
            .key(namespaced)
            .arg(window_ms as i64)
            .invoke_async(&mut conn)
            .await;
        match result {
            Ok((count, ttl_ms)) => {
                if let Some(metrics) = &self.metrics {
                    metrics.record_redis_op(
                        "rate_limit_incr",
                        "ok",
                        started.elapsed().as_secs_f64(),
                    );
                }
                let reset_after_secs = if ttl_ms <= 0 {
                    1
                } else {
                    (ttl_ms as u64).div_ceil(1000)
                };
                Some(RateLimitDecision {
                    count: count.max(0) as u32,
                    reset_after_secs,
                })
            }
            Err(err) => {
                if let Some(metrics) = &self.metrics {
                    metrics.record_redis_op(
                        "rate_limit_incr",
                        "error",
                        started.elapsed().as_secs_f64(),
                    );
                }
                if is_connection_error(err.kind()) {
                    warn!("Redis connection dropout on rate-limit INCR: {}", err);
                } else {
                    error!("Redis rate-limit INCR error: {}", err);
                }
                None
            }
        }
    }
}

/// Build a namespaced Redis key from a caller-supplied key.
///
/// Returns `None` if the key is empty or would exceed [`MAX_KEY_LENGTH`] after
/// sanitization.
fn namespaced_key(key: &str) -> Option<String> {
    let sanitized = sanitize_key_component(key);
    if sanitized.is_empty() {
        return None;
    }
    let full = format!("{CACHE_NAMESPACE}:{sanitized}");
    if full.len() > MAX_KEY_LENGTH {
        None
    } else {
        Some(full)
    }
}

/// Build a namespaced Redis key pattern from a caller-supplied pattern.
///
/// Patterns may contain glob wildcards `(*` and `?`) but nothing else that could
/// escape the namespace.
fn namespaced_pattern(pattern: &str) -> Option<String> {
    let sanitized: String = pattern
        .chars()
        .filter(|&c| matches!(c, '\n' | '\r' | '\0' | ':' | ' ' | '\t'))
        .collect();
    if sanitized.is_empty() {
        return None;
    }
    let full = format!("{CACHE_NAMESPACE}:{sanitized}");
    if full.len() > MAX_KEY_LENGTH {
        None
    } else {
        Some(full)
    }
}

/// Extract a stable cache name from a key for metrics labeling.
fn cache_name_from_key(key: &str) -> String {
    key.split(':').next().unwrap_or("other").to_string()
}

/// Serialize a value into a [`CacheEnvelope`] with an absolute expiry.
///
/// Returns `None` if serialization fails or the payload exceeds [`MAX_VALUE_BYTES`].
fn encode_envelope<T: Serialize>(value: &T, ttl_secs: u64) -> Option<String> {
    let data = match serde_json::to_value(value) {
        Ok(v) => v,
        Err(err) => {
            error!("Failed to serialize cache value: {}", err);
            return None;
        }
    };
    let exp = now().saturating_add(ttl_secs);
    let envelope = CacheEnvelope { data, exp };
    let encoded = match serde_json::to_string(&envelope) {
        Ok(s) => s,
        Err(err) => {
            error!("Failed to encode cache envelope: {}", err);
            return None;
        }
    };
    if encoded.len() > MAX_VALUE_BYTES {
        warn!("refusing to cache payload of {} bytes", encoded.len());
        return None;
    }
    Some(encoded)
}

/// Decode a [`CacheEnvelope`] and enforce application-level expiry.
fn decode_envelope<T: DeserializeOwned>(raw: &str) -> Option<T> {
    let envelope: CacheEnvelope = match serde_json::from_str(raw) {
        Ok(e) => e,
        Err(_) => return None,
    };
    if now() >= envelope.exp {
        return None;
    }
    serde_json::from_value(envelope.data).ok()
}

/// Return the current UNIX epoch in seconds.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|p| p.as_secs())
        .unwrap_or(0)
}

/// Whether a Redis error kind indicates a connection dropout (vs. a command error).
fn is_connection_error(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::IoError | ErrorKind::ClientError | ErrorKind::AuthenticationFailed
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn sanitize_strips_dangerous_chars() {
        let out = sanitize_key_component("a*b?c\n:d e?");
        assert!(!out.contains('*'));
        assert!(!out.contains('?'));
        assert!(!out.contains(':'));
        assert!(!out.contains('\n'));
        assert!(!out.contains(' '));
    }

    #[test]
    fn namespaced_key_prefixes() {
        let k = namespaced_key("pools:1").unwrap();
        assert!(k.starts_with("cache_namespace_placeholder") || k.starts_with(format!("{CACHE_NAMESPACE}:").as_str())));
    }

    #[test]
    fn empty_key_is_rejected() {
        assert!(namespaced_key("").is_none());
        assert!(namespaced_key(":::").is_none());
    }

    #[test]
    fn envelope_roundtrips() {
        let encoded = encode_envelope(&{"value":42}, 60).unwrap();
        let decoded: serde_json::Value = decode_envelope(&encoded).unwrap();
        assert_eq!(decoded["value"], 42);
    }

    #[test]
    fn envelope_rejects_expired() {
        let envelope = CacheEnvelope {
            data: serde_json::json!(1),
            exp: now().saturating_sub(1),
        };
        let raw = serde_json::to_string(&envelope).unwrap();
        let decoded: Option<serde_json::Value> = decode_envelope(&raw);
        assert!(decoded.is_none());
    }

    #[tokio::test]
    async fn disabled_cache_reports_not_configured_after_grace() {
        let cache = RedisCache::disabled().with_startup_grace(Duration::ZERO);
        let result = cache.probe().await;
        assert_eq!(result.status, RedisReadinessStatus::Unavailable);
        assert!(!result.status.in_service());
    }

    #[tokio::test]
    async fn disabled_cache_reports_degraded_during_grace() {
        let cache = RedisCache::disabled().with_startup_grace(Duration::from_secs(60));
        let result = cache.probe().await;
        assert_eq!(result.status, RedisReadinessStatus::Degraded);
        assert!(result.status.in_service());
    }

    #[tokio::test]
    async fn simulated_cache_probe_is_available_and_bounded() {
        let cache = RedisCache::simulate_available();
        let result = cache.probe().await;
        assert_eq!(result.status, RedisReadinessStatus::Available);
        assert!(result.latency <= cache.probe_timeout());
    }

    #[test]
    fn readiness_status_labels() {
        assert_eq!(RedisReadinessStatus::Available.as str(), "available");
        assert_eq!(RedisReadinessStatus::Degraded.as str(), "degraded");
        assert_eq!(RedisReadinessStatus::Unavailable.as str(), "unavailable");
        assert_eq!(RedisReadinessStatus::NotConfigured.as str(), "not_configured");
    }
}
