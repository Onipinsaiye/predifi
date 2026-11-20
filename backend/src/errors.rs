//! Centralized application error type (#51).
//!
//! Lower-level errors (database, Soroban RPC, JWT) are mapped to [`AppError`]
//! variants, and every variant renders as the standard error envelope from
//! [`crate::response`]. Messages carried by variants must be safe for
//! clients: mappings below never copy driver, SQL, constraint or RPC error
//! text into a 4xx message, and 5xx messages are replaced with catalog
//! messages by the envelope builder.

use std::fmt;

use crate::response::FieldViolation;

/// Structured, client-safe details for an abuse-control rejection (#103).
///
/// Carries only what a legitimate caller needs to understand and recover from
/// the rejection: a stable machine-readable reason, how long to wait, and the
/// quota that was exceeded. It never contains internal counters, keys, IPs or
/// other server state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbuseRejection {
    /// Stable reason code, e.g. `market_creation_quota` or `wallet_action_quota`.
    pub reason: &'static str,
    /// Seconds the caller should wait before retrying (rendered as `Retry-After`).
    pub retry_after_secs: u64,
    /// Human-readable quota description, e.g. `10 per 3600s`.
    pub quota: String,
    /// Optional scope the quota applies to, e.g. `wallet` or `network`.
    pub scope: Option<String>,
}

impl AbuseRejection {
    /// Build a rejection with a reason, retry delay and quota description.
    pub fn new(reason: &'static str, retry_after_secs: u64, quota: impl Into<String>) -> Self {
        Self {
            reason,
            retry_after_secs,
            quota: quota.into(),
            scope: None,
        }
    }

    /// Attach the scope the quota applies to.
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }
}

/// Rejection for a privileged moderation action that lacks a fresh,
/// action-bound step-up proof (security issue: step-up verification).
///
/// Clients receive only a stable reason code and the required proof kind; the
/// proof freshness window and any internal state are never disclosed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepUpRequired {
    /// Stable reason code, e.g. `step_up_required`, `step_up_stale`,
    /// `step_up_replayed`, `step_up_action_mismatch`.
    pub reason: &'static str,
    /// Action the caller must re-prove for, e.g. `hide_content`.
    pub action: &'static str,
}

impl StepUpRequired {
    /// Build a step-up rejection for the given action and reason.
    pub fn new(reason: &'static str, action: &'static str) -> Self {
        Self { reason, action }
    }
}

/// Centralized application error type for mapping lower-level errors
/// (like database errors) to high-level variants used by the API.
#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    Conflict(String),
    InvalidInput(String),
    /// Validation failure with per-field details.
    Validation(Vec<FieldViolation>),
    Unauthorized(String),
    /// Token was revoked (logout, compromise, or user-wide revocation).
    TokenRevoked,
    Forbidden(String),
    /// Privileged action requires a fresh, action-bound step-up proof.
    StepUpRequired(StepUpRequired),
    RateLimited,
    /// Abuse-control rejection with transparent, client-safe quota details.
    AbuseRejected(AbuseRejection),
    DatabaseUnavailable,
    TokenStoreUnavailable,
    /// Upstream dependency (Soroban RPC) failed or its circuit is open.
    UpstreamUnavailable(String),
    /// Upstream dependency did not answer within its deadline.
    UpstreamTimeout(String),
    ServiceUnavailable(String),
    Internal(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::ServiceUnavailable(msg) => write!(f, "service unavailable: {}", msg),
            AppError::NotFound(msg) => write!(f, "not found: {}", msg),
            AppError::Conflict(msg) => write!(f, "conflict: {}", msg),
            AppError::InvalidInput(msg) => write!(f, "invalid input: {}", msg),
            AppError::Validation(v) => write!(f, "invalid input: {} field(s) rejected", v.len()),
            AppError::Unauthorized(msg) => write!(f, "unauthorized: {}", msg),
            AppError::TokenRevoked => write!(f, "unauthorized: token revoked"),
            AppError::Forbidden(msg) => write!(f, "forbidden: {}", msg),
            AppError::StepUpRequired(s) => write!(
                f,
                "step-up required: reason={} action={}",
                s.reason, s.action
            ),
            AppError::RateLimited => write!(f, "rate limited"),
            AppError::AbuseRejected(r) => write!(
                f,
                "abuse rejected: reason={} retry_after={}s quota={}",
                r.reason, r.retry_after_secs, r.quota
            ),
            AppError::DatabaseUnavailable => write!(f, "database unavailable"),
            AppError::TokenStoreUnavailable => write!(f, "token store unavailable"),
            AppError::UpstreamUnavailable(msg) => write!(f, "upstream unavailable: {}", msg),
            AppError::UpstreamTimeout(msg) => write!(f, "upstream timeout: {}", msg),
            AppError::Internal(msg) => write!(f, "internal error: {}", msg),
        }
    }
}

impl std::error::Error for AppError {}

impl AppError {
    /// HTTP status and stable error code for this error.
    pub fn status_and_code(&self) -> (axum::http::StatusCode, &'static str) {
        use crate::response::error_codes as c;
        use axum::http::StatusCode as S;
        match self {
            AppError::NotFound(_) => (S::NOT_FOUND, c::NOT_FOUND),
            AppError::Conflict(_) => (S::CONFLICT, c::CONFLICT),
            AppError::InvalidInput(_) | AppError::Validation(_) => {
                (S::BAD_REQUEST, c::INVALID_INPUT)
            }
            AppError::Unauthorized(_) => (S::UNAUTHORIZED, c::UNAUTHORIZED),
            AppError::TokenRevoked => (S::UNAUTHORIZED, c::TOKEN_REVOKED),
            AppError::Forbidden(_) => (S::FORBIDDEN, c::FORBIDDEN),
            AppError::StepUpRequired(_) => (S::FORBIDDEN, c::FORBIDDEN),
            AppError::RateLimited => (S::TOO_MANY_REQUESTS, c::RATE_LIMIT_EXCEEDED),
            AppError::AbuseRejected(_) => (S::TOO_MANY_REQUESTS, c::RATE_LIMIT_EXCEEDED),
            AppError::DatabaseUnavailable => (S::SERVICE_UNAVAILABLE, c::DATABASE_UNAVAILABLE),
            AppError::TokenStoreUnavailable => {
                (S::SERVICE_UNAVAILABLE, c::TOKEN_STORE_UNAVAILABLE)
            }
            AppError::UpstreamUnavailable(_) => (S::BAD_GATEWAY, c::UPSTREAM_UNAVAILABLE),
            AppError::UpstreamTimeout(_) => (S::GATEWAY_TIMEOUT, c::UPSTREAM_TIMEOUT),
            AppError::ServiceUnavailable(_) => (S::SERVICE_UNAVAILABLE, c::SERVICE_UNAVAILABLE),
            AppError::Internal(_) => (S::INTERNAL_SERVER_ERROR, c::INTERNAL_ERROR),
        }
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        // Constraint names, column names and SQL text stay in the logs: the
        // client only gets a generic message for the mapped code.
        match e {
            sqlx::Error::RowNotFound => AppError::NotFound("record not found".to_string()),
            sqlx::Error::PoolTimedOut
            | sqlx::Error::PoolClosed
            | sqlx::Error::Io(_)
            | sqlx::Error::Tls(_) => {
                tracing::warn!(error = %e, "database unavailable");
                AppError::DatabaseUnavailable
            }
            sqlx::Error::Database(db_err) => match db_err.code().as_deref() {
                // unique_violation
                Some("23505") => AppError::Conflict("resource already exists".to_string()),
                // foreign_key_violation
                Some("23503") => {
                    AppError::Conflict("referenced resource does not exist".to_string())
                }
                // not_null_violation
                Some("23502") => AppError::InvalidInput("a required field is missing".to_string()),
                // check_violation
                Some("23514") => AppError::InvalidInput("a field value is out of range".to_string()),
                _ => AppError::Internal(db_err.message().to_string()),
            },
            other => AppError::Internal(other.to_string()),
        }
    }
}

impl From<crate::jwt::JwtVerifyError> for AppError {
    fn from(e: crate::jwt::JwtVerifyError) -> Self {
        use crate::jwt::JwtVerifyError as J;
        match e {
            J::Revoked => AppError::TokenRevoked,
            J::TokenStoreUnavailable => AppError::TokenStoreUnavailable,
            // A bad server secret is a server fault, not a client one.
            J::InvalidSecret(inner) => AppError::Internal(inner.to_string()),
            J::Expired => AppError::Unauthorized("token has expired".to_string()),
            J::WrongTokenType => {
                AppError::Unauthorized("token type is not valid for this endpoint".to_string())
            }
            // Signature, format, decode and key-version failures share one
            // message so clients cannot probe token internals.
            _ => AppError::Unauthorized("invalid token".to_string()),
        }
    }
}

impl From<crate::rpc_resilience::RpcError> for AppError {
    fn from(e: crate::rpc_resilience::RpcError) -> Self {
        use crate::rpc_resilience::RpcError as R;
        match e {
            R::Timeout | R::DeadlineExceeded => AppError::UpstreamTimeout(e.to_string()),
            _ => AppError::UpstreamUnavailable(e.to_string()),
        }
    }
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        use crate::response::ApiResponse;

        let (status, code) = self.status_and_code();
        let mut response = match self {
            AppError::Validation(details) => {
                ApiResponse::<()>::validation_error("request validation failed", details)
                    .into_response()
            }
            AppError::NotFound(msg)
            | AppError::Conflict(msg)
            | AppError::InvalidInput(msg)
            | AppError::Unauthorized(msg)
            | AppError::Forbidden(msg)
            | AppError::UpstreamUnavailable(msg)
            | AppError::UpstreamTimeout(msg)
            | AppError::ServiceUnavailable(msg)
            | AppError::Internal(msg) => ApiResponse::<()>::error(status, code, msg).into_response(),
            AppError::StepUpRequired(step_up) => {
                let mut details = serde_json::Map::new();
                details.insert(
                    "reason".to_string(),
                    serde_json::Value::String(step_up.reason.to_string()),
                );
                details.insert(
                    "action".to_string(),
                    serde_json::Value::String(step_up.action.to_string()),
                );
                let resp = ApiResponse::<()>::error(
                    status,
                    code,
                    crate::response::safe_message(code),
                )
                .into_response();
                let (parts, body) = resp.into_parts();
                let mut value = match axum::body::to_bytes(body, usize::MAX).await {
                    Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
                        .unwrap_or_else(|_| serde_json::json!({})),
                    Err(_) => serde_json::json!({}),
                };
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("details".to_string(), serde_json::Value::Object(details));
                }
                let bytes = serde_json::to_vec(&value).unwrap_or_default();
                axum::response::Response::from_parts(parts, axum::body::Body::from(bytes))
            }
            AppError::AbuseRejected(rejection) => {
                let mut details = serde_json::Map::new();
                details.insert(
                    "reason".to_string(),
                    serde_json::Value::String(rejection.reason.to_string()),
                );
                details.insert(
                    "retry_after_secs".to_string(),
                    serde_json::Value::from(rejection.retry_after_secs),
                );
                details.insert(
                    "quota".to_string(),
                    serde_json::Value::String(rejection.quota.clone()),
                );
                if let Some(scope) = &rejection.scope {
                    details.insert(
                        "scope".to_string(),
                        serde_json::Value::String(scope.clone()),
                    );
                }
                let mut resp = ApiResponse::<()>::error(
                    status,
                    code,
                    crate::response::safe_message(code),
                )
                .into_response();
                // Attach the structured details alongside the standard envelope.
                let (mut parts, body) = resp.into_parts();
                let mut value = match axum::body::to_bytes(body, usize::MAX).await {
                    Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
                        .unwrap_or_else(|_| serde_json::json!({})),
                    Err(_) => serde_json::json!({}),
                };
                if let Some(obj) = value.as_object_mut() {
                    obj.insert(
                        "details".to_string(),
                        serde_json::Value::Object(details),
                    );
                }
                let bytes = serde_json::to_vec(&value).unwrap_or_default();
                parts.headers.insert(
                    axum::http::header::RETRY_AFTER,
                    axum::http::HeaderValue::from_str(&rejection.retry_after_secs.to_string())
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("1")),
                );
                resp = axum::response::Response::from_parts(
                    parts,
                    axum::body::Body::from(bytes),
                );
                resp
            }
            AppError::TokenRevoked
            | AppError::RateLimited
            | AppError::DatabaseUnavailable
            | AppError::TokenStoreUnavailable => {
                ApiResponse::<()>::error(status, code, crate::response::safe_message(code))
                    .into_response()
