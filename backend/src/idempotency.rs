//! Idempotency keys for market mutations (#48).
//!
//! A client that times out after the server accepted a create, prediction, or
//! claim-submission request can safely retry it by sending the same
//! `Idempotency-Key` header: the server executes the request once and replays
//! the stored response to every exact retry.
//!
//! ## Semantics
//!
//! * **Key format** — 1..=[`MAX_KEY_LEN`] visible ASCII characters. Clients
//!   should send a fresh UUID per logical operation. Requests without the
//!   header are not deduplicated.
//! * **Scope** — a record is bound to `(wallet, operation, key)`. The wallet
//!   is the `sub` of the verified bearer access token, so a key is only
//!   honoured for the wallet that created it, and the same key on another
//!   endpoint is an independent record. Sending the header without a valid
//!   access token is rejected with `401`.
//! * **Fingerprint** — SHA-256 over the HTTP method, request path, and the
//!   JSON body with object keys sorted (so field order and whitespace do not
//!   matter). Non-JSON bodies are hashed byte-for-byte.
//! * **Retention** — completed records are replayed for
//!   [`DEFAULT_RETENTION`] (24h) after the first request. Afterwards the key
//!   is treated as unused; [`IdempotencyStore::purge_expired`] deletes the
//!   rows.
//! * **Conflicts**
//!   - exact retry of a completed request → stored status and body replayed
//!     with `Idempotent-Replayed: true`;
//!   - same key, different fingerprint → `422 IDEMPOTENCY_KEY_REUSED`, the
//!     request is not executed;
//!   - same key while the first request is still executing → `409
//!     IDEMPOTENCY_REQUEST_IN_PROGRESS` with `Retry-After: 1`.
//! * **What is stored** — `2xx` and deterministic `4xx` responses. `5xx` and
//!   transient `4xx` (408, 409, 425, 429) release the key so a retry
//!   re-executes. Responses produced by this middleware are never stored.
//! * **Crash recovery** — the executing request holds a lease of
//!   [`DEFAULT_LEASE`]. If the process dies mid-request, an exact retry takes
//!   the record over once the lease expires. A slow original owner can no
//!   longer overwrite the record afterwards: completion is fenced by an owner
//!   token.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::MethodRouter;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use tracing::{debug, warn};

use crate::response::{error_codes, ApiResponse};

/// Request header carrying the client-chosen idempotency key.
pub const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
/// Response header set on replayed responses.
pub const IDEMPOTENT_REPLAYED_HEADER: &str = "idempotent-replayed";
/// Maximum accepted key length.
pub const MAX_KEY_LEN: usize = 255;
/// How long a key is honoured after the first request.
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
/// How long an executing request owns its key before a retry may take over.
pub const DEFAULT_LEASE: Duration = Duration::from_secs(30);
/// Upper bound on a request body read for fingerprinting.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// Upper bound on a response body recorded for replay.
pub const MAX_STORED_RESPONSE_BYTES: usize = 64 * 1024;

/// Error codes emitted by the idempotency middleware.
pub mod codes {
    pub const KEY_INVALID: &str = "IDEMPOTENCY_KEY_INVALID";
    pub const KEY_REUSED: &str = "IDEMPOTENCY_KEY_REUSED";
    pub const IN_PROGRESS: &str = "IDEMPOTENCY_REQUEST_IN_PROGRESS";
}

/// Retention and lease durations.
#[derive(Debug, Clone, Copy)]
pub struct IdempotencyPolicy {
    pub retention: Duration,
    pub lease: Duration,
}

impl Default for IdempotencyPolicy {
    fn default() -> Self {
        Self {
            retention: DEFAULT_RETENTION,
            lease: DEFAULT_LEASE,
        }
    }
}

/// The `(wallet, operation, key)` triple a record is bound to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Scope {
    pub wallet: String,
    pub operation: String,
    pub key: String,
}

/// A response recorded for replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// Outcome of [`IdempotencyStore::begin`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Begin {
    /// The caller owns the key and must execute the request, then call
    /// [`IdempotencyStore::complete`] or [`IdempotencyStore::release`] with
    /// this owner token.
    Owned { owner_token: String },
    /// An exact retry of a completed request.
    Replay(StoredResponse),
    /// The key was already used with a different request fingerprint.
    Mismatch,
    /// The original request is still executing.
    InProgress,
}

#[derive(Debug, Clone)]
struct MemoryRecord {
    fingerprint: String,
    owner_token: Option<String>,
    locked_until: Option<Instant>,
    response: Option<StoredResponse>,
    expires_at: Instant,
}

/// Persistence for idempotency records.
///
/// [`postgres`](Self::postgres) is used by the server so records are shared
/// by every replica; [`memory`](Self::memory) backs unit tests and has
/// identical semantics.
#[derive(Clone)]
pub struct IdempotencyStore(Backend);

#[derive(Clone)]
enum Backend {
    Postgres(PgPool),
    Memory(Arc<Mutex<HashMap<Scope, MemoryRecord>>>),
}

impl IdempotencyStore {
    pub fn postgres(pool: PgPool) -> Self {
        Self(Backend::Postgres(pool))
    }

    pub fn memory() -> Self {
        Self(Backend::Memory(Arc::new(Mutex::new(HashMap::new()))))
    }

    /// Atomically claim `scope` for a request with `fingerprint`, or report
    /// what the existing record says about it.
    pub async fn begin(
        &self,
        scope: &Scope,
        fingerprint: &str,
        policy: IdempotencyPolicy,
    ) -> Result<Begin, sqlx::Error> {
        let owner_token = uuid::Uuid::new_v4().to_string();
        match &self.0 {
            Backend::Memory(records) => {
                let mut records = records.lock().expect("idempotency store lock");
                let now = Instant::now();
                let claimable = match records.get(scope) {
                    None => true,
                    Some(r) if r.expires_at <= now => true,
                    Some(r) if r.fingerprint != fingerprint => return Ok(Begin::Mismatch),
                    Some(r) => match &r.response {
                        Some(stored) => return Ok(Begin::Replay(stored.clone())),
                        None => r.locked_until.is_some_and(|until| until <= now),
                    },
                };
                if !claimable {
                    return Ok(Begin::InProgress);
                }
                records.insert(
                    scope.clone(),
                    MemoryRecord {
                        fingerprint: fingerprint.to_string(),
                        owner_token: Some(owner_token.clone()),
                        locked_until: Some(now + policy.lease),
                        response: None,
                        expires_at: now + policy.retention,
                    },
                );
                Ok(Begin::Owned { owner_token })
            }
            Backend::Postgres(pool) => {
                // Two attempts cover a record that expires between the claim
                // and the follow-up read.
                for _ in 0..2 {
                    // Insert a fresh record, or take over one that has expired
                    // or whose owner abandoned an exact-match request.
                    let claimed = sqlx::query(
                        r#"
                        INSERT INTO idempotency_keys (
                            wallet, operation, idempotency_key, request_fingerprint,
                            state, owner_token, locked_until, expires_at
                        )
                        VALUES (
                            $1, $2, $3, $4, 'in_progress', $5,
                            NOW() + ($6::bigint * INTERVAL '1 millisecond'),
                            NOW() + ($7::bigint * INTERVAL '1 millisecond')
                        )
                        ON CONFLICT (wallet, operation, idempotency_key) DO UPDATE SET
                            request_fingerprint = EXCLUDED.request_fingerprint,
                            state = 'in_progress',
                            owner_token = EXCLUDED.owner_token,
                            locked_until = EXCLUDED.locked_until,
                            response_status = NULL,
                            response_content_type = NULL,
                            response_body = NULL,
                            created_at = NOW(),
                            completed_at = NULL,
                            expires_at = EXCLUDED.expires_at
                        WHERE idempotency_keys.expires_at <= NOW()
                           OR (idempotency_keys.state = 'in_progress'
                               AND idempotency_keys.locked_until <= NOW()
                               AND idempotency_keys.request_fingerprint = EXCLUDED.request_fingerprint)
                        RETURNING owner_token
                        "#,
                    )
                    .bind(&scope.wallet)
                    .bind(&scope.operation)
                    .bind(&scope.key)
                    .bind(fingerprint)
                    .bind(&owner_token)
                    .bind(duration_ms(policy.lease))
                    .bind(duration_ms(policy.retention))
                    .fetch_optional(pool)
                    .await?;
                    if claimed.is_some() {
                        return Ok(Begin::Owned { owner_token });
                    }

                    let existing = sqlx::query(
                        r#"
                        SELECT request_fingerprint, state, response_status,
                               response_content_type, response_body
                        FROM idempotency_keys
                        WHERE wallet = $1 AND operation = $2 AND idempotency_key = $3
                          AND expires_at > NOW()
                        "#,
                    )
                    .bind(&scope.wallet)
                    .bind(&scope.operation)
                    .bind(&scope.key)
                    .fetch_optional(pool)
                    .await?;
                    let Some(row) = existing else { continue };

                    let stored_fingerprint: String = row.try_get("request_fingerprint")?;
                    if stored_fingerprint != fingerprint {
                        return Ok(Begin::Mismatch);
                    }
                    let state: String = row.try_get("state")?;
                    if state != "completed" {
                        return Ok(Begin::InProgress);
                    }
                    let status: i16 = row.try_get("response_status")?;
                    return Ok(Begin::Replay(StoredResponse {
                        status: status as u16,
                        content_type: row.try_get("response_content_type")?,
                        body: row
                            .try_get::<Option<Vec<u8>>, _>("response_body")?
                            .unwrap_or_default(),
                    }));
                }
                Ok(Begin::InProgress)
            }
        }
    }

    /// Record the final response. Returns `false` when `owner_token` no
    /// longer owns the record (its lease expired and a retry took over).
    pub async fn complete(
        &self,
        scope: &Scope,
        owner_token: &str,
        response: &StoredResponse,
    ) -> Result<bool, sqlx::Error> {
        match &self.0 {
            Backend::Memory(records) => {
                let mut records = records.lock().expect("idempotency store lock");
                match records.get_mut(scope) {
                    Some(r)
                        if r.response.is_none()
                            && r.owner_token.as_deref() == Some(owner_token) =>
                    {
                        r.response = Some(response.clone());
                        r.owner_token = None;
                        r.locked_until = None;
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
            Backend::Postgres(pool) => {
                let result = sqlx::query(
                    r#"
                    UPDATE idempotency_keys
                    SET state = 'completed',
                        response_status = $5,
                        response_content_type = $6,
                        response_body = $7,
                        completed_at = NOW(),
                        owner_token = NULL,
                        locked_until = NULL
                    WHERE wallet = $1 AND operation = $2 AND idempotency_key = $3
                      AND state = 'in_progress' AND owner_token = $4
                    "#,
                )
                .bind(&scope.wallet)
                .bind(&scope.operation)
                .bind(&scope.key)
                .bind(owner_token)
                .bind(response.status as i16)
                .bind(&response.content_type)
                .bind(&response.body)
                .execute(pool)
                .await?;
                Ok(result.rows_affected() == 1)
            }
        }
    }

    /// Drop an in-progress record so the next retry re-executes the request.
    pub async fn release(&self, scope: &Scope, owner_token: &str) -> Result<(), sqlx::Error> {
        match &self.0 {
            Backend::Memory(records) => {
                let mut records = records.lock().expect("idempotency store lock");
                if records
                    .get(scope)
                    .is_some_and(|r| r.owner_token.as_deref() == Some(owner_token))
                {
                    records.remove(scope);
                }
                Ok(())
            }
            Backend::Postgres(pool) => {
                sqlx::query(
                    r#"
                    DELETE FROM idempotency_keys
                    WHERE wallet = $1 AND operation = $2 AND idempotency_key = $3
                      AND state = 'in_progress' AND owner_token = $4
                    "#,
                )
                .bind(&scope.wallet)
                .bind(&scope.operation)
                .bind(&scope.key)
                .bind(owner_token)
                .execute(pool)
                .await?;
                Ok(())
            }
        }
    }

    /// Delete records past their retention window. Returns rows removed.
    pub async fn purge_expired(&self) -> Result<u64, sqlx::Error> {
        match &self.0 {
            Backend::Memory(records) => {
                let mut records = records.lock().expect("idempotency store lock");
                let now = Instant::now();
                let before = records.len();
                records.retain(|_, r| r.expires_at > now);
                Ok((before - records.len()) as u64)
            }
            Backend::Postgres(pool) => {
                let result = sqlx::query("DELETE FROM idempotency_keys WHERE expires_at <= NOW()")
                    .execute(pool)
                    .await?;
                Ok(result.rows_affected())
            }
        }
    }
}

fn duration_ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

/// Validate an `Idempotency-Key` header value.
pub fn validate_key(raw: &str) -> Result<&str, &'static str> {
    if raw.is_empty() {
        return Err("Idempotency-Key must not be empty");
    }
    if raw.len() > MAX_KEY_LEN {
        return Err("Idempotency-Key must be at most 255 characters");
    }
    if !raw.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("Idempotency-Key must contain only visible ASCII characters");
    }
    Ok(raw)
}

/// Fingerprint a request: SHA-256 over method, path, and canonical body.
pub fn fingerprint(method: &str, path: &str, body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(method.as_bytes());
    hasher.update([0]);
    hasher.update(path.as_bytes());
    hasher.update([0]);
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(value) => hasher.update(canonical_json(&value).as_bytes()),
        Err(_) => hasher.update(body),
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Serialise JSON with object keys sorted at every level.
pub fn canonical_json(value: &serde_json::Value) -> String {
    use serde_json::Value;
    fn write(value: &Value, out: &mut String) {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                out.push('{');
                for (i, key) in keys.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&Value::String(key.clone()).to_string());
                    out.push(':');
                    write(&map[key], out);
                }
                out.push('}');
            }
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            scalar => out.push_str(&scalar.to_string()),
        }
    }
    let mut out = String::new();
    write(value, &mut out);
    out
}

/// Whether a response is final and should be replayed to retries.
pub fn is_replayable(status: StatusCode) -> bool {
    if status.is_success() {
        return true;
    }
    status.is_client_error()
        && !matches!(
            status,
            StatusCode::REQUEST_TIMEOUT
                | StatusCode::CONFLICT
                | StatusCode::TOO_EARLY
                | StatusCode::TOO_MANY_REQUESTS
        )
}

/// Middleware state for one mutation endpoint.
#[derive(Clone)]
pub struct Idempotency {
    store: IdempotencyStore,
    operation: &'static str,
    policy: IdempotencyPolicy,
    jwt_secret: Arc<str>,
    jwt_key_version: u64,
}

impl Idempotency {
    pub fn new(
        store: IdempotencyStore,
        operation: &'static str,
        jwt_secret: &str,
        jwt_key_version: u64,
    ) -> Self {
        Self {
            store,
            operation,
            policy: IdempotencyPolicy::default(),
            jwt_secret: Arc::from(jwt_secret),
            jwt_key_version,
        }
    }

    pub fn with_policy(mut self, policy: IdempotencyPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Wrap `route` so requests carrying `Idempotency-Key` are deduplicated.
    pub fn guard<S>(self, route: MethodRouter<S>) -> MethodRouter<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        route.layer(axum::middleware::from_fn_with_state(
            self,
            idempotency_middleware,
        ))
    }

    fn wallet(&self, headers: &HeaderMap) -> Option<String> {
        let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
        let token = crate::jwt::extract_bearer_token(value)?;
        crate::jwt::verify_jwt_token_strict(token, &self.jwt_secret, "access", self.jwt_key_version)
            .ok()
            .map(|claims| claims.sub)
    }
}

fn error_response(status: StatusCode, code: &'static str, message: &str) -> Response {
    ApiResponse::<()>::error(status, code, message).into_response()
}

fn replay(stored: StoredResponse) -> Response {
    let status = StatusCode::from_u16(stored.status).unwrap_or(StatusCode::OK);
    let mut response = (status, Body::from(stored.body)).into_response();
    let headers = response.headers_mut();
    if let Some(content_type) = stored
        .content_type
        .and_then(|ct| HeaderValue::from_str(&ct).ok())
    {
        headers.insert(header::CONTENT_TYPE, content_type);
    }
    headers.insert(IDEMPOTENT_REPLAYED_HEADER, HeaderValue::from_static("true"));
    response
}

/// Axum middleware implementing the semantics in the module docs.
pub async fn idempotency_middleware(
    State(idem): State<Idempotency>,
    request: Request,
    next: Next,
) -> Response {
    let Some(raw_key) = request.headers().get(IDEMPOTENCY_KEY_HEADER) else {
        return next.run(request).await;
    };
    let key = match raw_key
        .to_str()
        .map_err(|_| "Idempotency-Key must be ASCII")
    {
        Ok(raw) => match validate_key(raw) {
            Ok(key) => key.to_string(),
            Err(msg) => return error_response(StatusCode::BAD_REQUEST, codes::KEY_INVALID, msg),
        },
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, codes::KEY_INVALID, msg),
    };
    let Some(wallet) = idem.wallet(request.headers()) else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            error_codes::UNAUTHORIZED,
            "Idempotency-Key requires a valid bearer access token",
        );
    };

    let (parts, body) = request.into_parts();
    let bytes: Bytes = match axum::body::to_bytes(body, MAX_REQUEST_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                error_codes::INVALID_INPUT,
                "request body too large",
            )
        }
    };
    let scope = Scope {
        wallet,
        operation: idem.operation.to_string(),
        key,
    };
    let fp = fingerprint(parts.method.as_str(), parts.uri.path(), &bytes);

    let owner_token = match idem.store.begin(&scope, &fp, idem.policy).await {
        Ok(Begin::Owned { owner_token }) => owner_token,
        Ok(Begin::Replay(stored)) => {
            debug!(operation = idem.operation, "replaying idempotent response");
            return replay(stored);
        }
        Ok(Begin::Mismatch) => {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                codes::KEY_REUSED,
                "Idempotency-Key was already used with a different request",
            )
        }
        Ok(Begin::InProgress) => {
            let mut response = error_response(
                StatusCode::CONFLICT,
                codes::IN_PROGRESS,
                "a request with this Idempotency-Key is still being processed",
            );
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
            return response;
        }
        Err(e) => {
            warn!(error = %e, "idempotency store unavailable");
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                error_codes::DATABASE_UNAVAILABLE,
                "idempotency store unavailable",
            );
        }
    };

    let response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    let status = response.status();

    if !is_replayable(status) {
        if let Err(e) = idem.store.release(&scope, &owner_token).await {
            warn!(error = %e, "failed to release idempotency key");
        }
        return response;
    }

    let (parts, body) = response.into_parts();
    let body = match axum::body::to_bytes(body, MAX_STORED_RESPONSE_BYTES).await {
        Ok(body) => body,
        Err(_) => {
            let _ = idem.store.release(&scope, &owner_token).await;
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                error_codes::INTERNAL_ERROR,
                "response too large to record for idempotent replay",
            );
        }
    };
    let stored = StoredResponse {
        status: status.as_u16(),
        content_type: parts
            .headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        body: body.to_vec(),
    };
    match idem.store.complete(&scope, &owner_token, &stored).await {
        Ok(true) => {}
        Ok(false) => warn!(
            operation = idem.operation,
            "idempotency lease expired before completion; response not recorded"
        ),
        Err(e) => warn!(error = %e, "failed to record idempotent response"),
    }
    Response::from_parts(parts, Body::from(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::post;
    use axum::Router;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    const SECRET: &str = "test-secret-key-that-is-long-enough-for-hs256-0123456789";

    fn token(wallet: &str) -> String {
        crate::jwt::sign_jwt_for_test(wallet, SECRET)
    }

    fn scope(key: &str) -> Scope {
        Scope {
            wallet: "GWALLET".into(),
            operation: "op".into(),
            key: key.into(),
        }
    }

    /// A router whose handler counts executions and sleeps so concurrent
    /// duplicates overlap.
    fn app(store: IdempotencyStore, policy: IdempotencyPolicy) -> (Router, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let handler = post(move |body: String| {
            let counter = counter.clone();
            async move {
                let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                tokio::time::sleep(Duration::from_millis(50)).await;
                if body.contains("fail") {
                    return (StatusCode::INTERNAL_SERVER_ERROR, "boom".to_string());
                }
                (StatusCode::OK, format!("{{\"execution\":{n}}}"))
            }
        });
        let idem = Idempotency::new(store, "predictions.create", SECRET, 0).with_policy(policy);
        (Router::new().route("/p", idem.guard(handler)), calls)
    }

    fn req(key: Option<&str>, wallet: Option<&str>, body: &str) -> Request {
        let mut builder = Request::builder().method("POST").uri("/p");
        if let Some(key) = key {
            builder = builder.header(IDEMPOTENCY_KEY_HEADER, key);
        }
        if let Some(wallet) = wallet {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {}", token(wallet)));
        }
        builder.body(Body::from(body.to_string())).unwrap()
    }

    async fn send(app: &Router, request: Request) -> (StatusCode, bool, String) {
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let replayed = response.headers().contains_key(IDEMPOTENT_REPLAYED_HEADER);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, replayed, String::from_utf8(body.to_vec()).unwrap())
    }

    #[test]
    fn fingerprint_ignores_key_order_and_whitespace() {
        let a = fingerprint("POST", "/p", br#"{"a":1,"b":{"c":2,"d":[1,2]}}"#);
        let b = fingerprint(
            "POST",
            "/p",
            b"{ \"b\": {\"d\": [1, 2], \"c\": 2}, \"a\": 1 }",
        );
        assert_eq!(a, b);
        assert_ne!(
            a,
            fingerprint("POST", "/p", br#"{"a":2,"b":{"c":2,"d":[1,2]}}"#)
        );
        assert_ne!(
            a,
            fingerprint("POST", "/q", br#"{"a":1,"b":{"c":2,"d":[1,2]}}"#)
        );
        assert_ne!(
            a,
            fingerprint("PUT", "/p", br#"{"a":1,"b":{"c":2,"d":[1,2]}}"#)
        );
    }

    #[test]
    fn key_validation() {
        assert!(validate_key("3f2a-uuid").is_ok());
        assert!(validate_key("").is_err());
        assert!(validate_key(&"k".repeat(MAX_KEY_LEN)).is_ok());
        assert!(validate_key(&"k".repeat(MAX_KEY_LEN + 1)).is_err());
        assert!(validate_key("has space").is_err());
    }

    #[test]
    fn only_final_responses_are_replayable() {
        assert!(is_replayable(StatusCode::OK));
        assert!(is_replayable(StatusCode::BAD_REQUEST));
        assert!(is_replayable(StatusCode::NOT_FOUND));
        assert!(!is_replayable(StatusCode::CONFLICT));
        assert!(!is_replayable(StatusCode::TOO_MANY_REQUESTS));
        assert!(!is_replayable(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(!is_replayable(StatusCode::SERVICE_UNAVAILABLE));
    }

    #[tokio::test]
    async fn requests_without_key_are_not_deduplicated() {
        let (app, calls) = app(IdempotencyStore::memory(), IdempotencyPolicy::default());
        send(&app, req(None, None, "{}")).await;
        send(&app, req(None, None, "{}")).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn key_without_valid_token_is_rejected() {
        let (app, calls) = app(IdempotencyStore::memory(), IdempotencyPolicy::default());
        let (status, _, _) = send(&app, req(Some("k1"), None, "{}")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn exact_retry_replays_stored_response() {
        let (app, calls) = app(IdempotencyStore::memory(), IdempotencyPolicy::default());
        let first = send(&app, req(Some("k1"), Some("GA"), r#"{"x":1}"#)).await;
        let retry = send(&app, req(Some("k1"), Some("GA"), r#"{ "x": 1 }"#)).await;
        assert_eq!(first.0, StatusCode::OK);
        assert!(!first.1);
        assert_eq!(retry.0, StatusCode::OK);
        assert!(retry.1, "retry must be marked as replayed");
        assert_eq!(first.2, retry.2);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn same_key_different_payload_conflicts() {
        let (app, calls) = app(IdempotencyStore::memory(), IdempotencyPolicy::default());
        send(&app, req(Some("k1"), Some("GA"), r#"{"x":1}"#)).await;
        let (status, _, body) = send(&app, req(Some("k1"), Some("GA"), r#"{"x":2}"#)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body.contains(codes::KEY_REUSED), "{body}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn keys_are_bound_to_the_wallet() {
        let (app, calls) = app(IdempotencyStore::memory(), IdempotencyPolicy::default());
        send(&app, req(Some("k1"), Some("GA"), r#"{"x":1}"#)).await;
        let other = send(&app, req(Some("k1"), Some("GB"), r#"{"x":2}"#)).await;
        assert_eq!(other.0, StatusCode::OK);
        assert!(!other.1, "another wallet must never see a replay");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn concurrent_duplicates_execute_once() {
        let (app, calls) = app(IdempotencyStore::memory(), IdempotencyPolicy::default());
        let requests = (0..16).map(|_| {
            let app = app.clone();
            async move { send(&app, req(Some("k1"), Some("GA"), r#"{"x":1}"#)).await }
        });
        let results = futures_join_all(requests).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let ok = results.iter().filter(|r| r.0 == StatusCode::OK).count();
        let in_progress = results
            .iter()
            .filter(|r| r.0 == StatusCode::CONFLICT && r.2.contains(codes::IN_PROGRESS))
            .count();
        assert_eq!(ok + in_progress, results.len(), "{results:?}");
        assert!(ok >= 1);
        // Once the winner finishes, every retry is a replay.
        let retry = send(&app, req(Some("k1"), Some("GA"), r#"{"x":1}"#)).await;
        assert!(retry.1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn server_errors_release_the_key() {
        let (app, calls) = app(IdempotencyStore::memory(), IdempotencyPolicy::default());
        let first = send(&app, req(Some("k1"), Some("GA"), r#"{"fail":true}"#)).await;
        assert_eq!(first.0, StatusCode::INTERNAL_SERVER_ERROR);
        let retry = send(&app, req(Some("k1"), Some("GA"), r#"{"fail":true}"#)).await;
        assert!(!retry.1, "5xx must not be replayed");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn expired_records_are_reusable_and_purged() {
        let policy = IdempotencyPolicy {
            retention: Duration::from_millis(100),
            lease: Duration::from_secs(5),
        };
        let store = IdempotencyStore::memory();
        let (app, calls) = app(store.clone(), policy);
        send(&app, req(Some("k1"), Some("GA"), r#"{"x":1}"#)).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        // After retention a different payload is a fresh request, not a conflict.
        let after = send(&app, req(Some("k1"), Some("GA"), r#"{"x":2}"#)).await;
        assert_eq!(after.0, StatusCode::OK);
        assert!(!after.1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(store.purge_expired().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn abandoned_lease_is_taken_over_and_fenced() {
        let store = IdempotencyStore::memory();
        let policy = IdempotencyPolicy {
            retention: Duration::from_secs(60),
            lease: Duration::from_millis(50),
        };
        let crashed = match store.begin(&scope("k"), "fp", policy).await.unwrap() {
            Begin::Owned { owner_token } => owner_token,
            other => panic!("expected ownership, got {other:?}"),
        };
        assert_eq!(
            store.begin(&scope("k"), "fp", policy).await.unwrap(),
            Begin::InProgress
        );
        tokio::time::sleep(Duration::from_millis(80)).await;
        let retry = match store.begin(&scope("k"), "fp", policy).await.unwrap() {
            Begin::Owned { owner_token } => owner_token,
            other => panic!("expected takeover, got {other:?}"),
        };
        let stored = StoredResponse {
            status: 200,
            content_type: None,
            body: b"late".to_vec(),
        };
        assert!(!store
            .complete(&scope("k"), &crashed, &stored)
            .await
            .unwrap());
        assert!(store.complete(&scope("k"), &retry, &stored).await.unwrap());
    }

    async fn futures_join_all<F>(futures: impl Iterator<Item = F>) -> Vec<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let handles: Vec<_> = futures.map(tokio::spawn).collect();
        let mut out = Vec::with_capacity(handles.len());
        for handle in handles {
            out.push(handle.await.expect("task panicked"));
        }
        out
    }
}
