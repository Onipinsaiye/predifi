//! Bounded retries and circuit breaking for Soroban RPC (#58).
//!
//! Every Soroban JSON-RPC call should go through [`ResilientRpcClient`] so a
//! slow or failing RPC node cannot pile up workers or stretch API latency.
//!
//! ## Error classification
//!
//! Each failure is an [`RpcError`] and maps to an [`ErrorClass`]:
//!
//! | Failure                                         | Class                    |
//! |-------------------------------------------------|--------------------------|
//! | connection refused / DNS / TLS (never sent)     | `Retryable`              |
//! | HTTP 429, HTTP 503, JSON-RPC rate-limit codes   | `Retryable` (throttled)  |
//! | per-attempt timeout, HTTP 408/500/502/504,      | `RetryableIfIdempotent`  |
//! | JSON-RPC `-32603` internal error                |                          |
//! | other HTTP 4xx, other JSON-RPC errors,          | `Permanent`              |
//! | malformed responses                             |                          |
//!
//! `RetryableIfIdempotent` failures may have reached the node, so they are
//! retried only for idempotent calls. Transaction submission
//! (`sendTransaction`) is [`Idempotency::NonIdempotent`]: it is retried only
//! when the request provably did not take effect (connection failures,
//! throttling). After an ambiguous failure the caller must look the
//! transaction up by hash (`getTransaction`) instead of resubmitting blindly.
//!
//! ## Backoff and deadlines
//!
//! Attempt `n` (0-based) waits `min(max_delay, base_delay * 2^n)` with equal
//! jitter (a random value in `[d/2, d]`). A `Retry-After` from the node is
//! honoured when it is longer. Every call has an overall deadline
//! (`max_elapsed`): no attempt starts, and no sleep is taken, that would end
//! past it. Each attempt's timeout is clipped to the time remaining.
//!
//! ## Circuit breaker
//!
//! One [`CircuitBreaker`] is shared per RPC URL ([`shared_breaker`]).
//!
//! * **Closed** — calls flow; `failure_threshold` consecutive upstream
//!   failures (timeouts, connection errors, 5xx, throttling) open it.
//!   Permanent client errors do not count: they say nothing about node health.
//! * **Open** — calls fail immediately with [`RpcError::CircuitOpen`] (no
//!   network I/O, no waiting) until `open_duration` has passed.
//! * **Half-open** — up to `half_open_max_probes` calls are let through.
//!   `success_threshold` successes close the circuit; any failure opens it
//!   again for another `open_duration`.
//!
//! Time comes from a [`Clock`], so tests can drive the whole state machine
//! with [`FakeClock`] and a scripted [`RpcTransport`] deterministically.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;

/// Boxed future used by the object-safe [`Clock`] and [`RpcTransport`].
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ── Errors ───────────────────────────────────────────────────────────────────

/// A failed Soroban RPC call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcError {
    /// The attempt did not complete within its timeout.
    Timeout,
    /// The request could not be delivered (connect, DNS, TLS).
    Connect(String),
    /// The node rejected the request because of load (HTTP 429/503 or a
    /// JSON-RPC rate-limit code).
    Throttled { retry_after: Option<Duration> },
    /// HTTP status other than 2xx, 429 or 503.
    Http(u16),
    /// The node answered with a JSON-RPC error object.
    JsonRpc { code: i64, message: String },
    /// The response could not be parsed.
    InvalidResponse(String),
    /// The circuit breaker is open; no request was sent.
    CircuitOpen,
    /// The overall call deadline was reached.
    DeadlineExceeded,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "rpc attempt timed out"),
            Self::Connect(e) => write!(f, "rpc connection failed: {e}"),
            Self::Throttled { .. } => write!(f, "rpc node throttled the request"),
            Self::Http(status) => write!(f, "rpc returned HTTP {status}"),
            Self::JsonRpc { code, message } => write!(f, "rpc error {code}: {message}"),
            Self::InvalidResponse(e) => write!(f, "invalid rpc response: {e}"),
            Self::CircuitOpen => write!(f, "rpc circuit breaker is open"),
            Self::DeadlineExceeded => write!(f, "rpc call deadline exceeded"),
        }
    }
}

impl std::error::Error for RpcError {}

/// How a failure may be retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Safe to retry for any call: the request did not take effect.
    Retryable,
    /// The request may have taken effect; retry only idempotent calls.
    RetryableIfIdempotent,
    /// Retrying will not help.
    Permanent,
}

/// JSON-RPC codes some Soroban RPC providers use for rate limiting.
const JSONRPC_THROTTLE_CODES: &[i64] = &[-32005, -32029];
/// JSON-RPC internal error.
const JSONRPC_INTERNAL_ERROR: i64 = -32603;

impl RpcError {
    pub fn class(&self) -> ErrorClass {
        match self {
            Self::Connect(_) | Self::Throttled { .. } => ErrorClass::Retryable,
            Self::Timeout => ErrorClass::RetryableIfIdempotent,
            Self::Http(408 | 500 | 502 | 504) => ErrorClass::RetryableIfIdempotent,
            Self::Http(_) => ErrorClass::Permanent,
            Self::JsonRpc { code, .. } if JSONRPC_THROTTLE_CODES.contains(code) => {
                ErrorClass::Retryable
            }
            Self::JsonRpc { code, .. } if *code == JSONRPC_INTERNAL_ERROR => {
                ErrorClass::RetryableIfIdempotent
            }
            Self::JsonRpc { .. } | Self::InvalidResponse(_) => ErrorClass::Permanent,
            Self::CircuitOpen | Self::DeadlineExceeded => ErrorClass::Permanent,
        }
    }

    /// Whether the failure says the node is unhealthy (counts toward opening
    /// the circuit).
    pub fn is_upstream_failure(&self) -> bool {
        match self {
            Self::Timeout | Self::Connect(_) | Self::Throttled { .. } => true,
            Self::Http(status) => *status >= 500 || *status == 408,
            Self::JsonRpc { code, .. } => {
                *code == JSONRPC_INTERNAL_ERROR || JSONRPC_THROTTLE_CODES.contains(code)
            }
            Self::InvalidResponse(_) | Self::CircuitOpen | Self::DeadlineExceeded => false,
        }
    }

    fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Throttled { retry_after } => *retry_after,
            _ => None,
        }
    }
}

/// Whether repeating a call can change state on the node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Idempotency {
    Idempotent,
    NonIdempotent,
}

/// Idempotency of a Soroban RPC method. Only submission changes state.
pub fn method_idempotency(method: &str) -> Idempotency {
    match method {
        "sendTransaction" => Idempotency::NonIdempotent,
        _ => Idempotency::Idempotent,
    }
}

fn should_retry(class: ErrorClass, idempotency: Idempotency) -> bool {
    match class {
        ErrorClass::Retryable => true,
        ErrorClass::RetryableIfIdempotent => idempotency == Idempotency::Idempotent,
        ErrorClass::Permanent => false,
    }
}

// ── Clock ────────────────────────────────────────────────────────────────────

/// Monotonic time source. `now` is the time elapsed since the clock started.
pub trait Clock: Send + Sync {
    fn now(&self) -> Duration;
    fn sleep(&self, duration: Duration) -> BoxFuture<'_, ()>;
}

/// Real clock backed by Tokio.
#[derive(Debug)]
pub struct SystemClock {
    start: tokio::time::Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            start: tokio::time::Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }
    fn sleep(&self, duration: Duration) -> BoxFuture<'_, ()> {
        Box::pin(tokio::time::sleep(duration))
    }
}

/// Manually driven clock for deterministic tests: `sleep` returns at once
/// and advances time by the requested duration.
#[derive(Debug, Default)]
pub struct FakeClock {
    nanos: AtomicU64,
}

impl FakeClock {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn advance(&self, duration: Duration) {
        self.nanos
            .fetch_add(duration.as_nanos() as u64, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        Duration::from_nanos(self.nanos.load(Ordering::SeqCst))
    }
    fn sleep(&self, duration: Duration) -> BoxFuture<'_, ()> {
        self.advance(duration);
        Box::pin(std::future::ready(()))
    }
}

// ── Retry policy ─────────────────────────────────────────────────────────────

/// Bounded exponential backoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total attempts, including the first (minimum 1).
    pub max_attempts: u32,
    /// Delay before the first retry, before jitter.
    pub base_delay: Duration,
    /// Cap on any single delay.
    pub max_delay: Duration,
    /// Timeout of a single attempt (clipped to the remaining deadline).
    pub attempt_timeout: Duration,
    /// Overall deadline of a call, including all attempts and sleeps.
    pub max_elapsed: Duration,
    /// Apply equal jitter. Disable only in tests that assert exact delays.
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(2),
            attempt_timeout: Duration::from_secs(5),
            max_elapsed: Duration::from_secs(10),
            jitter: true,
        }
    }
}

impl RetryPolicy {
    /// Policy for background work, derived from `RPC_TIMEOUT_SECS`.
    pub fn from_config(config: &crate::config::Config) -> Self {
        let attempt_timeout = Duration::from_secs(config.rpc_timeout_secs.max(1));
        Self {
            attempt_timeout,
            max_elapsed: attempt_timeout * 2,
            ..Self::default()
        }
    }

    /// Short policy for health probes, derived from
    /// `RPC_HEALTH_TIMEOUT_SECS` and `RPC_HEALTH_RETRY_COUNT`. The overall
    /// deadline equals one attempt timeout plus a small retry budget, so a
    /// dead node cannot make `/health` slow.
    pub fn health_probe(config: &crate::config::Config) -> Self {
        let attempt_timeout = Duration::from_secs(config.rpc_health_timeout_secs.max(1));
        Self {
            max_attempts: u32::from(config.rpc_health_retry_count.max(1)),
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(500),
            attempt_timeout,
            max_elapsed: attempt_timeout + Duration::from_secs(1),
            jitter: true,
        }
    }

    /// Backoff before retry number `retry` (0 = first retry).
    pub fn backoff(&self, retry: u32, seed: u64) -> Duration {
        let exp = self
            .base_delay
            .saturating_mul(1u32.checked_shl(retry.min(31)).unwrap_or(u32::MAX));
        let capped = exp.min(self.max_delay);
        if !self.jitter || capped.is_zero() {
            return capped;
        }
        let half = capped / 2;
        let span = (capped - half).as_nanos() as u64;
        half + Duration::from_nanos(splitmix64(seed) % span.max(1))
    }
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

// ── Circuit breaker ──────────────────────────────────────────────────────────

/// Circuit breaker thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,
    pub open_duration: Duration,
    pub half_open_max_probes: u32,
    pub success_threshold: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            open_duration: Duration::from_secs(30),
            half_open_max_probes: 1,
            success_threshold: 1,
        }
    }
}

/// Observable breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

impl CircuitState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Open => "open",
            Self::HalfOpen => "half_open",
        }
    }
}

#[derive(Debug)]
enum Inner {
    Closed { consecutive_failures: u32 },
    Open { until: Duration },
    HalfOpen { in_flight: u32, successes: u32 },
}

/// Thread-safe circuit breaker shared by all callers of one RPC endpoint.
#[derive(Debug)]
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    inner: Mutex<Inner>,
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            inner: Mutex::new(Inner::Closed {
                consecutive_failures: 0,
            }),
        }
    }

    /// Current state at time `now` (an expired open state reads as half-open).
    pub fn state(&self, now: Duration) -> CircuitState {
        match *self.lock() {
            Inner::Closed { .. } => CircuitState::Closed,
            Inner::Open { until } if now >= until => CircuitState::HalfOpen,
            Inner::Open { .. } => CircuitState::Open,
            Inner::HalfOpen { .. } => CircuitState::HalfOpen,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Ask to send one request. Returns a permit that must be resolved with
    /// [`Permit::success`] or [`Permit::failure`]; dropping it unresolved
    /// (e.g. a cancelled call) releases a half-open probe slot.
    pub fn try_acquire(self: &Arc<Self>, now: Duration) -> Result<Permit, RpcError> {
        let mut inner = self.lock();
        let (next, probe) = match &*inner {
            Inner::Closed { .. } => (None, false),
            Inner::Open { until } if now < *until => return Err(RpcError::CircuitOpen),
            Inner::Open { .. } => {
                tracing::info!("rpc circuit half-open; probing");
                (
                    Some(Inner::HalfOpen {
                        in_flight: 1,
                        successes: 0,
                    }),
                    true,
                )
            }
            Inner::HalfOpen {
                in_flight,
                successes,
            } => {
                if *in_flight >= self.config.half_open_max_probes {
                    return Err(RpcError::CircuitOpen);
                }
                (
                    Some(Inner::HalfOpen {
                        in_flight: in_flight + 1,
                        successes: *successes,
                    }),
                    true,
                )
            }
        };
        if let Some(next) = next {
            *inner = next;
        }
        Ok(Permit {
            breaker: Arc::clone(self),
            probe,
            resolved: false,
        })
    }

    fn record(&self, probe: bool, success: bool, now: Duration) {
        let mut inner = self.lock();
        let next = match &*inner {
            Inner::Closed { .. } if success => Inner::Closed {
                consecutive_failures: 0,
            },
            Inner::Closed {
                consecutive_failures,
            } => {
                let failures = consecutive_failures + 1;
                if failures >= self.config.failure_threshold {
                    tracing::warn!(failures, "rpc circuit opened");
                    Inner::Open {
                        until: now + self.config.open_duration,
                    }
                } else {
                    Inner::Closed {
                        consecutive_failures: failures,
                    }
                }
            }
            Inner::HalfOpen {
                in_flight,
                successes,
            } if probe => {
                if !success {
                    tracing::warn!("rpc circuit probe failed; reopening");
                    Inner::Open {
                        until: now + self.config.open_duration,
                    }
                } else if successes + 1 >= self.config.success_threshold {
                    tracing::info!("rpc circuit closed");
                    Inner::Closed {
                        consecutive_failures: 0,
                    }
                } else {
                    Inner::HalfOpen {
                        in_flight: in_flight.saturating_sub(1),
                        successes: successes + 1,
                    }
                }
            }
            // Results of requests started in an earlier state are ignored.
            _ => return,
        };
        *inner = next;
    }

    fn release_probe(&self) {
        let mut inner = self.lock();
        if let Inner::HalfOpen { in_flight, .. } = &mut *inner {
            *in_flight = in_flight.saturating_sub(1);
        }
    }
}

/// Permission to send one request through a [`CircuitBreaker`].
#[must_use]
pub struct Permit {
    breaker: Arc<CircuitBreaker>,
    probe: bool,
    resolved: bool,
}

impl Permit {
    /// The node answered (even with a non-retryable client error).
    pub fn success(mut self, now: Duration) {
        self.resolved = true;
        self.breaker.record(self.probe, true, now);
    }
    /// The node is unhealthy (timeout, connection error, 5xx, throttling).
    pub fn failure(mut self, now: Duration) {
        self.resolved = true;
        self.breaker.record(self.probe, false, now);
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        if !self.resolved && self.probe {
            self.breaker.release_probe();
        }
    }
}

/// Process-wide breaker for an RPC URL, shared by API handlers and workers.
pub fn shared_breaker(url: &str) -> Arc<CircuitBreaker> {
    static BREAKERS: OnceLock<Mutex<HashMap<String, Arc<CircuitBreaker>>>> = OnceLock::new();
    let mut map = BREAKERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    Arc::clone(
        map.entry(url.to_string())
            .or_insert_with(|| Arc::new(CircuitBreaker::new(CircuitBreakerConfig::default()))),
    )
}

// ── Transport ────────────────────────────────────────────────────────────────

/// Sends one JSON-RPC request. Implementations must return
/// [`RpcError::Timeout`] when `timeout` elapses.
pub trait RpcTransport: Send + Sync {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: &'a Value,
        timeout: Duration,
    ) -> BoxFuture<'a, Result<Value, RpcError>>;
}

/// HTTP JSON-RPC transport for a Soroban RPC URL.
#[derive(Clone)]
pub struct HttpTransport {
    client: reqwest::Client,
    url: String,
}

impl HttpTransport {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: url.into(),
        }
    }

    async fn send_inner(
        &self,
        method: &str,
        params: &Value,
        timeout: Duration,
    ) -> Result<Value, RpcError> {
        let mut body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method });
        if !params.is_null() {
            body["params"] = params.clone();
        }
        let response = self
            .client
            .post(&self.url)
            .timeout(timeout)
            .json(&body)
            .send()
            .await
            .map_err(classify_reqwest)?;

        let status = response.status();
        if status.as_u16() == 429 || status.as_u16() == 503 {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            return Err(RpcError::Throttled { retry_after });
        }
        if !status.is_success() {
            return Err(RpcError::Http(status.as_u16()));
        }

        let value: Value = response.json().await.map_err(|e| {
            if e.is_timeout() {
                RpcError::Timeout
            } else {
                RpcError::InvalidResponse(e.to_string())
            }
        })?;
        parse_jsonrpc(value)
    }
}

impl RpcTransport for HttpTransport {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: &'a Value,
        timeout: Duration,
    ) -> BoxFuture<'a, Result<Value, RpcError>> {
        Box::pin(async move {
            // Outer guard in case the body read stalls past the client timeout.
            match tokio::time::timeout(timeout, self.send_inner(method, params, timeout)).await {
                Ok(result) => result,
                Err(_) => Err(RpcError::Timeout),
            }
        })
    }
}

fn classify_reqwest(e: reqwest::Error) -> RpcError {
    if e.is_timeout() {
        RpcError::Timeout
    } else if e.is_connect() {
        RpcError::Connect(e.to_string())
    } else {
        // Failure while sending: the node may have received the request.
        RpcError::InvalidResponse(e.to_string())
    }
}

/// Extract `result` from a JSON-RPC response, mapping `error` objects.
pub fn parse_jsonrpc(value: Value) -> Result<Value, RpcError> {
    if let Some(error) = value.get("error").filter(|e| !e.is_null()) {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error")
            .chars()
            .take(256)
            .collect();
        if JSONRPC_THROTTLE_CODES.contains(&code) {
            return Err(RpcError::Throttled { retry_after: None });
        }
        return Err(RpcError::JsonRpc { code, message });
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| RpcError::InvalidResponse("missing result".to_string()))
}

// ── Client ───────────────────────────────────────────────────────────────────

/// Soroban RPC client with classification, bounded retries, deadlines and a
/// shared circuit breaker.
#[derive(Clone)]
pub struct ResilientRpcClient {
    transport: Arc<dyn RpcTransport>,
    breaker: Arc<CircuitBreaker>,
    clock: Arc<dyn Clock>,
    policy: RetryPolicy,
    seed: Arc<AtomicU64>,
}

impl ResilientRpcClient {
    pub fn new(
        transport: Arc<dyn RpcTransport>,
        breaker: Arc<CircuitBreaker>,
        clock: Arc<dyn Clock>,
        policy: RetryPolicy,
    ) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        Self {
            transport,
            breaker,
            clock,
            policy,
            seed: Arc::new(AtomicU64::new(seed)),
        }
    }

    /// HTTP client for `url` using the process-wide breaker for that URL.
    pub fn http(url: &str, policy: RetryPolicy) -> Self {
        Self::new(
            Arc::new(HttpTransport::new(url)),
            shared_breaker(url),
            Arc::new(SystemClock::default()),
            policy,
        )
    }

    /// Client for the configured Soroban RPC URL with the background policy.
    pub fn from_config(config: &crate::config::Config) -> Self {
        Self::http(&config.stellar_rpc_url, RetryPolicy::from_config(config))
    }

    /// Fix the jitter seed (for deterministic tests).
    pub fn with_seed(self, seed: u64) -> Self {
        self.seed.store(seed, Ordering::SeqCst);
        self
    }

    pub fn circuit_state(&self) -> CircuitState {
        self.breaker.state(self.clock.now())
    }

    /// Call `method`, using [`method_idempotency`] to decide retry safety.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.call_with(method, params, method_idempotency(method))
            .await
    }

    /// Call `method` with an explicit idempotency.
    pub async fn call_with(
        &self,
        method: &str,
        params: Value,
        idempotency: Idempotency,
    ) -> Result<Value, RpcError> {
        let policy = self.policy;
        let start = self.clock.now();
        let deadline = start + policy.max_elapsed;
        let max_attempts = policy.max_attempts.max(1);
        let mut attempt: u32 = 0;

        loop {
            let now = self.clock.now();
            let remaining = deadline.saturating_sub(now);
            if remaining.is_zero() {
                return Err(RpcError::DeadlineExceeded);
            }

            let permit = self.breaker.try_acquire(now)?;
            let timeout = policy.attempt_timeout.min(remaining);
            let result = self.transport.send(method, &params, timeout).await;
            let now = self.clock.now();

            let error = match result {
                Ok(value) => {
                    permit.success(now);
                    return Ok(value);
                }
                Err(error) => error,
            };
            if error.is_upstream_failure() {
                permit.failure(now);
            } else {
                permit.success(now);
            }

            attempt += 1;
            let retry = should_retry(error.class(), idempotency) && attempt < max_attempts;
            if !retry {
                tracing::warn!(method, attempt, error = %error, "rpc call failed");
                return Err(error);
            }

            let seed = self.seed.fetch_add(1, Ordering::Relaxed);
            let mut delay = policy.backoff(attempt - 1, seed);
            if let Some(retry_after) = error.retry_after() {
                delay = delay.max(retry_after);
            }
            // Never sleep into, or past, the deadline: there would be no time
            // left for the attempt.
            if now + delay >= deadline {
                tracing::warn!(method, attempt, error = %error, "rpc retry budget exhausted");
                return Err(error);
            }
            tracing::debug!(method, attempt, delay_ms = delay.as_millis() as u64, error = %error, "retrying rpc call");
            self.clock.sleep(delay).await;
        }
    }
}

/// Probe `getHealth` with the short health-check policy.
pub async fn probe_health(config: &crate::config::Config) -> Result<(), RpcError> {
    ResilientRpcClient::http(&config.stellar_rpc_url, RetryPolicy::health_probe(config))
        .call("getHealth", Value::Null)
        .await
        .map(|_| ())
}

// ── Quotas, priority classes and failover ────────────────────────────────────

/// Priority class of an RPC request. Higher-priority classes are served first
/// when a network's request budget is contended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PriorityClass {
    /// Background indexing / backfill work.
    Background = 0,
    /// User-facing transaction preparation (simulation, fee estimation).
    UserTransaction = 1,
    /// Health probes and operator tooling.
    Critical = 2,
}

impl PriorityClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::UserTransaction => "user_transaction",
            Self::Critical => "critical",
        }
    }
}

/// Per-network request budget: a token bucket refilled at `refill_per_sec`
/// with a burst capacity of `burst`. Each request costs one token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestBudget {
    pub burst: u32,
    pub refill_per_sec: u32,
}

impl RequestBudget {
    pub const fn new(burst: u32, refill_per_sec: u32) -> Self {
        Self {
            burst,
            refill_per_sec,
        }
    }
}

/// Budgets and fallback ordering for one network.
#[derive(Debug, Clone)]
pub struct NetworkPolicy {
    /// Network identifier (e.g. `"testnet"`, `"mainnet"`).
    pub network: String,
    /// Budget applied to each provider URL on this network.
    pub budget: RequestBudget,
    /// Provider URLs in fallback order (index 0 is preferred).
    pub providers: Vec<String>,
    /// Retry policy used for calls on this network.
    pub retry: RetryPolicy,
}

impl NetworkPolicy {
    pub fn new(
        network: impl Into<String>,
        budget: RequestBudget,
        providers: Vec<String>,
        retry: RetryPolicy,
    ) -> Self {
        Self {
            network: network.into(),
            budget,
            providers,
            retry,
        }
    }
}

/// Health of a single provider endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderHealth {
    /// Serving requests normally.
    Healthy,
    /// Recently failed; still tried but ranked below healthy providers.
    Degraded,
    /// Circuit open or repeatedly failing; skipped until it recovers.
    Down,
}

impl ProviderHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Down => "down",
        }
    }
}

/// Snapshot of one provider for the operator dashboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderStatus {
    pub url: String,
    pub health: ProviderHealth,
    pub consecutive_failures: u32,
    pub total_requests: u64,
    pub total_failures: u64,
    pub circuit_state: CircuitState,
}

/// What to do when two providers return different results for the same
/// idempotent read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisagreementPolicy {
    /// Return the first provider's answer and log the disagreement.
    TrustPrimary,
    /// Treat the disagreement as an error so the caller can retry later.
    Fail,
}

impl Default for DisagreementPolicy {
    fn default() -> Self {
        Self::TrustPrimary
    }
}

/// Metrics sink for the quota/failover manager. Implementations forward to
/// whatever metrics backend the process uses; the default is a no-op.
pub trait RpcMetrics: Send + Sync {
    fn on_request(&self, _network: &str, _provider: &str, _priority: PriorityClass) {}
    fn on_retry(&self, _network: &str, _provider: &str, _attempt: u32) {}
    fn on_failure(&self, _network: &str, _provider: &str, _error: &RpcError) {}
    fn on_failover(&self, _network: &str, _from: &str, _to: &str) {}
    fn on_budget_exhausted(&self, _network: &str, _priority: PriorityClass) {}
    fn on_disagreement(&self, _network: &str, _method: &str) {}
}

/// No-op metrics sink.
#[derive(Debug, Default)]
pub struct NoopMetrics;

impl RpcMetrics for NoopMetrics {}

#[derive(Debug, Default)]
struct BudgetState {
    tokens: f64,
    last_refill: Duration,
    initialized: bool,
}

#[derive(Debug, Default)]
struct ProviderState {
    consecutive_failures: u32,
    total_requests: u64,
    total_failures: u64,
}

/// Per-network quota and failover manager.
///
/// * Enforces a token-bucket [`RequestBudget`] per provider.
/// * Tracks [`ProviderHealth`] and orders providers by health, then by the
///   configured fallback order.
/// * Retries across providers with bounded backoff and per-provider circuit
///   breakers.
/// * Detects disagreement between providers on idempotent reads.
/// * Uses idempotency keys for `sendTransaction` so a retry after an
///   ambiguous failure never duplicates submission.
pub struct RpcQuotaManager {
    policy: NetworkPolicy,
    clock: Arc<dyn Clock>,
    metrics: Arc<dyn RpcMetrics>,
    disagreement: DisagreementPolicy,
    budgets: Mutex<HashMap<String, BudgetState>>,
    providers: Mutex<HashMap<String, ProviderState>>,
    breakers: HashMap<String, Arc<CircuitBreaker>>,
    transports: HashMap<String, Arc<dyn RpcTransport>>,
    seed: AtomicU64,
}

impl std::fmt::Debug for RpcQuotaManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcQuotaManager")
            .field("network", &self.policy.network)
            .field("providers", &self.policy.providers)
            .finish()
    }
}

impl RpcQuotaManager {
    /// Build a manager for `policy`, creating an HTTP transport and a shared
    /// circuit breaker per provider URL.
    pub fn new(policy: NetworkPolicy, clock: Arc<dyn Clock>) -> Self {
        let mut transports = HashMap::new();
        let mut breakers = HashMap::new();
        for url in &policy.providers {
            transports.insert(url.clone(), Arc::new(HttpTransport::new(url)) as Arc<dyn RpcTransport>);
            breakers.insert(url.clone(), shared_breaker(url));
        }
        Self {
            policy,
            clock,
            metrics: Arc::new(NoopMetrics),
            disagreement: DisagreementPolicy::default(),
            budgets: Mutex::new(HashMap::new()),
            providers: Mutex::new(HashMap::new()),
            breakers,
            transports,
            seed: AtomicU64::new(0),
        }
    }

    /// Override the transport for a provider (tests / custom clients).
    pub fn with_transport(mut self, url: &str, transport: Arc<dyn RpcTransport>) -> Self {
        self.transports.insert(url.to_string(), transport);
        self
    }

    /// Override the circuit breaker for a provider (tests).
    pub fn with_breaker(mut self, url: &str, breaker: Arc<CircuitBreaker>) -> Self {
        self.breakers.insert(url.to_string(), breaker);
        self
    }

    pub fn with_metrics(mut self, metrics: Arc<dyn RpcMetrics>) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn with_disagreement_policy(mut self, policy: DisagreementPolicy) -> Self {
        self.disagreement = policy;
        self
    }

    pub fn with_seed(self, seed: u64) -> Self {
        self.seed.store(seed, Ordering::SeqCst);
        self
    }

    pub fn network(&self) -> &str {
        &self.policy.network
    }

    /// Snapshot of every provider for the operator dashboard.
    pub fn status(&self) -> Vec<ProviderStatus> {
        let now = self.clock.now();
        let providers = self.providers.lock().unwrap_or_else(|p| p.into_inner());
        self.policy
            .providers
            .iter()
            .map(|url| {
                let state = providers.get(url);
                let circuit_state = self
                    .breakers
                    .get(url)
                    .map(|b| b.state(now))
                    .unwrap_or(CircuitState::Closed);
                let consecutive_failures = state.map(|s| s.consecutive_failures).unwrap_or(0);
                let health = if circuit_state == CircuitState::Open {
                    ProviderHealth::Down
                } else if consecutive_failures > 0 {
                    ProviderHealth::Degraded
                } else {
                    ProviderHealth::Healthy
                };
                ProviderStatus {
                    url: url.clone(),
                    health,
                    consecutive_failures,
                    total_requests: state.map(|s| s.total_requests).unwrap_or(0),
                    total_failures: state.map(|s| s.total_failures).unwrap_or(0),
                    circuit_state,
                }
            })
            .collect()
    }

    /// Providers in the order they should be tried: healthy first, then
    /// degraded, then down; ties broken by configured fallback order.
    fn ordered_providers(&self) -> Vec<String> {
        let now = self.clock.now();
        let providers = self.providers.lock().unwrap_or_else(|p| p.into_inner());
        let mut ranked: Vec<(u8, usize, String)> = self
            .policy
            .providers
            .iter()
            .enumerate()
            .map(|(idx, url)| {
                let circuit = self
                    .breakers
                    .get(url)
                    .map(|b| b.state(now))
                    .unwrap_or(CircuitState::Closed);
                let failures = providers.get(url).map(|s| s.consecutive_failures).unwrap_or(0);
                let rank = if circuit == CircuitState::Open {
                    2
                } else if failures > 0 {
                    1
                } else {
                    0
                };
                (rank, idx, url.clone())
            })
            .collect();
        ranked.sort_by_key(|(rank, idx, _)| (*rank, *idx));
        ranked.into_iter().map(|(_, _, url)| url).collect()
    }

    /// Try to take one token from `url`'s budget. Returns `false` when the
    /// budget is exhausted for the current instant.
    fn try_take_token(&self, url: &str, now: Duration) -> bool {
        let budget = self.policy.budget;
        let mut budgets = self.budgets.lock().unwrap_or_else(|p| p.into_inner());
        let state = budgets.entry(url.to_string()).or_default();
        if !state.initialized {
            state.tokens = f64::from(budget.burst);
            state.last_refill = now;
            state.initialized = true;
        } else {
            let elapsed = now.saturating_sub(state.last_refill).as_secs_f64();
            if elapsed > 0.0 {
                state.tokens =
                    (state.tokens + elapsed * f64::from(budget.refill_per_sec)).min(f64::from(budget.burst));
                state.last_refill = now;
            }
        }
        if state.tokens >= 1.0 {
            state.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn record_outcome(&self, url: &str, success: bool) {
        let mut providers = self.providers.lock().unwrap_or_else(|p| p.into_inner());
        let state = providers.entry(url.to_string()).or_default();
        state.total_requests += 1;
        if success {
            state.consecutive_failures = 0;
        } else {
            state.consecutive_failures = state.consecutive_failures.saturating_add(1);
            state.total_failures += 1;
        }
    }

    /// Call `method` with the given priority class, applying quotas, failover
    /// and bounded retries.
    pub async fn call(
        &self,
        method: &str,
        params: Value,
        priority: PriorityClass,
    ) -> Result<Value, RpcError> {
        let idempotency = method_idempotency(method);
        let policy = self.policy.retry;
        let start = self.clock.now();
        let deadline = start + policy.max_elapsed;
        let max_attempts = policy.max_attempts.max(1);
        let mut attempt: u32 = 0;
        let mut last_error: Option<RpcError> = None;
        let mut last_provider: Option<String> = None;

        loop {
            if attempt >= max_attempts {
                break;
            }
            let now = self.clock.now();
            let remaining = deadline.saturating_sub(now);
            if remaining.is_zero() {
                return Err(last_error.unwrap_or(RpcError::DeadlineExceeded));
            }

            let ordered = self.ordered_providers();
            let mut chosen: Option<String> = None;
            for url in &ordered {
                if self.try_take_token(url, now) {
                    chosen = Some(url.clone());
                    break;
                }
            }
            let Some(url) = chosen else {
                self.metrics.on_budget_exhausted(&self.policy.network, priority);
                // No provider has budget right now. If we already have a
                // retryable error, surface it; otherwise report throttling.
                return Err(last_error.unwrap_or(RpcError::Throttled { retry_after: None }));
            };

            if let Some(prev) = &last_provider {
                if prev != &url {
                    self.metrics.on_failover(&self.policy.network, prev, &url);
                }
            }
            last_provider = Some(url.clone());
            self.metrics.on_request(&self.policy.network, &url, priority);

            let Some(transport) = self.transports.get(&url) else {
                return Err(RpcError::Connect(format!("no transport for {url}")));
            };
            let breaker = self
                .breakers
                .get(&url)
                .cloned()
                .unwrap_or_else(|| Arc::new(CircuitBreaker::new(CircuitBreakerConfig::default())));

            let permit = match breaker.try_acquire(now) {
                Ok(permit) => permit,
                Err(e) => {
                    // Circuit open: try the next provider without consuming
                    // an attempt.
                    last_error = Some(e);
                    continue;
                }
            };

            let timeout = policy.attempt_timeout.min(remaining);
            let result = transport.send(method, &params, timeout).await;
            let now = self.clock.now();

            match result {
                Ok(value) => {
                    permit.success(now);
                    self.record_outcome(&url, true);
                    return Ok(value);
                }
                Err(error) => {
                    if error.is_upstream_failure() {
                        permit.failure(now);
                    } else {
                        permit.success(now);
                    }
                    self.record_outcome(&url, false);
                    self.metrics.on_failure(&self.policy.network, &url, &error);

                    attempt += 1;
                    let retry = should_retry(error.class(), idempotency) && attempt < max_attempts;
                    if !retry {
                        tracing::warn!(network = %self.policy.network, method, attempt, error = %error, "rpc call failed");
                        return Err(error);
                    }

                    let seed = self.seed.fetch_add(1, Ordering::Relaxed);
                    let mut delay = policy.backoff(attempt - 1, seed);
                    if let Some(retry_after) = error.retry_after() {
                        delay = delay.max(retry_after);
                    }
                    if now + delay >= deadline {
                        tracing::warn!(network = %self.policy.network, method, attempt, error = %error, "rpc retry budget exhausted");
                        return Err(error);
                    }
                    self.metrics.on_retry(&self.policy.network, &url, attempt);
                    tracing::debug!(network = %self.policy.network, method, attempt, provider = %url, delay_ms = delay.as_millis() as u64, error = %error, "retrying rpc call");
                    last_error = Some(error);
                    self.clock.sleep(delay).await;
                }
            }
        }

        Err(last_error.unwrap_or(RpcError::DeadlineExceeded))
    }

    /// Idempotent read with disagreement detection: queries the two highest
    /// ranked providers and compares results. On disagreement the configured
    /// [`DisagreementPolicy`] decides the outcome.
    pub async fn call_consensus(
        &self,
        method: &str,
        params: Value,
        priority: PriorityClass,
    ) -> Result<Value, RpcError> {
        let first = self.call(method, params.clone(), priority).await?;
        let ordered = self.ordered_providers();
        let Some(second_url) = ordered.get(1).cloned() else {
            return Ok(first);
        };
        let Some(transport) = self.transports.get(&second_url).cloned() else {
            return Ok(first);
        };
        let now = self.clock.now();
        if !self.try_take_token(&second_url, now) {
            return Ok(first);
        }
        let timeout = self.policy.retry.attempt_timeout;
        match transport.send(method, &params, timeout).await {
            Ok(second) => {
                self.record_outcome(&second_url, true);
                if second == first {
                    Ok(first)
                } else {
                    self.metrics.on_disagreement(&self.policy.network, method);
                    tracing::warn!(network = %self.policy.network, method, "rpc providers disagree");
                    match self.disagreement {
                        DisagreementPolicy::TrustPrimary => Ok(first),
                        DisagreementPolicy::Fail => Err(RpcError::InvalidResponse(
                            "rpc providers disagree".to_string(),
                        )),
                    }
                }
            }
            Err(error) => {
                self.record_outcome(&second_url, false);
                self.metrics.on_failure(&self.policy.network, &second_url, &error);
                Ok(first)
            }
        }
    }
}

/// Process-wide registry of [`RpcQuotaManager`]s keyed by network.
pub fn shared_quota_manager(policy: NetworkPolicy) -> Arc<RpcQuotaManager> {
    static MANAGERS: OnceLock<Mutex<HashMap<String, Arc<RpcQuotaManager>>>> = OnceLock::new();
    let mut map = MANAGERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    Arc::clone(
        map.entry(policy.network.clone())
            .or_insert_with(|| Arc::new(RpcQuotaManager::new(policy, Arc::new(SystemClock::default())))),
    )
}
