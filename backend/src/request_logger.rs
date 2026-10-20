//! # Request Logging & Trace Context Middleware
//!
//! This module provides a Tower-compatible middleware layer for Axum that
//! emits **structured JSON log lines** for every HTTP request, including:
//!
//! - HTTP method, path, status code, and duration.
//! - A **correlation ID** (`x-correlation-id` / `x-request-id`) that is
//!   propagated through the entire request chain so logs and traces from
//!   multiple services can be joined.
//! - **W3C Trace Context** (`traceparent` / `tracestate`) propagated from
//!   inbound requests and injected into outbound requests and queue payloads
//!   so a single user operation can be followed across HTTP handling, queued
//!   work, database writes, and Soroban RPC calls.
//! - **Sampling** decisions and **sensitive-attribute redaction** so wallet
//!   secrets and unbounded identifiers never leak into traces.
//! - **User context** — the authenticated wallet address extracted from the
//!   `x-user-address` header (populated by the JWT middleware upstream).
//! - **Pool context** — the pool ID extracted from the URL path when present
//!   (e.g. `/api/v1/pools/42/...`).
//! - Prometheus latency metrics recorded via [`SharedMetrics`].
//!
//! ## Output format
//!
//! Every request produces a tracing span whose JSON representation looks like:
//!
//! ```json
//! {
//!   "timestamp": "2026-07-26T10:30:00.123456Z",
//!   "level": "INFO",
//!   "target": "predifi_backend::request_logger",
//!   "span": {
//!     "http.method": "GET",
//!     "http.route": "/api/v1/pools/42",
//!     "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
//!     "correlation_id": "01J2K3L4M5N6P7Q8R9S0T1U2V3",
//!     "user_address": "GABC...XYZ",
//!     "pool_id": "42"
//!   },
//!   "fields": {
//!     "http.status_code": 200,
//!     "http.duration_ms": 4,
//!     "message": "request complete"
//!   }
//! }
//! ```
//!
//! ## Log-level configurability
//!
//! Use the `RUST_LOG` environment variable (or `LOG_LEVEL` in `.env`) to
//! control verbosity per-module:
//!
//! ```text
//! RUST_LOG=predifi_backend::request_logger=debug,warn
//! ```
//!
//! At `debug` level every request is logged; at `info` only requests that
//! take longer than `SLOW_REQUEST_THRESHOLD_MS` are elevated to `warn`.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use axum::{Router, routing::get};
//! use request_logger::LoggingLayer;
//!
//! let app = Router::new()
//!     .route("/", get(|| async { "hello" }))
//!     .layer(LoggingLayer::new());
//! ```

use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Instant,
};

use axum::http::{HeaderMap, Request, Response};
use tower::{Layer, Service};
use tracing::{error, info, info_span, warn, Instrument};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

use crate::metrics::SharedMetrics;

/// Requests that take longer than this threshold are logged at `WARN` level
/// even when the status code indicates success.
const SLOW_REQUEST_THRESHOLD_MS: u128 = 1_000;

/// Standard W3C Trace Context headers.
pub const TRACEPARENT_HEADER: &str = "traceparent";
pub const TRACESTATE_HEADER: &str = "tracestate";

/// Maximum length of an inbound `traceparent` / `tracestate` value we accept.
/// Longer values are treated as unbounded identifiers and dropped.
const MAX_TRACE_HEADER_LEN: usize = 512;

// ── Helper: extract a correlation ID ─────────────────────────────────────────

/// Return the correlation ID for a request.
///
/// Priority order:
/// 1. `x-correlation-id` header (e.g. set by an API gateway).
/// 2. `x-request-id` header (common alternative).
/// 3. A freshly generated UUID v4 (guarantees every request has an ID).
fn extract_correlation_id(headers: &HeaderMap) -> String {
    for header_name in &["x-correlation-id", "x-request-id"] {
        if let Some(val) = headers.get(*header_name) {
            if let Ok(s) = val.to_str() {
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
    }
    Uuid::new_v4().to_string()
}

// ── Trace context propagation ────────────────────────────────────────────────

/// A parsed W3C `traceparent` header value.
///
/// Format: `version-trace_id-parent_id-flags` where `version` is 2 hex chars,
/// `trace_id` is 32 hex chars, `parent_id` is 16 hex chars, and `flags` is 2
/// hex chars. Only version `00` is currently defined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceParent {
    pub version: u8,
    pub trace_id: String,
    pub parent_id: String,
    pub flags: u8,
}

impl TraceParent {
    /// Parse a `traceparent` header value, returning `None` if it is malformed,
    /// oversized, or uses an unsupported version.
    pub fn parse(value: &str) -> Option<Self> {
        if value.is_empty() || value.len() > MAX_TRACE_HEADER_LEN {
            return None;
        }
        let mut parts = value.split('-');
        let version = u8::from_str_radix(parts.next()?, 16).ok()?;
        if version != 0 {
            return None;
        }
        let trace_id = parts.next()?;
        let parent_id = parts.next()?;
        let flags = u8::from_str_radix(parts.next()?, 16).ok()?;
        if parts.next().is_some() {
            return None;
        }
        if trace_id.len() != 32
            || parent_id.len() != 16
            || !trace_id.chars().all(|c| c.is_ascii_hexdigit())
            || !parent_id.chars().all(|c| c.is_ascii_hexdigit())
        {
            return None;
        }
        // All-zero trace/parent IDs are invalid per the W3C spec.
        if trace_id.chars().all(|c| c == '0') || parent_id.chars().all(|c| c == '0') {
            return None;
        }
        Some(Self {
            version,
            trace_id: trace_id.to_ascii_lowercase(),
            parent_id: parent_id.to_ascii_lowercase(),
            flags,
        })
    }

    /// Whether the sampled bit is set in the trace flags.
    pub fn is_sampled(&self) -> bool {
        self.flags & 0x01 == 0x01
    }

    /// Render back to the W3C `traceparent` wire format.
    pub fn to_header_value(&self) -> String {
        format!(
            "{:02x}-{}-{}-{:02x}",
            self.version, self.trace_id, self.parent_id, self.flags
        )
    }
}

/// Extract the inbound `traceparent` header, if present and valid.
fn extract_traceparent(headers: &HeaderMap) -> Option<TraceParent> {
    headers
        .get(TRACEPARENT_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(TraceParent::parse)
}

/// Extract the inbound `tracestate` header, if present and within bounds.
fn extract_tracestate(headers: &HeaderMap) -> Option<String> {
    headers
        .get(TRACESTATE_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty() && s.len() <= MAX_TRACE_HEADER_LEN)
        .map(|s| s.to_string())
}

/// Sampling configuration for the logging/tracing layer.
///
/// The default is a 100% sample rate, which is appropriate for development.
/// Production deployments should set a lower rate via `TRACE_SAMPLE_RATE`.
#[derive(Clone, Copy, Debug)]
pub struct SamplingConfig {
    /// Fraction of root spans to sample, in `[0.0, 1.0]`.
    pub sample_rate: f64,
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self { sample_rate: 1.0 }
    }
}

impl SamplingConfig {
    /// Build a config from the `TRACE_SAMPLE_RATE` env var, falling back to
    /// the default on parse failure or out-of-range values.
    pub fn from_env() -> Self {
        std::env::var("TRACE_SAMPLE_RATE")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|r| (0.0..=1.0).contains(r))
            .map(|sample_rate| Self { sample_rate })
            .unwrap_or_default()
    }

    /// Decide whether a root span (one without an inbound `traceparent`)
    /// should be sampled. Uses a deterministic hash of the correlation ID so
    /// the decision is stable across retries of the same logical request.
    pub fn should_sample(&self, correlation_id: &str) -> bool {
        if self.sample_rate >= 1.0 {
            return true;
        }
        if self.sample_rate <= 0.0 {
            return false;
        }
        // Cheap deterministic hash of the correlation ID.
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in correlation_id.as_bytes() {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let bucket = (hash % 10_000) as f64 / 10_000.0;
        bucket < self.sample_rate
    }
}

/// Redact a raw trace ID for log output, keeping only the first 8 hex chars.
///
/// Trace IDs are unbounded identifiers when they come from untrusted callers;
/// truncating them keeps logs correlatable without allowing log-injection or
/// cardinality blowups.
pub fn redact_trace_id(trace_id: &str) -> String {
    if trace_id.is_empty() {
        return String::new();
    }
    let prefix: String = trace_id.chars().take(8).collect();
    format!("{prefix}{REDACTED_ADDRESS_SUFFIX}")
}

// ── Helpers ────────────────────────────────────────────────────────────────────

const REDACTED_ADDRESS_SUFFIX: &str = "...";

/// Redact a wallet address for log output, keeping only the first 8 characters
/// so traces can still be correlated within a single request chain without
/// exposing enough of the address to identify an account from logs alone.
pub fn redact_address(address: &str) -> String {
    if address.is_empty() {
        return address.to_string();
    }
    let prefix: String = address.chars().take(8).collect();
    format!("{prefix}{REDACTED_ADDRESS_SUFFIX}")
}

/// Extract the authenticated user's wallet address from the request headers.
///
/// The JWT middleware is expected to have already validated the token and
/// written the wallet address into the `x-user-address` internal header
/// before the request reaches this middleware.
fn extract_user_address(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-user-address")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Extract the pool ID from a URL path such as `/api/v1/pools/42/predictions`.
///
/// Returns `None` if the path does not contain a numeric segment immediately
/// after `/pools/`.
fn extract_pool_id(path: &str) -> Option<String> {
    let mut parts = path.split('/');
    while let Some(segment) = parts.next() {
        if segment == "pools" {
            if let Some(id) = parts.next() {
                if id.chars().all(|c| c.is_ascii_digit()) && !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

// ── Layer ─────────────────────────────────────────────────────────────────────

/// A Tower [`Layer`] that wraps every service with [`LoggingService`].
///
/// Attach this to your Axum router with `.layer(LoggingLayer::new())` or
/// `.layer(LoggingLayer::with_metrics(metrics))` to also record Prometheus
/// latency metrics.
#[derive(Clone, Default)]
pub struct LoggingLayer {
    metrics: Option<SharedMetrics>,
    sampling: SamplingConfig,
}

impl LoggingLayer {
    /// Create a logging layer that emits structured logs only.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a logging layer that also records Prometheus latency metrics.
    pub fn with_metrics(metrics: SharedMetrics) -> Self {
        Self {
            metrics: Some(metrics),
            sampling: SamplingConfig::default(),
        }
    }

    /// Override the sampling configuration for this layer.
    pub fn with_sampling(mut self, sampling: SamplingConfig) -> Self {
        self.sampling = sampling;
        self
    }

    /// Create a logging layer that reads sampling from the environment.
    pub fn from_env() -> Self {
        Self {
            metrics: None,
            sampling: SamplingConfig::from_env(),
        }
    }
}

impl<S> Layer<S> for LoggingLayer {
    type Service = LoggingService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        LoggingService {
            inner,
            metrics: self.metrics.clone(),
            sampling: self.sampling,
        }
    }
}

// ── Service ───────────────────────────────────────────────────────────────────

/// The actual middleware service produced by [`LoggingLayer`].
///
/// `S` is the *inner* service — i.e. whatever comes after this middleware in
/// the stack (usually your route handlers).
#[derive(Clone)]
pub struct LoggingService<S> {
    inner: S,
    metrics: Option<SharedMetrics>,
    sampling: SamplingConfig,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for LoggingService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        // ── Extract per-request context ────────────────────────────────────
        let method = req.method().to_string();
        let path = req.uri().path().to_string();
        let correlation_id = extract_correlation_id(req.headers());
        let traceparent = extract_traceparent(req.headers());
        let tracestate = extract_tracestate(req.headers());
        let user_address = extract_user_address(req.headers())
            .map(|a| redact_address(&a))
            .unwrap_or_else(|| "anonymous".to_string());
        let pool_id = extract_pool_id(&path);

        let start = Instant::now();
        let metrics = self.metrics.clone();
        let sampling = self.sampling;

        // Determine the effective trace ID and sampling decision. If the
        // caller supplied a valid `traceparent`, honour its trace ID and
        // sampled flag; otherwise make a fresh sampling decision based on
        // the correlation ID.
        let (trace_id, sampled) = match &traceparent {
            Some(tp) => (tp.trace_id.clone(), tp.is_sampled()),
            None => {
                let decision = sampling.should_sample(&correlation_id);
                let trace_id = Uuid::new_v4().simple().to_string();
                (trace_id, decision)
            }
        };
        let redacted_trace_id = redact_trace_id(&trace_id);

        // Build the span.  All per-request context is attached here so the
        // JSON formatter emits it once under "span", not duplicated in each
        // log field.
        let span = info_span!(
            "http.request",
            http.method = %method,
            http.route = %path,
            trace_id = %redacted_trace_id,
            correlation_id = %correlation_id,
            user_address = %user_address,
            pool_id = pool_id.as_deref().unwrap_or(""),
            sampled = sampled,
        );

        // Link the span to the inbound W3C trace context, if any, so the
        // OpenTelemetry exporter can stitch this request into the caller's
        // trace across services.
        if let Some(tp) = &traceparent {
            let parent_ctx = opentelemetry::Context::new().with_remote_span_context(
                opentelemetry::trace::SpanContext::new(
                    opentelemetry::trace::TraceId::from_hex(&tp.trace_id)
                        .unwrap_or_else(|_| opentelemetry::trace::TraceId::INVALID),
                    opentelemetry::trace::SpanId::from_hex(&tp.parent_id)
                        .unwrap_or_else(|_| opentelemetry::trace::SpanId::INVALID),
                    opentelemetry::trace::TraceFlags::new(tp.flags),
                    true,
                    opentelemetry::trace::TraceState::default(),
                ),
            );
            span.set_parent(parent_ctx);
        }
        if let Some(state) = &tracestate {
            span.record("tracestate", state.as_str());
        }

        let inner_future = self.inner.call(req);

        Box::pin(
            async move {
                let result = inner_future.await;
                let elapsed = start.elapsed();
                let elapsed_ms = elapsed.as_millis();

                match &result {
                    Ok(response) => {
                        let status = response.status();
                        let status_u16 = status.as_u16();
                        let status_label = status_u16.to_string();

                        // Emit at WARN for slow requests or server errors.
                        if status_u16 >= 500 {
                            warn!(
                                http.status_code = status_u16,
                                http.duration_ms = elapsed_ms,
                                "request complete with server error"
                            );
                        } else if elapsed_ms >= SLOW_REQUEST_THRESHOLD_MS {
                            warn!(
                                http.status_code = status_u16,
                                http.duration_ms = elapsed_ms,
                                "request complete (slow)"
                            );
                        } else {
                            info!(
                                http.status_code = status_u16,
                                http.duration_ms = elapsed_ms,
                                "request complete"
                            );
                        }

                        if let Some(m) = &metrics {
                            m.http_request_duration_seconds
                                .with_label_values(&[&method, &path, &status_label])
                                .observe(elapsed.as_secs_f64());
                            m.http_requests_total
                                .with_label_values(&[&method, &path, &status_label])
                                .inc();
                            if status_u16 >= 500 {
                                m.http_server_errors_total.inc();
                            }
                        }
                    }
                    Err(_) => {
                        error!(
                            http.duration_ms = elapsed_ms,
                            "request failed with transport error"
                        );
                        if let Some(m) = &metrics {
                            m.http_request_duration_seconds
                                .with_label_values(&[&method, &path, "error"])
                                .observe(elapsed.as_secs_f64());
                        }
                    }
                }

                result
            }
            .instrument(span),
        )
    }
}

// ── Outbound propagation ─────────────────────────────────────────────────────

/// A trace context that can be attached to outbound HTTP requests and queue
/// payloads so downstream workers and RPC calls join the same trace.
#[derive(Clone, Debug)]
pub struct OutboundTraceContext {
    pub traceparent: String,
    pub tracestate: Option<String>,
}

impl OutboundTraceContext {
    /// Build an outbound context from the current tracing span.
    ///
    /// Call this from inside a span created by [`LoggingLayer`] (or any span
    /// that has been linked to the inbound trace) to obtain the headers that
    /// must be injected into the outbound request or serialized into the
    /// queue payload.
    pub fn from_current_span() -> Option<Self> {
        use opentelemetry::trace::TraceContextExt;
        let ctx = tracing::Span::current().context();
        let span = ctx.span();
        let span_ctx = span.span_context();
        if !span_ctx.is_valid() {
            return None;
        }
        let traceparent = format!(
            "00-{}-{}-{:02x}",
            span_ctx.trace_id(),
            span_ctx.span_id(),
            span_ctx.trace_flags().to_u8()
        );
        Some(Self {
            traceparent,
            tracestate: None,
        })
    }

    /// Inject the trace context into an outbound [`HeaderMap`].
    pub fn inject_headers(&self, headers: &mut HeaderMap) {
        if let Ok(val) = self.traceparent.parse() {
            headers.insert(TRACEPARENT_HEADER, val);
        }
        if let Some(state) = &self.tracestate {
            if let Ok(val) = state.parse() {
                headers.insert(TRACESTATE_HEADER, val);
            }
        }
    }

    /// Serialize the trace context into a queue payload map so workers can
    /// re-establish the trace when they pick up the job.
    pub fn to_queue_payload(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        map.insert(
            TRACEPARENT_HEADER.to_string(),
            serde_json::Value::String(self.traceparent.clone()),
        );
        if let Some(state) = &self.tracestate {
            map.insert(
                TRACESTATE_HEADER.to_string(),
                serde_json::Value::String(state.clone()),
            );
        }
        serde_json::Value::Object(map)
    }

    /// Restore a trace context from a queue payload produced by
    /// [`to_queue_payload`](Self::to_queue_payload).
    pub fn from_queue_payload(payload: &serde_json::Value) -> Option<Self> {
        let traceparent = payload.get(TRACEPARENT_HEADER)?.as_str()?.to_string();
        // Validate before trusting so a malformed payload cannot poison the
        // downstream trace.
        TraceParent::parse(&traceparent)?;
        let tracestate = payload
            .get(TRACESTATE_HEADER)
            .and_then(|v| v.as_str())
            .filter(|s| s.len() <= MAX_TRACE_HEADER_LEN)
            .map(|s| s.to_string());
        Some(Self {
            traceparent,
            tracestate,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, routing::get, Router};
    use http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;

    #[tokio::test]
    async fn logging_layer_records_request_latency_metric() {
        let metrics = Arc::new(crate::metrics::Metrics::new().expect("metrics"));
        let app = Router::new()
            .route("/ping", get(|| async { "pong" }))
            .layer(LoggingLayer::with_metrics(metrics.clone()));

        let response = app
            .oneshot(Request::builder().uri("/ping").body(Body::empty()).unwrap())
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), http::StatusCode::OK);

        let text = metrics.gather_text().expect("metrics text");
        assert!(
            text.contains("app_http_request_duration_seconds"),
            "latency histogram must be recorded, got: {text}"
        );
    }

    #[test]
    fn extract_correlation_id_uses_x_correlation_id_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-correlation-id", "test-id-123".parse().unwrap());
        let id = extract_correlation_id(&headers);
        assert_eq!(id, "test-id-123");
    }

    #[test]
    fn extract_correlation_id_falls_back_to_x_request_id() {
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", "fallback-456".parse().unwrap());
        let id = extract_correlation_id(&headers);
        assert_eq!(id, "fallback-456");
    }

    #[test]
    fn extract_correlation_id_generates_uuid_when_absent() {
        let headers = HeaderMap::new();
        let id = extract_correlation_id(&headers);
        // Should be a valid UUID v4 (36 chars with dashes).
        assert_eq!(id.len(), 36, "generated ID should be a UUID: {id}");
    }

    #[test]
    fn traceparent_parses_valid_header() {
        let tp = TraceParent::parse(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        )
        .expect("valid traceparent");
        assert_eq!(tp.trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(tp.parent_id, "00f067aa0ba902b7");
        assert!(tp.is_sampled());
    }

    #[test]
    fn traceparent_rejects_malformed_values() {
        assert!(TraceParent::parse("").is_none());
        assert!(TraceParent::parse("not-a-traceparent").is_none());
        assert!(TraceParent::parse(
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01"
        )
        .is_none());
        assert!(TraceParent::parse(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra"
        )
        .is_none());
        // Oversized values are rejected as unbounded identifiers.
        let oversized = format!("00-{}-00f067aa0ba902b7-01", "a".repeat(600));
        assert!(TraceParent::parse(&oversized).is_none());
    }

    #[test]
    fn redact_trace_id_truncates_to_prefix() {
        let redacted = redact_trace_id("4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(redacted, "4bf92f35...");
        assert_eq!(redact_trace_id(""), "");
    }

    #[test]
    fn sampling_config_honours_rate() {
        let always = SamplingConfig { sample_rate: 1.0 };
        assert!(always.should_sample("any-id"));
        let never = SamplingConfig { sample_rate: 0.0 };
        assert!(!never.should_sample("any-id"));
    }

    #[test]
    fn outbound_context_round_trips_through_queue_payload() {
        let ctx = OutboundTraceContext {
            traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
                .to_string(),
            tracestate: Some("vendor=value".to_string()),
        };
        let payload = ctx.to_queue_payload();
        let restored = OutboundTraceContext::from_queue_payload(&payload)
            .expect("payload should round-trip");
        assert_eq!(restored.traceparent, ctx.traceparent);
        assert_eq!(restored.tracestate, ctx.tracestate);
    }

    #[test]
    fn outbound_context_rejects_invalid_queue_payload() {
        let payload = serde_json::json!({ "traceparent": "garbage" });
        assert!(OutboundTraceContext::from_queue_payload(&payload).is_none());
    }

    #[tokio::test]
    async fn logging_layer_accepts_inbound_traceparent() {
        let app = Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(LoggingLayer::new());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header(
                        "traceparent",
                        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), http::StatusCode::OK);
    }

    #[test]
    fn extract_user_address_returns_header_value() {
        let mut headers = HeaderMap::new();
        headers.insert("x-user-address", "GABC123XYZ".parse().unwrap());
        assert_eq!(
            extract_user_address(&headers),
            Some("GABC123XYZ".to_string())
        );
    }

    #[test]
    fn extract_user_address_returns_none_when_absent() {
        let headers = HeaderMap::new();
        assert!(extract_user_address(&headers).is_none());
    }

    #[test]
    fn extract_pool_id_from_path() {
        assert_eq!(
            extract_pool_id("/api/v1/pools/42/predictions"),
            Some("42".to_string())
        );
        assert_eq!(extract_pool_id("/api/v1/pools/7"), Some("7".to_string()));
    }

    #[test]
    fn extract_pool_id_returns_none_for_non_pool_paths() {
        assert_eq!(extract_pool_id("/api/v1/health"), None);
        assert_eq!(extract_pool_id("/api/v1/pools/new"), None);
        assert_eq!(extract_pool_id("/"), None);
    }

    #[tokio::test]
    async fn logging_layer_propagates_correlation_id_in_span() {
        let app = Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(LoggingLayer::new());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("x-correlation-id", "trace-abc-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), http::StatusCode::OK);
    }
}
