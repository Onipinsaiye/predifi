//! Per-route rate limiting helpers for the backend API.
//!
//! The server applies one of the tiers defined in [`crate::constants`] to each
//! route group:
//!
//! - `Light` — cheap, stateless endpoints such as `/fees`, `/prices`, and
//!   `/health`; these are the most permissive and are intended for polling and
//!   health checks.
//! - `Read` — public read-only endpoints such as `/pools`, `/stats`, and
//!   `/leaderboard`; these are cached and human-readable data views.
//! - `User` — per-user history and prediction endpoints that are scoped to a
//!   specific wallet.
//! - `Write` — mutating or indexer-ingest routes under `/indexer/*`.
//! - `Token` — authentication and WebSocket handshake routes.
//!
//! ## Distributed semantics
//!
//! [`with_distributed_rate_limit`] keeps the counters in Redis so every replica
//! behind a load balancer shares the same limit. Requests are partitioned
//! deterministically by:
//!
//! - the authenticated `x-user-address` header for the `User` tier, and
//! - the client IP (trusting `X-Forwarded-For` / `X-Real-IP` only because the
//!   deployment sits behind the configured proxy) for every other tier.
//!
//! The counter is a fixed window keyed as
//! `predifi:rate_limit:<tier>:<identity>`. All replicas use the same key and an
//! atomic Lua `INCR`/`PEXPIRE`, so a burst spread across instances cannot
//! exceed the configured burst size.
//!
//! ## Redis outage policy
//!
//! The policies are deliberately different by tier:
//!
//! | Tier(s) | Redis unavailable |
//! | --- | --- |
//! | `Light`, `Read`, `User` | **Fail open**: serve the request (availability over a temporary, degraded limit). |
//! | `Write`, `Token` | **Fail closed**: return `503` so writes and credential operations cannot run without the shared limiter. |
//!
//! This is documented in `RATE_LIMITING.md` and `.env.example`.
//!
//! When a client exceeds its window the server replies with HTTP 429 and the
//! standardized JSON body returned by [`crate::response`], plus
//! `X-RateLimit-*` headers and `Retry-After`.

#![cfg_attr(test, allow(dead_code, unused_imports))]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header::HeaderValue, Extensions, HeaderMap},
    middleware::Next,
    response::Response,
};
use tower_governor::key_extractor::SmartIpKeyExtractor;

use crate::redis_cache::RedisCache;

/// Rate-limit tier — maps to a `(burst_size, period_secs)` pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateLimitTier {
    /// Cheap stateless endpoints (`/fees`, `/prices`, `/health`). 120 req / 60 s.
    Light,
    /// Public read endpoints (`/pools`, `/stats`, `/leaderboard`). 60 req / 60 s.
    Read,
    /// Per-user history / prediction endpoints. 30 req / 60 s.
    User,
    /// Indexer ingest endpoints (`/indexer/*`). 20 req / 60 s.
    Write,
    /// Token operations (`/auth/refresh`). 10 req / 60 s.
    Token,
    /// WebSocket inbound messages. 10 msg / 10 s per connection.
    WebSocket,
}

/// What to do when Redis is unavailable for a tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateLimitFailurePolicy {
    /// Serve the request and rely on the per-instance fallback (availability).
    FailOpen,
    /// Reject the request because the shared limiter is unavailable (safety).
    FailClosed,
}

impl RateLimitTier {
    /// Human-readable, stable tier name used in Redis keys and metrics.
    pub fn label(self) -> &'static str {
        match self {
            RateLimitTier::Light => "light",
            RateLimitTier::Read => "read",
            RateLimitTier::User => "user",
            RateLimitTier::Write => "write",
            RateLimitTier::Token => "token",
            RateLimitTier::WebSocket => "websocket",
        }
    }

    /// The configured `(burst, period_secs)` for this tier.
    pub fn burst_and_period(self) -> (u32, u64) {
        use crate::constants::*;
        match self {
            RateLimitTier::Light => (RATE_LIMIT_LIGHT_BURST, RATE_LIMIT_LIGHT_PERIOD_SECS),
            RateLimitTier::Read => (RATE_LIMIT_READ_BURST, RATE_LIMIT_READ_PERIOD_SECS),
            RateLimitTier::User => (RATE_LIMIT_USER_BURST, RATE_LIMIT_USER_PERIOD_SECS),
            RateLimitTier::Write => (RATE_LIMIT_WRITE_BURST, RATE_LIMIT_WRITE_PERIOD_SECS),
            RateLimitTier::Token => (RATE_LIMIT_TOKEN_BURST, RATE_LIMIT_TOKEN_PERIOD_SECS),
            RateLimitTier::WebSocket => (RATE_LIMIT_WS_BURST, RATE_LIMIT_WS_PERIOD_SECS),
        }
    }

    /// The Redis-outage policy for this tier.
    ///
    /// Reads fail open so a Redis incident does not take the public API down.
    /// Writes and authentication fail closed so an attacker cannot exploit a
    /// limiter outage to brute-force credentials or flood ingest.
    pub fn failure_policy(self) -> RateLimitFailurePolicy {
        match self {
            RateLimitTier::Light | RateLimitTier::Read | RateLimitTier::User => {
                RateLimitFailurePolicy::FailOpen
            }
            RateLimitTier::Write | RateLimitTier::Token | RateLimitTier::WebSocket => {
                RateLimitFailurePolicy::FailClosed
            }
        }
    }
}

/// Build the deterministic Redis key for a tier/identity pair.
///
/// The identity is sanitized so a spoofed forwarded header cannot inject
/// namespace separators or glob metacharacters.
pub fn rate_limit_key(tier: RateLimitTier, identity: &str) -> String {
    // Keep the key deterministic and bounded without slicing a UTF-8 string at
    // an arbitrary byte boundary. Only ASCII address/name characters survive,
    // so a spoofed header cannot inject namespace separators or globs.
    let identity: String = identity
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .take(128)
        .collect::<String>()
        .to_ascii_lowercase();
    let identity = if identity.is_empty() {
        "unknown".to_string()
    } else {
        identity
    };
    format!("rate_limit:{}:{}", tier.label(), identity)
}

/// Resolve the partition identity for a request.
///
/// `User` is per-wallet when the JWT middleware has populated
/// `x-user-address`; all other tiers use the client IP. When no address is
/// available (unit tests, health probes) the identity is `unknown`, which still
/// yields a stable shared bucket rather than bypassing the limiter.
pub fn request_identity(
    tier: RateLimitTier,
    headers: &HeaderMap,
    extensions: &Extensions,
) -> String {
    if tier == RateLimitTier::User {
        if let Some(user) = headers
            .get("x-user-address")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return user.to_string();
        }
    }

    if let Some(forwarded) = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return forwarded.to_string();
    }
    if let Some(real_ip) = headers
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return real_ip.to_string();
    }
    if let Some(info) = extensions.get::<ConnectInfo<SocketAddr>>() {
        return info.0.ip().to_string();
    }
    "unknown".to_string()
}

/// The class of expensive action an abuse quota governs.
///
/// Each class has its own quota so a burst of cheap wallet reads cannot starve
/// market creation, and vice versa.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionClass {
    /// Creating a new prediction market (expensive: validation + indexer write).
    MarketCreation,
    /// Wallet-triggering actions such as placing a prediction or claiming.
    WalletAction,
}

impl ActionClass {
    /// Stable label used in Redis keys and rejection metadata.
    pub fn label(self) -> &'static str {
        match self {
            ActionClass::MarketCreation => "market_creation",
            ActionClass::WalletAction => "wallet_action",
        }
    }
}

/// A quota policy for one action class.
///
/// Quotas are expressed per-wallet and per-network so that a single wallet
/// cannot exhaust capacity for everyone, and so that testnet/mainnet usage is
/// accounted for independently. IP addresses are deliberately *not* part of the
/// identity: shared NAT / proxy egress must not couple unrelated wallets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuotaPolicy {
    /// The action class this policy governs.
    pub action: ActionClass,
    /// Maximum number of actions allowed per wallet within `window`.
    pub per_wallet_limit: u32,
    /// Maximum number of actions allowed across the whole network within
    /// `window`. This bounds aggregate resource use even under account cycling.
    pub per_network_limit: u32,
    /// The fixed window over which the limits apply.
    pub window: Duration,
    /// Minimum validation cost (in abstract units) that must be paid before
    /// the expensive work runs. Requests below this are rejected up front.
    pub min_validation_cost: u32,
}

impl QuotaPolicy {
    /// Default market-creation policy: 5 per wallet / 200 per network per hour,
    /// requiring a validation cost of at least 1.
    pub fn market_creation() -> Self {
        QuotaPolicy {
            action: ActionClass::MarketCreation,
            per_wallet_limit: 5,
            per_network_limit: 200,
            window: Duration::from_secs(60 * 60),
            min_validation_cost: 1,
        }
    }

    /// Default wallet-action policy: 60 per wallet / 2000 per network per hour,
    /// requiring a validation cost of at least 1.
    pub fn wallet_action() -> Self {
        QuotaPolicy {
            action: ActionClass::WalletAction,
            per_wallet_limit: 60,
            per_network_limit: 2000,
            window: Duration::from_secs(60 * 60),
            min_validation_cost: 1,
        }
    }
}

/// Why an abuse check rejected a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbuseReason {
    /// The wallet exhausted its own quota for this action class.
    WalletQuotaExceeded,
    /// The network-wide quota for this action class was exhausted.
    NetworkQuotaExceeded,
    /// The request did not meet the minimum validation cost.
    InsufficientValidationCost,
}

impl AbuseReason {
    /// Stable machine-readable reason code for the rejection body.
    pub fn code(self) -> &'static str {
        match self {
            AbuseReason::WalletQuotaExceeded => "wallet_quota_exceeded",
            AbuseReason::NetworkQuotaExceeded => "network_quota_exceeded",
            AbuseReason::InsufficientValidationCost => "insufficient_validation_cost",
        }
    }
}

/// A structured, transparent rejection returned by the abuse controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbuseRejection {
    /// Why the request was rejected.
    pub reason: AbuseReason,
    /// Seconds the caller should wait before retrying.
    pub retry_after_secs: u64,
    /// The applicable per-wallet limit for the action class.
    pub per_wallet_limit: u32,
    /// The applicable per-network limit for the action class.
    pub per_network_limit: u32,
    /// The minimum validation cost required for the action class.
    pub min_validation_cost: u32,
}

impl AbuseRejection {
    /// Render the rejection as a transparent JSON body.
    pub fn to_json(&self) -> String {
        format!(
            "{{\"error\":\"rate_limited\",\"reason\":\"{}\",\"retry_after\":{},\
             \"per_wallet_limit\":{},\"per_network_limit\":{},\
             \"min_validation_cost\":{}}}",
            self.reason.code(),
            self.retry_after_secs,
            self.per_wallet_limit,
            self.per_network_limit,
            self.min_validation_cost,
        )
    }
}

/// A single fixed-window counter.
#[derive(Clone, Copy, Debug)]
struct WindowCounter {
    count: u32,
    window_start: Instant,
}

impl WindowCounter {
    fn new(now: Instant) -> Self {
        WindowCounter {
            count: 0,
            window_start: now,
        }
    }

    /// Reset the counter if the window has elapsed, returning the remaining
    /// seconds until the current window ends.
    fn refresh(&mut self, now: Instant, window: Duration) -> u64 {
        let elapsed = now.saturating_duration_since(self.window_start);
        if elapsed >= window {
            self.count = 0;
            self.window_start = now;
            window.as_secs()
        } else {
            (window - elapsed).as_secs().max(1)
        }
    }
}

/// Per-wallet + per-network abuse controller.
///
/// Identity is `(wallet, network)`; the client IP is never used as identity, so
/// many legitimate wallets behind a shared NAT/proxy are independent, while a
/// single wallet cycling IPs cannot escape its quota.
pub struct AbuseController {
    policies: HashMap<ActionClass, QuotaPolicy>,
    wallets: Mutex<HashMap<(String, String, ActionClass), WindowCounter>>,
    networks: Mutex<HashMap<(String, ActionClass), WindowCounter>>,
}

impl AbuseController {
    /// Build a controller with the default market-creation and wallet-action
    /// policies.
    pub fn with_defaults() -> Self {
        let mut policies = HashMap::new();
        let mc = QuotaPolicy::market_creation();
        let wa = QuotaPolicy::wallet_action();
        policies.insert(mc.action, mc);
        policies.insert(wa.action, wa);
        Self::new(policies)
    }

    /// Build a controller from an explicit set of policies.
    pub fn new(policies: HashMap<ActionClass, QuotaPolicy>) -> Self {
        AbuseController {
            policies,
            wallets: Mutex::new(HashMap::new()),
            networks: Mutex::new(HashMap::new()),
        }
    }

    /// The policy for `action`, if configured.
    pub fn policy(&self, action: ActionClass) -> Option<QuotaPolicy> {
        self.policies.get(&action).copied()
    }

    /// Check a market-creation request for `wallet` on `network`.
    ///
    /// `validation_cost` is the cost the caller is willing to pay for
    /// validation; it must meet the policy minimum before any expensive work.
    pub fn check_market_creation(
        &self,
        wallet: &str,
        network: &str,
        validation_cost: u32,
    ) -> Result<(), AbuseRejection> {
        self.check(ActionClass::MarketCreation, wallet, network, validation_cost)
    }

    /// Check a wallet-triggering action for `wallet` on `network`.
    pub fn check_wallet_action(
        &self,
        wallet: &str,
        network: &str,
        validation_cost: u32,
    ) -> Result<(), AbuseRejection> {
        self.check(ActionClass::WalletAction, wallet, network, validation_cost)
    }

    fn check(
        &self,
        action: ActionClass,
        wallet: &str,
        network: &str,
        validation_cost: u32,
    ) -> Result<(), AbuseRejection> {
        let policy = match self.policies.get(&action) {
            Some(policy) => *policy,
            // No policy configured means no abuse control for this class.
            None => return Ok(()),
        };

        if validation_cost < policy.min_validation_cost {
            return Err(AbuseRejection {
                reason: AbuseReason::InsufficientValidationCost,
                retry_after_secs: policy.window.as_secs(),
                per_wallet_limit: policy.per_wallet_limit,
                per_network_limit: policy.per_network_limit,
                min_validation_cost: policy.min_validation_cost,
            });
        }

        let wallet_key = (
            normalize_identity(wallet),
            normalize_identity(network),
            action,
        );
        let network_key = (normalize_identity(network), action);
        let now = Instant::now();

        // Check the network quota first so a single wallet cannot consume the
        // last network slot before we know the network is exhausted.
        {
            let mut networks = self.networks.lock().expect("abuse networks poisoned");
            let counter = networks
                .entry(network_key.clone())
                .or_insert_with(|| WindowCounter::new(now));
            let reset = counter.refresh(now, policy.window);
            if counter.count >= policy.per_network_limit {
                return Err(AbuseRejection {
                    reason: AbuseReason::NetworkQuotaExceeded,
                    retry_after_secs: reset,
                    per_wallet_limit: policy.per_wallet_limit,
                    per_network_limit: policy.per_network_limit,
                    min_validation_cost: policy.min_validation_cost,
                });
            }
        }

        let mut wallets = self.wallets.lock().expect("abuse wallets poisoned");
        let counter = wallets
            .entry(wallet_key)
            .or_insert_with(|| WindowCounter::new(now));
        let reset = counter.refresh(now, policy.window);
        if counter.count >= policy.per_wallet_limit {
            return Err(AbuseRejection {
                reason: AbuseReason::WalletQuotaExceeded,
                retry_after_secs: reset,
                per_wallet_limit: policy.per_wallet_limit,
                per_network_limit: policy.per_network_limit,
                min_validation_cost: policy.min_validation_cost,
            });
        }

        // Admit the request: consume one slot from both counters.
        counter.count += 1;
        drop(wallets);
        let mut networks = self.networks.lock().expect("abuse networks poisoned");
        if let Some(network_counter) = networks.get_mut(&network_key) {
            network_counter.count += 1;
        }
        Ok(())
    }
}

/// Normalize a wallet/network identifier for use as a map key.
fn normalize_identity(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[derive(Clone)]
struct DistributedRateLimitState {
    tier: RateLimitTier,
    redis: RedisCache,
}

fn insert_rate_limit_headers(
    response: &mut Response,
    limit: u32,
    remaining: u32,
    reset_after_secs: u64,
) {
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&limit.to_string()) {
        headers.insert("x-ratelimit-limit", value);
    }
    if let Ok(value) = HeaderValue::from_str(&remaining.to_string()) {
        headers.insert("x-ratelimit-remaining", value);
    }
    if let Ok(value) = HeaderValue::from_str(&reset_after_secs.to_string()) {
        headers.insert("x-ratelimit-reset", value);
    }
}

fn rate_limited_response(limit: u32, remaining: u32, reset_after_secs: u64) -> Response {
    let mut response = crate::response::rate_limit_error_response();
    insert_rate_limit_headers(&mut response, limit, remaining, reset_after_secs);
    if let Ok(value) = HeaderValue::from_str(&reset_after_secs.to_string()) {
        response.headers_mut().insert("retry-after", value);
    }
    response
}

fn limiter_unavailable_response() -> Response {
    let mut response = crate::response::rate_limiter_unavailable_response();
    response
        .headers_mut()
        .insert("retry-after", HeaderValue::from_static("1"));
    response
}

async fn distributed_rate_limit_middleware(
    State(state): State<DistributedRateLimitState>,
    request: Request,
    next: Next,
) -> Response {
    let (limit, period_secs) = state.tier.burst_and_period();
    let identity = request_identity(state.tier, request.headers(), request.extensions());
    let key = rate_limit_key(state.tier, &identity);

    match state.redis.rate_limit_consume(&key, period_secs).await {
        Some(decision) if decision.count > limit => {
            let remaining = 0;
            rate_limited_response(limit, remaining, decision.reset_after_secs)
        }
        Some(decision) => {
            let mut response = next.run(request).await;
            let remaining = limit.saturating_sub(decision.count);
            insert_rate_limit_headers(&mut response, limit, remaining, decision.reset_after_secs);
            response
        }
        None => match state.tier.failure_policy() {
            RateLimitFailurePolicy::FailOpen => next.run(request).await,
            RateLimitFailurePolicy::FailClosed => limiter_unavailable_response(),
        },
    }
}

/// Wrap `router` with a distributed, Redis-backed limiter for `tier`.
///
/// In `#[cfg(test)]` builds this is a no-op so parallel unit tests do not share
/// counters. Integration tests exercise the Redis methods directly (see
/// `redis_integration_tests.rs`).
#[cfg(not(test))]
pub fn with_distributed_rate_limit(
    router: axum::Router,
    tier: RateLimitTier,
    redis: RedisCache,
) -> axum::Router {
    use axum::middleware::from_fn_with_state;

    router.layer(from_fn_with_state(
        DistributedRateLimitState { tier, redis },
        distributed_rate_limit_middleware,
    ))
}

#[cfg(test)]
pub fn with_distributed_rate_limit(
    router: axum::Router,
    _tier: RateLimitTier,
    _redis: RedisCache,
) -> axum::Router {
    router
}

/// Legacy per-instance limiter (kept for callers that do not have Redis).
///
/// New code should prefer [`with_distributed_rate_limit`]. In `#[cfg(test)]`
/// builds this is a no-op, matching the previous behaviour.
#[cfg(not(test))]
pub fn with_rate_limit(router: axum::Router, tier: RateLimitTier) -> axum::Router {
    use std::sync::Arc;
    use tower_governor::governor::GovernorConfigBuilder;

    let (burst_size, period_secs) = tier.burst_and_period();

    let config = Arc::new(
        GovernorConfigBuilder::default()
            .period(std::time::Duration::from_secs(period_secs))
            .burst_size(burst_size)
            .key_extractor(SmartIpKeyExtractor)
            .error_handler(|_| crate::response::rate_limit_error_response())
            .finish()
            .expect("invalid governor config"),
    );

    router.layer(tower_governor::GovernorLayer { config })
}

#[cfg(test)]
pub fn with_rate_limit(router: axum::Router, _tier: RateLimitTier) -> axum::Router {
    router
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, StatusCode};

    #[test]
    fn tier_burst_and_period_values() {
        use crate::constants::*;

        let (b, p) = RateLimitTier::Light.burst_and_period();
        assert_eq!(
            (b, p),
            (RATE_LIMIT_LIGHT_BURST, RATE_LIMIT_LIGHT_PERIOD_SECS)
        );

        let (b, p) = RateLimitTier::Read.burst_and_period();
        assert_eq!((b, p), (RATE_LIMIT_READ_BURST, RATE_LIMIT_READ_PERIOD_SECS));

        let (b, p) = RateLimitTier::User.burst_and_period();
        assert_eq!((b, p), (RATE_LIMIT_USER_BURST, RATE_LIMIT_USER_PERIOD_SECS));

        let (b, p) = RateLimitTier::Write.burst_and_period();
        assert_eq!(
            (b, p),
            (RATE_LIMIT_WRITE_BURST, RATE_LIMIT_WRITE_PERIOD_SECS)
        );

        let (b, p) = RateLimitTier::Token.burst_and_period();
        assert_eq!(
            (b, p),
            (RATE_LIMIT_TOKEN_BURST, RATE_LIMIT_TOKEN_PERIOD_SECS)
        );

        let (b, p) = RateLimitTier::WebSocket.burst_and_period();
        assert_eq!((b, p), (RATE_LIMIT_WS_BURST, RATE_LIMIT_WS_PERIOD_SECS));
    }

    #[test]
    fn tiers_have_distinct_configs() {
        let configs = [
            RateLimitTier::Light.burst_and_period(),
            RateLimitTier::Read.burst_and_period(),
            RateLimitTier::User.burst_and_period(),
            RateLimitTier::Write.burst_and_period(),
            RateLimitTier::Token.burst_and_period(),
            RateLimitTier::WebSocket.burst_and_period(),
        ];
        for i in 0..configs.len() {
            for j in (i + 1)..configs.len() {
                assert_ne!(
                    configs[i], configs[j],
                    "tiers {i} and {j} share the same config"
                );
            }
        }
    }

    #[test]
    fn outage_policies_separate_reads_writes_and_auth() {
        assert_eq!(
            RateLimitTier::Read.failure_policy(),
            RateLimitFailurePolicy::FailOpen
        );
        assert_eq!(
            RateLimitTier::Light.failure_policy(),
            RateLimitFailurePolicy::FailOpen
        );
        assert_eq!(
            RateLimitTier::User.failure_policy(),
            RateLimitFailurePolicy::FailOpen
        );
        assert_eq!(
            RateLimitTier::Write.failure_policy(),
            RateLimitFailurePolicy::FailClosed
        );
        assert_eq!(
            RateLimitTier::Token.failure_policy(),
            RateLimitFailurePolicy::FailClosed
        );
    }

    #[test]
    fn keys_are_stable_and_sanitized() {
        let key = rate_limit_key(RateLimitTier::Read, " 203.0.113.9 ");
        assert_eq!(key, "rate_limit:read:203.0.113.9");
        let key = rate_limit_key(RateLimitTier::User, "GABC:DE*F?");
        assert_eq!(key, "rate_limit:user:GABCDEF");
        assert_eq!(
            rate_limit_key(RateLimitTier::Token, "   "),
            "rate_limit:token:unknown"
        );
    }

    #[test]
    fn user_tier_prefers_the_authenticated_address() {
        let mut headers = HeaderMap::new();
        headers.insert("x-user-address", HeaderValue::from_static("GABC"));
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.9"));
        let extensions = Extensions::new();

        assert_eq!(
            request_identity(RateLimitTier::User, &headers, &extensions),
            "GABC"
        );
        assert_eq!(
            request_identity(RateLimitTier::Read, &headers, &extensions),
            "203.0.113.9"
        );
    }

    #[test]
    fn forwarded_header_uses_first_hop() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.7, 10.0.0.2"),
        );
        assert_eq!(
            request_identity(RateLimitTier::Read, &headers, &Extensions::new()),
            "198.51.100.7"
        );
    }

    #[test]
    fn rate_limited_response_carries_standard_headers() {
        let response = rate_limited_response(60, 0, 42);
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response
                .headers()
                .get("x-ratelimit-limit")
                .and_then(|v| v.to_str().ok()),
            Some("60")
        );
        assert_eq!(
            response
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok()),
            Some("0")
        );
        assert_eq!(
            response
                .headers()
                .get("x-ratelimit-reset")
                .and_then(|v| v.to_str().ok()),
            Some("42")
        );
        assert_eq!(
            response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok()),
            Some("42")
        );
    }

    #[test]
    fn limiter_unavailable_is_a_503_with_retry_after() {
        let response = limiter_unavailable_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok()),
            Some("1")
        );
    }

    fn controller_with_limits(
        per_wallet: u32,
        per_network: u32,
        min_cost: u32,
    ) -> AbuseController {
        let mut policies = HashMap::new();
        policies.insert(
            ActionClass::MarketCreation,
            QuotaPolicy {
                action: ActionClass::MarketCreation,
                per_wallet_limit: per_wallet,
                per_network_limit: per_network,
                window: Duration::from_secs(3600),
                min_validation_cost: min_cost,
            },
        );
        policies.insert(
            ActionClass::WalletAction,
            QuotaPolicy {
                action: ActionClass::WalletAction,
                per_wallet_limit: per_wallet,
                per_network_limit: per_network,
                window: Duration::from_secs(3600),
                min_validation_cost: min_cost,
            },
        );
        AbuseController::new(policies)
    }

    #[test]
    fn market_creation_quota_exhaustion_rejects_with_metadata() {
        let controller = controller_with_limits(2, 100, 1);
        assert!(controller
            .check_market_creation("GABC", "testnet", 1)
            .is_ok());
        assert!(controller
            .check_market_creation("GABC", "testnet", 1)
            .is_ok());

        let rejection = controller
            .check_market_creation("GABC", "testnet", 1)
            .expect_err("third creation should be rejected");
        assert_eq!(rejection.reason, AbuseReason::WalletQuotaExceeded);
        assert_eq!(rejection.per_wallet_limit, 2);
        assert_eq!(rejection.per_network_limit, 100);
        assert_eq!(rejection.min_validation_cost, 1);
        assert!(rejection.retry_after_secs > 0);
        let json = rejection.to_json();
        assert!(json.contains("\"reason\":\"wallet_quota_exceeded\""));
        assert!(json.contains("\"per_wallet_limit\":2"));
    }

    #[test]
    fn insufficient_validation_cost_is_rejected_before_work() {
        let controller = controller_with_limits(10, 100, 5);
        let rejection = controller
            .check_wallet_action("GABC", "testnet", 1)
            .expect_err("cost below minimum should be rejected");
        assert_eq!(rejection.reason, AbuseReason::InsufficientValidationCost);
        assert_eq!(rejection.min_validation_cost, 5);
        // The rejected request must not have consumed quota.
        assert!(controller
            .check_wallet_action("GABC", "testnet", 5)
            .is_ok());
    }

    #[test]
    fn network_quota_bounds_account_cycling() {
        let controller = controller_with_limits(10, 3, 1);
        assert!(controller
            .check_market_creation("GAAA", "testnet", 1)
            .is_ok());
        assert!(controller
            .check_market_creation("GBBB", "testnet", 1)
            .is_ok());
        assert!(controller
            .check_market_creation("GCCC", "testnet", 1)
            .is_ok());
        // A brand-new wallet is still bounded by the network quota.
        let rejection = controller
            .check_market_creation("GDDD", "testnet", 1)
            .expect_err("network quota should reject a fresh wallet");
        assert_eq!(rejection.reason, AbuseReason::NetworkQuotaExceeded);
    }

    #[test]
    fn multi_wallet_users_operate_within_limits() {
        let controller = controller_with_limits(3, 100, 1);
        // A legitimate user with several wallets, each under its own quota.
        for wallet in ["GAAA", "GBBB", "GCCC"] {
            for _ in 0..3 {
                assert!(
                    controller
                        .check_market_creation(wallet, "testnet", 1)
                        .is_ok(),
                    "wallet {wallet} should stay within its quota"
                );
            }
        }
    }

    #[test]
    fn shared_ip_does_not_couple_wallets() {
        // The controller has no IP input at all; two wallets on the same
        // network are independent up to the network quota.
        let controller = controller_with_limits(2, 100, 1);
        assert!(controller
            .check_wallet_action("GAAA", "testnet", 1)
            .is_ok());
        assert!(controller
            .check_wallet_action("GAAA", "testnet", 1)
            .is_ok());
        assert!(controller
            .check_wallet_action("GAAA", "testnet", 1)
            .is_err());
        // A different wallet behind the same IP still has full quota.
        assert!(controller
            .check_wallet_action("GBBB", "testnet", 1)
            .is_ok());
        assert!(controller
            .check_wallet_action("GBBB", "testnet", 1)
            .is_ok());
    }

    #[test]
    fn networks_are_accounted_independently() {
        let controller = controller_with_limits(1, 100, 1);
        assert!(controller
            .check_market_creation("GAAA", "testnet", 1)
            .is_ok());
        assert!(controller
            .check_market_creation("GAAA", "testnet", 1)
            .is_err());
        // The same wallet on mainnet has its own quota.
        assert!(controller
            .check_market_creation("GAAA", "mainnet", 1)
            .is_ok());
    }

    #[test]
    fn default_policies_are_configured() {
        let controller = AbuseController::with_defaults();
        assert!(controller.policy(ActionClass::MarketCreation).is_some());
        assert!(controller.policy(ActionClass::WalletAction).is_some());
        assert!(controller
            .check_market_creation("GAAA", "testnet", 1)
            .is_ok());
    }
}
