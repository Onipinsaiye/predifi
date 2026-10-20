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

use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header::HeaderValue, Extensions, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
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
    let mut response = (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "status": "error",
            "error": "Rate limiter unavailable",
        })),
    )
        .into_response();
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
    use axum::http::HeaderValue;

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
}
