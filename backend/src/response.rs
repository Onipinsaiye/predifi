//! # API Response Wrapper
//!
//! Provides one JSON envelope for every API response (#51).
//!
//! ```json
//! // success
//! { "status": "success", "data": <T> }
//!
//! // error (envelope version 1)
//! {
//!   "status": "error",
//!   "error": {
//!     "version": 1,
//!     "code": "INVALID_INPUT",
//!     "message": "<safe, human-readable message>",
//!     "request_id": "<id echoed in the x-request-id response header>",
//!     "retryable": false,
//!     "details": [ { "field": "limit", "reason": "must be between 1 and 100" } ]
//!   }
//! }
//! ```
//!
//! Rules every error follows:
//!
//! * `code` is a stable, machine-readable value from [`error_codes`]. Clients
//!   branch on it, never on `message`.
//! * `message` is safe to show to users. For any 5xx response the caller's
//!   message is replaced with the catalog message for the code, and the
//!   original text is only logged together with `request_id`. Database, RPC
//!   and internal error strings therefore never reach clients.
//! * `request_id` matches the `x-request-id` response header and the server
//!   logs. It comes from the inbound `x-request-id` / `x-correlation-id`
//!   header when present, otherwise a UUID is generated.
//! * `retryable` tells clients whether the same request may succeed later
//!   without changes (see [`is_retryable`]).
//! * `details` is only present for validation failures.
//!
//! [`error_envelope_layer`] wraps the router so failures produced outside
//! handlers (path/query/JSON rejections, unknown routes, wrong methods, body
//! size limits) use the same envelope.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use crate::response::ApiResponse;
//!
//! ApiResponse::success(my_data)
//! ApiResponse::<()>::error(StatusCode::NOT_FOUND, "NOT_FOUND", "Resource not found")
//! ```

use axum::{
    extract::Request,
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use uuid::Uuid;

/// Version of the error envelope shape. Bump only for breaking changes and
/// follow the API deprecation policy (`docs/API_VERSIONING_POLICY.md`).
pub const ERROR_ENVELOPE_VERSION: u32 = 1;

/// Response header carrying the request identifier.
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Longest inbound request id we echo back. Longer values are replaced.
const MAX_REQUEST_ID_LEN: usize = 128;

/// Largest non-JSON error body the envelope layer reads when rewriting.
const MAX_REWRITE_BODY_BYTES: usize = 16 * 1024;

/// Longest client-facing rejection text kept for 4xx rewrites.
const MAX_REJECTION_MESSAGE_LEN: usize = 256;

tokio::task_local! {
    /// Request id of the request currently being handled, set by
    /// [`error_envelope_layer`].
    static CURRENT_REQUEST_ID: String;
}

/// One invalid field in a validation failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldViolation {
    /// Field name, query parameter or JSON pointer that failed validation.
    pub field: String,
    /// Why the value was rejected.
    pub reason: String,
}

impl FieldViolation {
    pub fn new(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            reason: reason.into(),
        }
    }
}

/// Standardized error detail structure.
#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    /// Envelope version ([`ERROR_ENVELOPE_VERSION`]).
    pub version: u32,
    /// Machine-readable error code for programmatic handling.
    pub code: &'static str,
    /// Safe, human-readable error message.
    pub message: String,
    /// Request identifier for tracing and support.
    pub request_id: String,
    /// Whether retrying the same request later may succeed.
    pub retryable: bool,
    /// Field-level validation failures. Present only for validation errors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Vec<FieldViolation>>,
}

/// Generic JSON envelope returned by every API handler.
#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    /// `"success"` or `"error"`.
    pub status: &'static str,
    /// Present on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// Present on error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDetail>,
}

/// Stable error codes. Adding a code is backward compatible; renaming or
/// removing one is a breaking change.
pub mod error_codes {
    pub const NOT_FOUND: &str = "NOT_FOUND";
    pub const CONFLICT: &str = "CONFLICT";
    pub const INVALID_INPUT: &str = "INVALID_INPUT";
    pub const METHOD_NOT_ALLOWED: &str = "METHOD_NOT_ALLOWED";
    pub const PAYLOAD_TOO_LARGE: &str = "PAYLOAD_TOO_LARGE";
    pub const UNSUPPORTED_MEDIA_TYPE: &str = "UNSUPPORTED_MEDIA_TYPE";
    pub const SERVICE_UNAVAILABLE: &str = "SERVICE_UNAVAILABLE";
    pub const INTERNAL_ERROR: &str = "INTERNAL_ERROR";
    pub const DATABASE_UNAVAILABLE: &str = "DATABASE_UNAVAILABLE";
    pub const UPSTREAM_UNAVAILABLE: &str = "UPSTREAM_UNAVAILABLE";
    pub const UPSTREAM_TIMEOUT: &str = "UPSTREAM_TIMEOUT";
    pub const TOKEN_STORE_UNAVAILABLE: &str = "TOKEN_STORE_UNAVAILABLE";
    pub const RATE_LIMIT_EXCEEDED: &str = "RATE_LIMIT_EXCEEDED";
    pub const RATE_LIMITER_UNAVAILABLE: &str = "RATE_LIMITER_UNAVAILABLE";
    pub const UNAUTHORIZED: &str = "UNAUTHORIZED";
    pub const TOKEN_REVOKED: &str = "TOKEN_REVOKED";
    pub const FORBIDDEN: &str = "FORBIDDEN";
    pub const API_VERSION_SUNSET: &str = "API_VERSION_SUNSET";
    pub const NOTIFICATION_DEAD: &str = "NOTIFICATION_DEAD";
    pub const NOTIFICATION_NOT_FOUND: &str = "NOTIFICATION_NOT_FOUND";
}

/// Whether a request that failed with `code` may succeed if retried unchanged.
pub fn is_retryable(code: &str) -> bool {
    use error_codes::*;
    matches!(
        code,
        SERVICE_UNAVAILABLE
            | DATABASE_UNAVAILABLE
            | UPSTREAM_UNAVAILABLE
            | UPSTREAM_TIMEOUT
            | TOKEN_STORE_UNAVAILABLE
            | RATE_LIMIT_EXCEEDED
            | RATE_LIMITER_UNAVAILABLE
    )
}

/// Client-safe message used for 5xx responses (and for rewritten bodies).
pub fn safe_message(code: &str) -> &'static str {
    use error_codes::*;
    match code {
        DATABASE_UNAVAILABLE => "database not available",
        UPSTREAM_UNAVAILABLE => "upstream service unavailable",
        UPSTREAM_TIMEOUT => "upstream service timed out",
        TOKEN_STORE_UNAVAILABLE => "token store unavailable",
        RATE_LIMITER_UNAVAILABLE => "rate limiter unavailable",
        SERVICE_UNAVAILABLE => "service temporarily unavailable",
        NOT_FOUND => "resource not found",
        METHOD_NOT_ALLOWED => "method not allowed",
        PAYLOAD_TOO_LARGE => "request body too large",
        UNSUPPORTED_MEDIA_TYPE => "unsupported media type",
        INVALID_INPUT => "invalid request",
        UNAUTHORIZED => "authentication required",
        TOKEN_REVOKED => "token has been revoked",
        FORBIDDEN => "forbidden",
        CONFLICT => "conflict",
        API_VERSION_SUNSET => "this API version or route has been retired",
        RATE_LIMIT_EXCEEDED => "Too many requests",
        _ => "internal server error",
    }
}

/// Default error code for a bare HTTP status.
pub fn code_for_status(status: StatusCode) -> &'static str {
    use error_codes::*;
    match status {
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => INVALID_INPUT,
        StatusCode::UNAUTHORIZED => UNAUTHORIZED,
        StatusCode::FORBIDDEN => FORBIDDEN,
        StatusCode::NOT_FOUND => NOT_FOUND,
        StatusCode::METHOD_NOT_ALLOWED => METHOD_NOT_ALLOWED,
        StatusCode::CONFLICT => CONFLICT,
        StatusCode::GONE => API_VERSION_SUNSET,
        StatusCode::PAYLOAD_TOO_LARGE => PAYLOAD_TOO_LARGE,
        StatusCode::UNSUPPORTED_MEDIA_TYPE => UNSUPPORTED_MEDIA_TYPE,
        StatusCode::TOO_MANY_REQUESTS => RATE_LIMIT_EXCEEDED,
        StatusCode::BAD_GATEWAY => UPSTREAM_UNAVAILABLE,
        StatusCode::GATEWAY_TIMEOUT => UPSTREAM_TIMEOUT,
        StatusCode::SERVICE_UNAVAILABLE => SERVICE_UNAVAILABLE,
        _ => INTERNAL_ERROR,
    }
}

/// Request id of the request being handled, or a fresh UUID outside a request.
pub fn current_request_id() -> String {
    CURRENT_REQUEST_ID
        .try_with(|id| id.clone())
        .unwrap_or_else(|_| Uuid::new_v4().to_string())
}

impl<T: Serialize> ApiResponse<T> {
    /// Wrap a successful payload.
    pub fn success(data: T) -> (StatusCode, Json<Self>) {
        (
            StatusCode::OK,
            Json(Self {
                status: "success",
                data: Some(data),
                error: None,
            }),
        )
    }

    /// Wrap an error with an explicit HTTP status code, error code, and message.
    ///
    /// For 5xx statuses `message` is logged and replaced with
    /// [`safe_message`] so internal details never reach clients.
    pub fn error(
        status_code: StatusCode,
        code: &'static str,
        message: impl Into<String>,
    ) -> (StatusCode, Json<Self>) {
        Self::build_error(status_code, code, message.into(), current_request_id(), None)
    }

    /// Like [`ApiResponse::error`] with an explicit request id.
    pub fn error_with_request_id(
        status_code: StatusCode,
        code: &'static str,
        message: impl Into<String>,
        request_id: String,
    ) -> (StatusCode, Json<Self>) {
        Self::build_error(status_code, code, message.into(), request_id, None)
    }

    /// `400 INVALID_INPUT` with field-level details.
    pub fn validation_error(
        message: impl Into<String>,
        details: Vec<FieldViolation>,
    ) -> (StatusCode, Json<Self>) {
        Self::build_error(
            StatusCode::BAD_REQUEST,
            error_codes::INVALID_INPUT,
            message.into(),
            current_request_id(),
            Some(details),
        )
    }

    fn build_error(
        status_code: StatusCode,
        code: &'static str,
        message: String,
        request_id: String,
        details: Option<Vec<FieldViolation>>,
    ) -> (StatusCode, Json<Self>) {
        let message = if status_code.is_server_error() {
            if message != safe_message(code) {
                tracing::error!(
                    request_id = %request_id,
                    status = status_code.as_u16(),
                    code,
                    detail = %message,
                    "request failed"
                );
            }
            safe_message(code).to_string()
        } else {
            message
        };
        (
            status_code,
            Json(Self {
                status: "error",
                data: None,
                error: Some(ErrorDetail {
                    version: ERROR_ENVELOPE_VERSION,
                    code,
                    message,
                    request_id,
                    retryable: is_retryable(code),
                    details,
                }),
            }),
        )
    }
}

impl<T: Serialize + Send> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> Response {
        let code = if self.error.is_some() {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::OK
        };
        (code, Json(self)).into_response()
    }
}

/// Message included in every 429 response.
pub const RATE_LIMIT_MESSAGE: &str = "Too many requests";

/// Build the HTTP 429 response used by the rate limiters.
pub fn rate_limit_error_response() -> Response {
    ApiResponse::<()>::error(
        StatusCode::TOO_MANY_REQUESTS,
        error_codes::RATE_LIMIT_EXCEEDED,
        RATE_LIMIT_MESSAGE,
    )
    .into_response()
}

/// Build the HTTP 503 response used when the distributed rate limiter cannot
/// reach its store on a fail-closed tier.
pub fn rate_limiter_unavailable_response() -> Response {
    ApiResponse::<()>::error(
        StatusCode::SERVICE_UNAVAILABLE,
        error_codes::RATE_LIMITER_UNAVAILABLE,
        safe_message(error_codes::RATE_LIMITER_UNAVAILABLE),
    )
    .into_response()
}

fn inbound_request_id(req: &Request) -> String {
    for name in [REQUEST_ID_HEADER, "x-correlation-id"] {
        if let Some(value) = req.headers().get(name).and_then(|v| v.to_str().ok()) {
            let value = value.trim();
            if !value.is_empty()
                && value.len() <= MAX_REQUEST_ID_LEN
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
            {
                return value.to_string();
            }
        }
    }
    Uuid::new_v4().to_string()
}

fn is_json(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.starts_with("application/json"))
        .unwrap_or(false)
}

/// Middleware that scopes a request id for [`ApiResponse::error`], echoes it
/// in `x-request-id`, and rewrites any non-envelope error response (axum
/// extractor rejections, unknown routes, body limits, bare status codes) into
/// the standard error envelope.
pub async fn error_envelope_layer(req: Request, next: Next) -> Response {
    let request_id = inbound_request_id(&req);
    let response = CURRENT_REQUEST_ID
        .scope(request_id.clone(), next.run(req))
        .await;

    let status = response.status();
    let mut response = if (status.is_client_error() || status.is_server_error())
        && !is_json(&response)
    {
        let (parts, body) = response.into_parts();
        let code = code_for_status(status);
        let message = if status.is_client_error() {
            let text = axum::body::to_bytes(body, MAX_REWRITE_BODY_BYTES)
                .await
                .ok()
                .and_then(|b| String::from_utf8(b.to_vec()).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| safe_message(code).to_string());
            text.chars().take(MAX_REJECTION_MESSAGE_LEN).collect()
        } else {
            safe_message(code).to_string()
        };
        let mut rewritten =
            ApiResponse::<()>::error_with_request_id(status, code, message, request_id.clone())
                .into_response();
        // Keep caller-set headers such as Retry-After, Allow and rate-limit
        // headers; the body-describing ones come from the new body.
        for (name, value) in parts.headers.iter() {
            if name != header::CONTENT_TYPE && name != header::CONTENT_LENGTH {
                rewritten.headers_mut().insert(name.clone(), value.clone());
            }
        }
        rewritten
    } else {
        response
    };

    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    response
}

/// Fallback for unmatched routes: `404 NOT_FOUND` in the standard envelope.
pub async fn not_found_fallback() -> Response {
    ApiResponse::<()>::error(
        StatusCode::NOT_FOUND,
        error_codes::NOT_FOUND,
        safe_message(error_codes::NOT_FOUND),
    )
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn to_value<T: Serialize>(r: &ApiResponse<T>) -> Value {
        serde_json::to_value(r).unwrap()
    }

    #[test]
    fn success_sets_status_and_data() {
        let (code, Json(resp)) = ApiResponse::success(42u32);
        assert_eq!(code, StatusCode::OK);
        assert_eq!(resp.status, "success");
        assert_eq!(resp.data, Some(42u32));
        assert!(resp.error.is_none());
    }

    #[test]
    fn error_sets_status_and_message() {
        let (code, Json(resp)) =
            ApiResponse::<()>::error(StatusCode::NOT_FOUND, "NOT_FOUND", "not found");
        assert_eq!(code, StatusCode::NOT_FOUND);
        assert_eq!(resp.status, "error");
        assert!(resp.data.is_none());
        assert_eq!(resp.error.as_ref().unwrap().code, "NOT_FOUND");
        assert_eq!(resp.error.as_ref().unwrap().message, "not found");
        assert!(!resp.error.as_ref().unwrap().request_id.is_empty());
    }

    #[test]
    fn success_serializes_without_error_field() {
        let (_, Json(resp)) = ApiResponse::success("hello");
        let v = to_value(&resp);
        assert!(
            v.get("error").is_none(),
            "error field must be absent on success"
        );
        assert_eq!(v["status"], "success");
        assert_eq!(v["data"], "hello");
    }

    #[test]
    fn error_serializes_without_data_field() {
        let (_, Json(resp)) =
            ApiResponse::<()>::error(StatusCode::BAD_REQUEST, "INVALID_INPUT", "bad input");
        let v = to_value(&resp);
        assert!(
            v.get("data").is_none(),
            "data field must be absent on error"
        );
        assert_eq!(v["status"], "error");
        assert_eq!(v["error"]["code"], "INVALID_INPUT");
        assert_eq!(v["error"]["message"], "bad input");
        assert!(v["error"]["request_id"].is_string());
    }

    #[test]
    fn success_works_with_struct_payload() {
        #[derive(Serialize, PartialEq, Debug)]
        struct Payload {
            id: u32,
            name: String,
        }
        let (code, Json(resp)) = ApiResponse::success(Payload {
            id: 1,
            name: "pool".into(),
        });
        assert_eq!(code, StatusCode::OK);
        assert_eq!(resp.data.unwrap().id, 1);
    }

    #[test]
    fn error_with_custom_request_id() {
        let custom_id = "custom-request-123".to_string();
        let (_, Json(resp)) = ApiResponse::<()>::error_with_request_id(
            StatusCode::SERVICE_UNAVAILABLE,
            "SERVICE_UNAVAILABLE",
            "service temporarily unavailable",
            custom_id.clone(),
        );
        assert_eq!(resp.status, "error");
        assert_eq!(resp.error.as_ref().unwrap().request_id, custom_id);
    }
}
