//! W3C Trace Context propagation across HTTP, queue, and outbound boundaries.
//!
//! This module threads a single `trace_id` through every stage of a user
//! operation so operators can follow it end to end:
//!
//! ```text
//! HTTP request (traceparent header)
//!       │
//!       ├── LoggingLayer extracts (or starts) the trace context
//!       │
//!       ├── queued Job carries `trace_context` in its payload
//!       │         └── worker restores the context before running the handler
//!       │
//!       └── outbound requests (incl. Soroban RPC) get `traceparent` injected
//! ```
//!
//! # Standard
//!
//! The wire format is the [W3C Trace Context](https://www.w3.org/TR/trace-context/)
//! `traceparent` header: `00-<32 hex trace id>-<16 hex span id>-<2 hex flags>`,
//! with an optional `tracestate` companion header. The same structure is
//! serialized into queue payloads (`Job::trace_context`) so an in-flight job
//! can be joined back to the HTTP request that produced it.
//!
//! # Sampling
//!
//! Sampling is controlled by the `TRACE_SAMPLE_RATE` environment variable
//! (default `1.0`, range `0.0..=1.0`).
//!
//! The policy is:
//!
//! 1. If a valid incoming `traceparent` is present, its sampled flag is
//!    honoured verbatim - an upstream decision is never overridden.
//! 2. Otherwise a new trace is started and sampled with probability
//!    `TRACE_SAMPLE_RATE`.
//! 3. Context is propagated regardless of the sampled flag so unsampled
//!    requests still keep log correlation; only export is gated by sampling.
//!
//! When telemetry is enabled the OpenTelemetry provider is configured with a
//! matching `ParentBased(TraceIdRatioBased)` sampler (see [`crate::telemetry`]).
//!
//! # Redaction
//!
//! Trace attributes must never carry wallet secrets or unbounded identifiers.
//! [`redact_attributes`] recursively sanitizes attribute maps:
//!
//! * Keys matching sensitive patterns (`secret`, `mnemonic`, `password`,
//!   `authorization`, `token`, `seed`, ...) become `[REDACTED]`.
//! * Keys identifying a wallet/user (`user_address`, `creator`, `wallet`, ...)
//!   keep only an 8-character prefix plus `...`.
//! * Every other string value is bounded to [`MAX_ATTRIBUTE_VALUE_LEN`]
//!   characters so identifiers cannot grow without limit.
//!
//! [`payload_preview`] applies the same rules to opaque queue payloads before
//! they are attached to spans, so payload contents are observable without
//! leaking secrets.

use std::future::Future;
use std::str::FromStr;

use http::{HeaderMap, HeaderValue};
use opentelemetry::trace::{
    SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
};
use opentelemetry::Context;
use serde::{Deserialize, Serialize};
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;
use uuid::Uuid;

// ── Constants ─────────────────────────────────────────────────────────────────

/// W3C header carrying the trace context.
pub const TRACEPARENT_HEADER: &str = "traceparent";
/// W3C header carrying vendor-specific trace state.
pub const TRACESTATE_HEADER: &str = "tracestate";
/// Environment variable configuring the head sampling rate.
pub const SAMPLE_RATE_ENV: &str = "TRACE_SAMPLE_RATE";
/// Default sampling rate when `TRACE_SAMPLE_RATE` is unset or invalid.
pub const DEFAULT_SAMPLE_RATE: f64 = 1.0;
/// Replacement text used for redacted attribute values.
pub const REDACTED: &str = "[REDACTED]";
/// Maximum retained length for a single string attribute value.
pub const MAX_ATTRIBUTE_VALUE_LEN: usize = 64;
/// Maximum retained length for the redacted payload preview on a span.
pub const MAX_PAYLOAD_PREVIEW_LEN: usize = 512;

/// Attribute keys containing any of these fragments are fully redacted.
const SENSITIVE_KEY_PATTERNS: &[&str] = &[
    "secret",
    "password",
    "passphrase",
    "mnemonic",
    "seed",
    "private_key",
    "privatekey",
    "signing_key",
    "authorization",
    "jwt",
    "api_key",
    "apikey",
    "token",
    "bearer",
    "credential",
];

/// Attribute keys containing any of these fragments identify a wallet or
/// account and are reduced to a short prefix.
const IDENTIFIER_KEY_PATTERNS: &[&str] = &[
    "user_address",
    "user",
    "creator",
    "wallet",
    "address",
    "account",
    "referrer",
    "referred_user",
    "destination",
];

const ZERO_TRACE_ID: &str = "00000000000000000000000000000000";
const ZERO_SPAN_ID: &str = "0000000000000000";

// ── TraceContext ──────────────────────────────────────────────────────────────

/// A W3C Trace Context (trace id, parent span id, sampling decision).
///
/// Serializable so it can be embedded in queue payloads and restored by the
/// worker without losing the correlation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceContext {
    /// 32 lowercase hex characters.
    pub trace_id: String,
    /// 16 lowercase hex characters.
    pub span_id: String,
    /// The W3C sampled flag (`01`/`00`).
    pub sampled: bool,
    /// Optional vendor `tracestate` value, propagated verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracestate: Option<String>,
}

impl TraceContext {
    /// Create a fresh context with random ids and the given sampling decision.
    pub fn new(sampled: bool) -> Self {
        let trace_id = Uuid::new_v4().simple().to_string();
        let span_raw = Uuid::new_v4().simple().to_string();
        Self {
            trace_id,
            span_id: span_raw[..16].to_string(),
            sampled,
            tracestate: None,
        }
    }

    /// Parse a W3C `traceparent` header value.
    ///
    /// Returns `None` for malformed values: wrong field lengths, non-lowercase
    /// hex, the forbidden `ff` version, or all-zero trace/span ids.
    pub fn parse_traceparent(value: &str) -> Option<Self> {
        let parts: Vec<&str> = value.trim().split('-').collect();
        if parts.len() < 4 {
            return None;
        }
        let (version, trace_id, span_id, flags) = (parts[0], parts[1], parts[2], parts[3]);
        if version == "ff" {
            return None;
        }
        let is_lower_hex = |s: &str| {
            s.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        };
        if version.len() != 2
            || !is_lower_hex(version)
            || trace_id.len() != 32
            || !is_lower_hex(trace_id)
            || span_id.len() != 16
            || !is_lower_hex(span_id)
            || flags.len() != 2
            || !is_lower_hex(flags)
        {
            return None;
        }
        if trace_id == ZERO_TRACE_ID || span_id == ZERO_SPAN_ID {
            return None;
        }
        let flag_byte = u8::from_str_radix(flags, 16).ok()?;
        Some(Self {
            trace_id: trace_id.to_string(),
            span_id: span_id.to_string(),
            sampled: flag_byte & 0x01 == 0x01,
            tracestate: None,
        })
    }

    /// Serialize to the W3C `traceparent` format.
    pub fn to_traceparent(&self) -> String {
        let flags = if self.sampled { "01" } else { "00" };
        format!("00-{}-{}-{}", self.trace_id, self.span_id, flags)
    }

    /// Extract a context from HTTP headers (`traceparent` + `tracestate`).
    pub fn extract(headers: &HeaderMap) -> Option<Self> {
        let raw = headers.get(TRACEPARENT_HEADER)?.to_str().ok()?;
        let mut ctx = Self::parse_traceparent(raw)?;
        ctx.tracestate = headers
            .get(TRACESTATE_HEADER)
            .and_then(|v| v.to_str().ok())
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        Some(ctx)
    }

    /// Inject this context into an HTTP header map.
    pub fn inject(&self, headers: &mut HeaderMap) {
        if let Ok(value) = HeaderValue::from_str(&self.to_traceparent()) {
            headers.insert(TRACEPARENT_HEADER, value);
        }
        if let Some(state) = &self.tracestate {
            if let Ok(value) = HeaderValue::from_str(state) {
                headers.insert(TRACESTATE_HEADER, value);
            }
        }
    }

    /// Convenience: build a header map containing this context.
    pub fn to_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        self.inject(&mut headers);
        headers
    }

    /// Build the OpenTelemetry [`Context`] representing this context as a
    /// remote parent, for use with `OpenTelemetrySpanExt::set_parent`.
    pub fn otel_context(&self) -> Context {
        let trace_id = TraceId::from_hex(&self.trace_id).unwrap_or(TraceId::INVALID);
        let span_id = SpanId::from_hex(&self.span_id).unwrap_or(SpanId::INVALID);
        let flags = if self.sampled {
            TraceFlags::SAMPLED
        } else {
            TraceFlags::NOT_SAMPLED
        };
        let trace_state = self
            .tracestate
            .as_deref()
            .and_then(|ts| TraceState::from_str(ts).ok())
            .unwrap_or_default();
        let span_context = SpanContext::new(trace_id, span_id, flags, true, trace_state);
        Context::new().with_remote_span_context(span_context)
    }

    /// Make `span` a child of this trace context in the OpenTelemetry pipeline.
    ///
    /// This is a no-op when the OpenTelemetry layer is not installed
    /// (`TELEMETRY_ENABLED` is not `true`).
    pub fn set_as_parent(&self, span: &Span) {
        span.set_parent(self.otel_context());
    }
}

// ── Sampling ──────────────────────────────────────────────────────────────────

/// Parse a sample rate string, clamping to `0.0..=1.0` and falling back to
/// [`DEFAULT_SAMPLE_RATE`] for invalid input.
pub fn parse_sample_rate(raw: &str) -> f64 {
    raw.trim()
        .parse::<f64>()
        .map(|rate| rate.clamp(0.0, 1.0))
        .unwrap_or(DEFAULT_SAMPLE_RATE)
}

/// The configured head sampling rate (`TRACE_SAMPLE_RATE`, default `1.0`).
pub fn sample_rate() -> f64 {
    std::env::var(SAMPLE_RATE_ENV)
        .map(|raw| parse_sample_rate(&raw))
        .unwrap_or(DEFAULT_SAMPLE_RATE)
}

/// Uniform pseudo-random value in `[0, 1)` derived from a UUID.
fn random_unit() -> f64 {
    let value = Uuid::new_v4().as_u128();
    (value as f64) / (u128::MAX as f64)
}

/// Start a new root context, sampling it per [`sample_rate`].
pub fn start_root() -> TraceContext {
    TraceContext::new(random_unit() < sample_rate())
}

/// Continue the incoming trace when a valid `traceparent` header is present,
/// otherwise start a new (possibly unsampled) root context.
///
/// An incoming sampled flag is always honoured - the upstream decision wins.
pub fn from_headers_or_new(headers: &HeaderMap) -> TraceContext {
    if let Some(ctx) = TraceContext::extract(headers) {
        return ctx;
    }
    start_root()
}

// ── Ambient (task-local) context ──────────────────────────────────────────────

tokio::task_local! {
    /// Trace context of the request or job currently being processed.
    static CURRENT_TRACE: TraceContext;
}

/// The trace context scoped to the current Tokio task, if any.
pub fn current() -> Option<TraceContext> {
    CURRENT_TRACE.try_with(Clone::clone).ok()
}

/// The current context serialized as a `traceparent` value.
pub fn current_traceparent() -> Option<String> {
    current().map(|ctx| ctx.to_traceparent())
}

/// Run `future` with `ctx` as the ambient trace context.
pub async fn scope<F>(ctx: TraceContext, future: F) -> F::Output
where
    F: Future,
{
    CURRENT_TRACE.scope(ctx, future).await
}

/// Run `future` with `ctx` as the ambient trace context when present,
/// otherwise unchanged.
pub async fn scope_optional<F>(ctx: Option<TraceContext>, future: F) -> F::Output
where
    F: Future,
{
    match ctx {
        Some(ctx) => CURRENT_TRACE.scope(ctx, future).await,
        None => future.await,
    }
}

// ── Outbound injection ────────────────────────────────────────────────────────

/// Headers to attach to an outbound request so the receiving service can join
/// this trace.
///
/// Resolution order:
///
/// 1. The active OpenTelemetry span, when the telemetry layer is installed
///    (uses the span's real id, so the remote parent reference resolves).
/// 2. The task-local context from an HTTP request or queued job.
/// 3. A freshly started root context, so outbound requests - including
///    Soroban RPC calls - always carry a valid `traceparent`.
pub fn outbound_headers() -> HeaderMap {
    let otel_span_context = Span::current().context().span().span_context().clone();
    if otel_span_context.is_valid() {
        return headers_from_span_context(&otel_span_context);
    }
    match current() {
        Some(ctx) => ctx.to_headers(),
        None => start_root().to_headers(),
    }
}

fn headers_from_span_context(span_context: &SpanContext) -> HeaderMap {
    let flags = if span_context.trace_flags().is_sampled() {
        "01"
    } else {
        "00"
    };
    let traceparent = format!(
        "00-{}-{}-{}",
        span_context.trace_id(),
        span_context.span_id(),
        flags
    );
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&traceparent) {
        headers.insert(TRACEPARENT_HEADER, value);
    }
    let state = span_context.trace_state().header();
    if !state.is_empty() {
        if let Ok(value) = HeaderValue::from_str(&state) {
            headers.insert(TRACESTATE_HEADER, value);
        }
    }
    headers
}

// ── Redaction ─────────────────────────────────────────────────────────────────

/// Whether an attribute key names a secret that must never be logged.
pub fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    SENSITIVE_KEY_PATTERNS.iter().any(|p| key.contains(p))
}

/// Whether an attribute key identifies a wallet/account whose value should be
/// shortened to a prefix.
pub fn is_identifier_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    IDENTIFIER_KEY_PATTERNS.iter().any(|p| key.contains(p))
}

/// Truncate `value` to `max` characters, appending `...` when shortened.
pub fn truncate_bounded(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_owned();
    }
    let head: String = value.chars().take(max).collect();
    format!("{head}...")
}

/// Bound a single value to [`MAX_ATTRIBUTE_VALUE_LEN`] characters.
pub fn bound_value(value: &str) -> String {
    truncate_bounded(value, MAX_ATTRIBUTE_VALUE_LEN)
}

/// Shorten a wallet address to an 8-character prefix plus `...`.
pub fn redact_wallet(address: &str) -> String {
    crate::request_logger::redact_address(address)
}

/// Recursively sanitize a JSON attribute map for tracing:
///
/// * sensitive keys -> `[REDACTED]`
/// * wallet/user identifier keys -> 8-character prefix
/// * all other strings -> bounded to [`MAX_ATTRIBUTE_VALUE_LEN`] characters
pub fn redact_attributes(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, inner)| {
                    let redacted = if is_sensitive_key(key) {
                        Value::String(REDACTED.to_owned())
                    } else if is_identifier_key(key) {
                        match inner {
                            Value::String(s) => Value::String(redact_wallet(s)),
                            other => redact_attributes(other),
                        }
                    } else {
                        redact_attributes(inner)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_attributes).collect()),
        Value::String(s) => Value::String(bound_value(s)),
        other => other.clone(),
    }
}

/// A redacted, bounded preview of an opaque queue payload, safe to attach to
/// a span or log line.
///
/// JSON payloads are redacted key-by-key; non-JSON payloads are simply bounded
/// (they are never logged verbatim beyond [`MAX_ATTRIBUTE_VALUE_LEN`]).
pub fn payload_preview(payload: &str) -> String {
    let preview = match serde_json::from_str::<serde_json::Value>(payload) {
        Ok(value) => redact_attributes(&value).to_string(),
        Err(_) => bound_value(payload),
    };
    truncate_bounded(&preview, MAX_PAYLOAD_PREVIEW_LEN)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    #[test]
    fn parses_valid_traceparent() {
        let ctx = TraceContext::parse_traceparent(VALID_TRACEPARENT).expect("valid");
        assert_eq!(ctx.trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(ctx.span_id, "00f067aa0ba902b7");
        assert!(ctx.sampled);
        assert_eq!(ctx.to_traceparent(), VALID_TRACEPARENT);
    }

    #[test]
    fn parses_unsampled_traceparent() {
        let ctx = TraceContext::parse_traceparent(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00",
        )
        .expect("valid");
        assert!(!ctx.sampled);
    }

    #[test]
    fn rejects_malformed_traceparent() {
        // Too few fields.
        assert!(TraceContext::parse_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7").is_none());
        // Wrong trace id length.
        assert!(TraceContext::parse_traceparent("00-abc-00f067aa0ba902b7-01").is_none());
        // Uppercase hex is not canonical.
        assert!(TraceContext::parse_traceparent(
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01"
        )
        .is_none());
        // Forbidden version.
        assert!(TraceContext::parse_traceparent(
            "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
        )
        .is_none());
        // All-zero ids are invalid.
        assert!(TraceContext::parse_traceparent(
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01"
        )
        .is_none());
        assert!(TraceContext::parse_traceparent(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01"
        )
        .is_none());
    }

    #[test]
    fn extract_and_inject_round_trip_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(TRACEPARENT_HEADER, VALID_TRACEPARENT.parse().unwrap());
        headers.insert(TRACESTATE_HEADER, "vendor=value".parse().unwrap());

        let ctx = TraceContext::extract(&headers).expect("extractable");
        assert_eq!(ctx.tracestate.as_deref(), Some("vendor=value"));

        let mut out = HeaderMap::new();
        ctx.inject(&mut out);
        assert_eq!(out.get(TRACEPARENT_HEADER).unwrap(), VALID_TRACEPARENT);
        assert_eq!(out.get(TRACESTATE_HEADER).unwrap(), "vendor=value");
    }

    #[test]
    fn extract_returns_none_without_header() {
        assert!(TraceContext::extract(&HeaderMap::new()).is_none());
    }

    #[test]
    fn incoming_sampled_flag_is_honoured() {
        // Upstream says "do not sample" - the decision must be preserved even
        // though the default rate is 1.0.
        let mut headers = HeaderMap::new();
        headers.insert(
            TRACEPARENT_HEADER,
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00"
                .parse()
                .unwrap(),
        );
        let ctx = from_headers_or_new(&headers);
        assert!(!ctx.sampled, "upstream unsampled decision must be honoured");
        assert_eq!(ctx.trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
    }

    #[test]
    fn incoming_sampled_true_is_honoured() {
        let mut headers = HeaderMap::new();
        headers.insert(TRACEPARENT_HEADER, VALID_TRACEPARENT.parse().unwrap());
        let ctx = from_headers_or_new(&headers);
        assert!(ctx.sampled);
    }

    #[test]
    fn parse_sample_rate_clamps_and_defaults() {
        assert_eq!(parse_sample_rate("0.5"), 0.5);
        assert_eq!(parse_sample_rate(" 2.0 "), 1.0, "clamped to upper bound");
        assert_eq!(parse_sample_rate("-1"), 0.0, "clamped to lower bound");
        assert_eq!(parse_sample_rate("nonsense"), DEFAULT_SAMPLE_RATE);
    }

    #[test]
    fn sample_rate_env_is_configurable() {
        std::env::set_var(SAMPLE_RATE_ENV, "0.25");
        assert_eq!(sample_rate(), 0.25);
        std::env::remove_var(SAMPLE_RATE_ENV);
        assert_eq!(sample_rate(), DEFAULT_SAMPLE_RATE);
    }

    #[test]
    fn zero_sample_rate_marks_new_roots_unsampled() {
        std::env::set_var(SAMPLE_RATE_ENV, "0.0");
        let ctx = start_root();
        assert!(!ctx.sampled, "rate 0.0 must not sample new traces");
        std::env::remove_var(SAMPLE_RATE_ENV);
    }

    #[test]
    fn full_sample_rate_marks_new_roots_sampled() {
        std::env::set_var(SAMPLE_RATE_ENV, "1.0");
        let ctx = start_root();
        assert!(ctx.sampled, "rate 1.0 must sample new traces");
        std::env::remove_var(SAMPLE_RATE_ENV);
    }

    #[tokio::test]
    async fn scope_exposes_and_hides_current_context() {
        assert!(current().is_none(), "no ambient context outside scope");

        let ctx = TraceContext::new(true);
        let captured = scope(ctx.clone(), async {
            current().expect("scoped context visible")
        })
        .await;
        assert_eq!(captured, ctx);
        assert!(current().is_none(), "context must not leak past scope");
    }

    #[tokio::test]
    async fn scope_optional_passes_through_when_absent() {
        let value = scope_optional(None, async { current_traceparent() }).await;
        assert_eq!(value, None);
    }

    #[test]
    fn otel_context_preserves_trace_id_and_sampling() {
        let ctx = TraceContext::new(true);
        let otel = ctx.otel_context();
        let span_context = otel.span().span_context().clone();
        assert!(span_context.is_valid());
        assert_eq!(span_context.trace_id().to_string(), ctx.trace_id);
        assert!(span_context.trace_flags().is_sampled());
        assert!(span_context.is_remote());
    }

    #[test]
    fn outbound_headers_fall_back_to_fresh_root() {
        // No ambient scope and no OTel layer in this test -> a fresh root.
        let headers = outbound_headers();
        let ctx = TraceContext::extract(&headers).expect("outbound headers carry traceparent");
        assert!(!ctx.trace_id.is_empty());
    }

    #[tokio::test]
    async fn outbound_headers_prefer_ambient_context() {
        let ctx = TraceContext::new(true);
        let headers = scope(ctx.clone(), async { outbound_headers() }).await;
        let injected = TraceContext::extract(&headers).expect("traceparent injected");
        assert_eq!(injected.trace_id, ctx.trace_id);
        assert_eq!(injected.span_id, ctx.span_id);
        assert!(injected.sampled);
    }

    // ── Redaction ─────────────────────────────────────────────────────────────

    #[test]
    fn sensitive_keys_are_fully_redacted() {
        for key in [
            "wallet_secret",
            "mnemonic",
            "password",
            "authorization",
            "api_key",
            "seed_phrase",
            "jwt",
            "access_token",
        ] {
            assert!(is_sensitive_key(key), "{key} must be sensitive");
        }
        assert!(!is_sensitive_key("pool_id"));
        assert!(!is_sensitive_key("user_address"));
    }

    #[test]
    fn redact_attributes_removes_secrets_and_bounds_identifiers() {
        let long_id = "unbounded-event-identifier-".repeat(20); // 540 chars
        let wallet = "GABCDEFGHIJKLMNOPQRSTUVWXYZ234567ABCDEFGHIJKLMNOPQRSTUVW"; // 56 chars
        let value = serde_json::json!({
            "wallet_secret": "correct horse battery staple",
            "mnemonic": ["alpha", "beta"],
            "authorization": "Bearer eyJhbGciOi.payload.sig",
            "user_address": wallet,
            "event_id": long_id,
            "amount": 42,
            "nested": { "api_key": "sk-live-123", "label": "ok" },
        });

        let redacted = redact_attributes(&value);
        let text = redacted.to_string();

        assert!(!text.contains("correct horse battery staple"));
        assert!(!text.contains("alpha"), "nested secrets must be redacted");
        assert!(!text.contains("eyJhbGciOi"));
        assert!(!text.contains("sk-live-123"));
        assert!(!text.contains(&long_id), "unbounded id must be bounded");
        assert!(!text.contains(wallet), "full wallet address must not appear");

        assert_eq!(redacted["wallet_secret"], REDACTED);
        assert_eq!(redacted["authorization"], REDACTED);
        assert_eq!(redacted["nested"]["api_key"], REDACTED);
        assert_eq!(redacted["amount"], 42, "non-sensitive values survive");

        let preview = redacted["event_id"].as_str().expect("string");
        assert!(
            preview.chars().count() <= MAX_ATTRIBUTE_VALUE_LEN + 3,
            "bounded identifier too long: {}",
            preview.chars().count()
        );

        let addr = redacted["user_address"].as_str().expect("string");
        let expected_prefix: String = wallet.chars().take(8).collect();
        assert!(
            addr.starts_with(&expected_prefix),
            "prefix kept: {addr} (expected {expected_prefix}...)"
        );
        assert!(addr.ends_with("..."));
        assert!(addr.len() < wallet.len());
    }

    #[test]
    fn payload_preview_redacts_json_payloads() {
        let payload = serde_json::json!({
            "pool_id": 7,
            "wallet_secret": "do-not-log-me",
            "user": "GBLAHBLAHBLAHBLAHBLAHBLAHBLAHBLAHBLAHBLAHBLAHBLAHBLAH",
        })
        .to_string();

        let preview = payload_preview(&payload);
        assert!(!preview.contains("do-not-log-me"));
        assert!(!preview.contains("GBLAHBLAHBLAHBLAHBLAHBLAHBLAHBLAHBLAHBLAHBLAHBLAHBLAH"));
        assert!(preview.contains("\"pool_id\":7"));
        assert!(preview.contains(REDACTED));
    }

    #[test]
    fn payload_preview_bounds_non_json_payloads() {
        let raw = "x".repeat(10_000);
        let preview = payload_preview(&raw);
        assert!(
            preview.chars().count() <= MAX_ATTRIBUTE_VALUE_LEN + 3,
            "raw payload preview must be bounded"
        );
    }

    #[test]
    fn truncate_bounded_is_exact_at_boundary() {
        assert_eq!(bound_value(&"a".repeat(64)), "a".repeat(64));
        assert_eq!(bound_value(&"a".repeat(65)), format!("{}...", "a".repeat(64)));
        // Character (not byte) counting for multi-byte input.
        assert_eq!(truncate_bounded("ééé", 2), "éé...");
    }
}
