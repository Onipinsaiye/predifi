//! WebSocket broadcast for live prediction events.
//!
//! Clients connect at `GET /api/v1/ws?address=<wallet>` with a valid access JWT
//! in the `Authorization: Bearer <token>` header. Query-parameter tokens are
//! accepted only outside production (they leak into access logs). Subscribers
//! receive events whose `user_address` matches the subscribed wallet.
//! Omitting `address` delivers all events (useful for dashboards). The indexer
//! calls [`EventBus::send`] whenever a new prediction is indexed.
//!
//! # Resumable delivery
//!
//! Every published event is stamped with a strictly monotonic `cursor` (u64).
//! On reconnect a client can pass `?cursor=<last_seen>` to replay missed events
//! up to [`MAX_REPLAY_SIZE`] from the in-memory [`ReplayBuffer`].  Gap frames
//! are sent when the cursor is expired, ahead of the server, or when the replay
//! is capped.

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::broadcast;
use tracing::info_span;
use tracing::Instrument;

use crate::config::Config;
use crate::constants::{RATE_LIMIT_WS_BURST, RATE_LIMIT_WS_PERIOD_SECS};
use crate::jwt::{extract_bearer_token, verify_jwt_token_strict};

const CHANNEL_CAPACITY: usize = 256;

/// Maximum allowed message size in bytes to prevent memory exhaustion attacks.
const MAX_MESSAGE_SIZE: usize = 1_048_576; // 1 MB

/// Maximum active concurrent WebSocket connections allowed before returning 429.
const MAX_ACTIVE_CONNECTIONS: usize = 10_000;

/// Per-IP handshake attempts allowed inside [`CONNECT_WINDOW`].
const MAX_CONNECT_ATTEMPTS_PER_IP: usize = 10;
const CONNECT_WINDOW: Duration = Duration::from_secs(60);

/// Number of recent events held in memory for replay upon reconnection.
pub const DEFAULT_RETENTION_CAPACITY: usize = 5_000;

/// Maximum events replayed to a single reconnecting client.
pub const MAX_REPLAY_SIZE: usize = 250;

// ---------------------------------------------------------------------------
// Sequenced event & replay types
// ---------------------------------------------------------------------------

/// A single broadcast event with its assigned cursor.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SequencedEvent {
    pub cursor: u64,
    pub payload: String,
    pub pool_id: Option<u64>,
    pub user_address: Option<String>,
}

/// Result returned by [`ReplayBuffer::get_replay`].
pub enum ReplayResult {
    Empty,
    CursorExpired {
        oldest_cursor: u64,
        latest_cursor: u64,
    },
    CursorAhead {
        latest_cursor: u64,
    },
    Events {
        events: Vec<SequencedEvent>,
        truncated: bool,
        next_cursor: Option<u64>,
        latest_cursor: u64,
    },
}

/// Bounded ring buffer that retains recent sequenced events for replay.
pub struct ReplayBuffer {
    events: std::collections::VecDeque<SequencedEvent>,
    capacity: usize,
    oldest_cursor: u64,
    latest_cursor: u64,
}

impl ReplayBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            events: std::collections::VecDeque::with_capacity(capacity),
            capacity,
            oldest_cursor: 0,
            latest_cursor: 0,
        }
    }

    pub fn push(
        &mut self,
        cursor: u64,
        payload: String,
        pool_id: Option<u64>,
        user_address: Option<String>,
    ) {
        if self.events.is_empty() {
            self.oldest_cursor = cursor;
        }
        self.latest_cursor = cursor;
        self.events.push_back(SequencedEvent {
            cursor,
            payload,
            pool_id,
            user_address,
        });
        if self.events.len() > self.capacity {
            self.events.pop_front();
            if let Some(front) = self.events.front() {
                self.oldest_cursor = front.cursor;
            }
        }
    }

    pub fn get_replay(
        &self,
        last_seen_cursor: u64,
        wallet_filter: Option<&str>,
        pool_filter: Option<u64>,
        max_limit: usize,
    ) -> ReplayResult {
        if self.events.is_empty() {
            if last_seen_cursor > 0 {
                return ReplayResult::CursorAhead { latest_cursor: 0 };
            }
            return ReplayResult::Empty;
        }
        if last_seen_cursor > self.latest_cursor {
            return ReplayResult::CursorAhead {
                latest_cursor: self.latest_cursor,
            };
        }
        if last_seen_cursor < self.oldest_cursor.saturating_sub(1) {
            return ReplayResult::CursorExpired {
                oldest_cursor: self.oldest_cursor,
                latest_cursor: self.latest_cursor,
            };
        }
        let mut matched = Vec::new();
        let mut truncated = false;
        let mut next_cursor = None;
        for ev in &self.events {
            if ev.cursor <= last_seen_cursor {
                continue;
            }
            let matches_wallet =
                wallet_filter.map_or(true, |w| ev.user_address.as_deref() == Some(w));
            let matches_pool = pool_filter.map_or(true, |p| ev.pool_id == Some(p));
            if matches_wallet && matches_pool {
                if matched.len() >= max_limit {
                    truncated = true;
                    next_cursor = Some(ev.cursor);
                    break;
                }
                matched.push(ev.clone());
            }
        }
        if matched.is_empty() && !truncated {
            ReplayResult::Empty
        } else {
            ReplayResult::Events {
                events: matched,
                truncated,
                next_cursor,
                latest_cursor: self.latest_cursor,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ConnectionGuard
// ---------------------------------------------------------------------------

/// RAII guard to safely track and decrement active connections on drop or panic.
pub struct ConnectionGuard(Arc<AtomicUsize>);

impl ConnectionGuard {
    pub fn new(active_connections: Arc<AtomicUsize>) -> Self {
        let count = active_connections.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::info!(active_connections = count, "websocket client connected");
        Self(active_connections)
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        let count = self.0.fetch_sub(1, Ordering::Relaxed) - 1;
        tracing::info!(active_connections = count, "websocket client disconnected");
    }
}

// ---------------------------------------------------------------------------
// ConnectRateLimiter
// ---------------------------------------------------------------------------

/// Sliding-window limiter for WebSocket handshake attempts per client IP.
struct ConnectRateLimiter {
    attempts: Mutex<HashMap<String, Vec<Instant>>>,
}

impl ConnectRateLimiter {
    fn new() -> Self {
        Self {
            attempts: Mutex::new(HashMap::new()),
        }
    }

    /// Returns `true` if this IP is allowed another handshake.
    fn check(&self, ip: &str) -> bool {
        let now = Instant::now();
        let mut map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 10_000 {
            map.retain(|_, stamps| stamps.iter().any(|t| now.duration_since(*t) < CONNECT_WINDOW));
        }
        let entry = map.entry(ip.to_string()).or_default();
        entry.retain(|t| now.duration_since(*t) < CONNECT_WINDOW);
        if entry.len() >= MAX_CONNECT_ATTEMPTS_PER_IP {
            return false;
        }
        entry.push(now);
        true
    }
}

// ---------------------------------------------------------------------------
// WsConnectParams
// ---------------------------------------------------------------------------

/// Optional query parameters for the WebSocket endpoint.
#[derive(Debug, Deserialize, Default)]
pub struct WsConnectParams {
    /// When set, only events whose `user_address` equals this value are forwarded.
    pub address: Option<String>,
    /// Optional JWT passed as a query parameter when headers are unavailable.
    /// Ignored in production — tokens in URLs leak into access logs and history.
    pub token: Option<String>,
    /// When set, only events whose `pool_id` equals this value are forwarded.
    pub pool_id: Option<u64>,
    /// Resume from this cursor (canonical name).
    pub cursor: Option<u64>,
    /// Alias for `cursor` — the last cursor the client successfully received.
    pub last_seen_cursor: Option<u64>,
}

// ---------------------------------------------------------------------------
// EventBus
// ---------------------------------------------------------------------------

/// Shareable handle to the broadcast channel with cursor-stamped replay support.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<String>,
    /// Number of currently connected WebSocket clients.
    active_connections: Arc<AtomicUsize>,
    connect_limiter: Arc<ConnectRateLimiter>,
    next_cursor: Arc<std::sync::atomic::AtomicU64>,
    replay_buffer: Arc<std::sync::RwLock<ReplayBuffer>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    /// Create a new bus with [`DEFAULT_RETENTION_CAPACITY`] replay capacity.
    pub fn new() -> Self {
        Self::with_retention(DEFAULT_RETENTION_CAPACITY)
    }

    /// Create a new bus with a custom replay retention capacity.
    pub fn with_retention(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            tx,
            active_connections: Arc::new(AtomicUsize::new(0)),
            connect_limiter: Arc::new(ConnectRateLimiter::new()),
            next_cursor: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            replay_buffer: Arc::new(std::sync::RwLock::new(ReplayBuffer::new(capacity))),
        }
    }

    /// Number of WebSocket clients currently connected.
    pub fn active_connections(&self) -> usize {
        self.active_connections.load(Ordering::Relaxed)
    }

    /// The cursor that will be assigned to the *next* published event.
    pub fn latest_cursor(&self) -> u64 {
        self.next_cursor.load(Ordering::SeqCst).saturating_sub(1)
    }

    /// The oldest cursor still held in the replay buffer (0 if empty).
    pub fn oldest_retained_cursor(&self) -> u64 {
        self.replay_buffer
            .read()
            .unwrap()
            .oldest_cursor
    }

    /// Publish a serialisable event to all connected WebSocket clients.
    ///
    /// The event is stamped with a monotonic cursor, stored in the replay
    /// buffer, and broadcast over the channel.  Silently drops the message if
    /// there are no subscribers (the replay buffer always retains it).
    pub fn send<T: Serialize>(&self, event: &T) {
        let Ok(mut value) = serde_json::to_value(event) else {
            return;
        };

        let cursor = self.next_cursor.fetch_add(1, Ordering::SeqCst);

        let pool_id = value
            .get("pool_id")
            .and_then(|v| v.as_u64());

        let user_address = value
            .get("user_address")
            .or_else(|| value.get("user"))
            .and_then(|v| v.as_str())
            .map(str::to_string);

        if let Some(obj) = value.as_object_mut() {
            obj.insert("cursor".to_string(), serde_json::json!(cursor));
        }

        let Ok(json) = serde_json::to_string(&value) else {
            return;
        };

        self.replay_buffer
            .write()
            .unwrap()
            .push(cursor, json.clone(), pool_id, user_address);

        let _ = self.tx.send(json);
    }

    /// Subscribe to the broadcast channel.
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    fn allow_connect(&self, ip: &str) -> bool {
        self.connect_limiter.check(ip)
    }
}

// ---------------------------------------------------------------------------
// should_deliver_event
// ---------------------------------------------------------------------------

/// Returns `true` when `json` should be delivered to a subscriber with `wallet_filter` and `pool_filter`.
pub fn should_deliver_event(
    json: &str,
    wallet_filter: Option<&str>,
    pool_filter: Option<u64>,
) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return false;
    };

    if let Some(wallet) = wallet_filter {
        if value.get("user_address").and_then(|v| v.as_str()) != Some(wallet) {
            return false;
        }
    }

    if let Some(pool_id) = pool_filter {
        if value.get("pool_id").and_then(|v| v.as_u64()) != Some(pool_id) {
            return false;
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Helpers: IP, token, origin
// ---------------------------------------------------------------------------

fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn extract_ws_token(
    headers: &HeaderMap,
    params: &WsConnectParams,
    production: bool,
) -> Option<String> {
    let header_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(extract_bearer_token)
        .map(str::to_string);

    if production {
        return header_token;
    }

    header_token.or_else(|| params.token.clone())
}

/// Validate the Origin header to prevent CSRF-like WebSocket hijacking.
fn validate_origin(headers: &HeaderMap, config: &Config) -> bool {
    if config.allowed_ws_origins.is_empty() {
        return !config.is_production();
    }

    let origin = match headers.get("origin") {
        Some(value) => match value.to_str() {
            Ok(origin_str) => origin_str,
            Err(_) => {
                tracing::warn!("Invalid Origin header format");
                return false;
            }
        },
        None => {
            tracing::warn!("Missing Origin header in WebSocket upgrade request");
            return false;
        }
    };

    if origin.eq_ignore_ascii_case("null") {
        return false;
    }

    config.allowed_ws_origins.iter().any(|allowed| allowed == origin)
}

fn unauthorized_response(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({
            "error": message,
        })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Gap frame helpers
// ---------------------------------------------------------------------------

fn gap_cursor_expired(oldest_cursor: u64, latest_cursor: u64) -> String {
    serde_json::json!({
        "type": "gap",
        "code": "CURSOR_EXPIRED",
        "message": "Requested cursor is older than retention window. Full resync required.",
        "oldest_cursor": oldest_cursor,
        "latest_cursor": latest_cursor,
    })
    .to_string()
}

fn gap_cursor_ahead(latest_cursor: u64) -> String {
    serde_json::json!({
        "type": "gap",
        "code": "CURSOR_AHEAD",
        "message": "Requested cursor is ahead of latest server cursor",
        "latest_cursor": latest_cursor,
    })
    .to_string()
}

fn gap_replay_truncated(next_cursor: u64, latest_cursor: u64) -> String {
    serde_json::json!({
        "type": "gap",
        "code": "REPLAY_TRUNCATED",
        "message": "Replay capped at max replay size. Resync remaining state via REST.",
        "next_cursor": next_cursor,
        "latest_cursor": latest_cursor,
    })
    .to_string()
}

fn gap_slow_client_lagged(missed_count: u64) -> String {
    serde_json::json!({
        "type": "gap",
        "code": "SLOW_CLIENT_LAGGED",
        "message": "Receiver fell behind and dropped events",
        "missed_count": missed_count,
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// ws_handler
// ---------------------------------------------------------------------------

/// Axum handler — validates JWT credentials, then upgrades to WebSocket.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(params): Query<WsConnectParams>,
    State(config): State<Arc<Config>>,
    State(bus): State<EventBus>,
) -> impl IntoResponse {
    if !validate_origin(&headers, &config) {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({
                "error": "invalid or missing Origin header",
            })),
        )
            .into_response();
    }

    let ip = client_ip(&headers);
    if !bus.allow_connect(&ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(serde_json::json!({
                "error": "websocket connection rate limit exceeded",
            })),
        )
            .into_response();
    }

    if bus.active_connections() >= MAX_ACTIVE_CONNECTIONS {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(serde_json::json!({
                "error": "maximum active websocket connections reached",
            })),
        )
            .into_response();
    }

    let Some(token) = extract_ws_token(&headers, &params, config.is_production()) else {
        return unauthorized_response("missing or invalid authorization token");
    };

    let claims = match verify_jwt_token_strict(
        &token,
        &config.secret_key,
        "access",
        config.jwt_key_version,
    ) {
        Ok(claims) => claims,
        Err(error) => return unauthorized_response(&error.to_string()),
    };

    if let Some(ref target_address) = params.address {
        if target_address != &claims.sub {
            return (
                StatusCode::FORBIDDEN,
                axum::Json(serde_json::json!({
                    "error": "unauthorized wallet subscription",
                })),
            )
                .into_response();
        }
    }

    let wallet_filter = params.address;
    let pool_filter = params.pool_id;
    let requested_cursor = params.cursor.or(params.last_seen_cursor);

    let span = info_span!(
        "websocket.connect",
        wallet = ?wallet_filter,
        pool_id = ?pool_filter,
        subject = %claims.sub,
        requested_cursor = ?requested_cursor,
    );
    ws.on_upgrade(move |socket| {
        handle_socket(socket, bus, wallet_filter, pool_filter, requested_cursor).instrument(span)
    })
}

// ---------------------------------------------------------------------------
// handle_socket
// ---------------------------------------------------------------------------

async fn handle_socket(
    mut socket: WebSocket,
    bus: EventBus,
    wallet_filter: Option<String>,
    pool_filter: Option<u64>,
    requested_cursor: Option<u64>,
) {
    // Subscribe before snapshotting the buffer to avoid a race where events
    // published between snapshot and subscribe are missed.
    let mut rx = bus.subscribe();
    let _guard = ConnectionGuard::new(bus.active_connections.clone());

    // Replay missed events if the client supplied a cursor.
    let last_sent_cursor = if let Some(cursor) = requested_cursor {
        let replay = bus
            .replay_buffer
            .read()
            .unwrap()
            .get_replay(cursor, wallet_filter.as_deref(), pool_filter, MAX_REPLAY_SIZE);

        match replay {
            ReplayResult::Empty => cursor,

            ReplayResult::CursorExpired {
                oldest_cursor,
                latest_cursor,
            } => {
                let frame = gap_cursor_expired(oldest_cursor, latest_cursor);
                if socket.send(Message::Text(frame)).await.is_err() {
                    return;
                }
                cursor
            }

            ReplayResult::CursorAhead { latest_cursor } => {
                let frame = gap_cursor_ahead(latest_cursor);
                if socket.send(Message::Text(frame)).await.is_err() {
                    return;
                }
                cursor
            }

            ReplayResult::Events {
                events,
                truncated,
                next_cursor,
                latest_cursor,
            } => {
                let mut highest = cursor;
                for ev in &events {
                    if socket
                        .send(Message::Text(ev.payload.clone()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    if ev.cursor > highest {
                        highest = ev.cursor;
                    }
                }
                if truncated {
                    let nc = next_cursor.unwrap_or(latest_cursor);
                    let frame = gap_replay_truncated(nc, latest_cursor);
                    if socket.send(Message::Text(frame)).await.is_err() {
                        return;
                    }
                }
                highest
            }
        }
    } else {
        0
    };

    run_socket(
        &mut socket,
        &mut rx,
        wallet_filter.as_deref(),
        pool_filter,
        last_sent_cursor,
    )
    .await;

    let _ = socket.send(Message::Close(None)).await;
}

// ---------------------------------------------------------------------------
// WsRateLimiter
// ---------------------------------------------------------------------------

/// Per-connection message rate limiter using a sliding window.
struct WsRateLimiter {
    window_size: std::time::Duration,
    max_messages: u32,
    timestamps: Vec<Instant>,
}

impl WsRateLimiter {
    fn new(max_messages: u32, window_secs: u64) -> Self {
        Self {
            window_size: std::time::Duration::from_secs(window_secs),
            max_messages,
            timestamps: Vec::with_capacity(max_messages as usize + 1),
        }
    }

    /// Returns `true` if the message is allowed, `false` if rate-limited.
    fn check(&mut self) -> bool {
        let now = Instant::now();
        self.timestamps
            .retain(|t| now.duration_since(*t) < self.window_size);
        if self.timestamps.len() >= self.max_messages as usize {
            return false;
        }
        self.timestamps.push(now);
        true
    }
}

// ---------------------------------------------------------------------------
// run_socket
// ---------------------------------------------------------------------------

async fn run_socket(
    socket: &mut WebSocket,
    rx: &mut broadcast::Receiver<String>,
    wallet_filter: Option<&str>,
    pool_filter: Option<u64>,
    initial_last_sent_cursor: u64,
) {
    let mut rate_limiter = WsRateLimiter::new(RATE_LIMIT_WS_BURST, RATE_LIMIT_WS_PERIOD_SECS);
    let mut last_sent_cursor = initial_last_sent_cursor;

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(msg) => {
                        // Parse the cursor from the payload for duplicate suppression.
                        let msg_cursor = serde_json::from_str::<serde_json::Value>(&msg)
                            .ok()
                            .and_then(|v| v.get("cursor").and_then(|c| c.as_u64()))
                            .unwrap_or(0);

                        // Duplicate suppression: skip events already sent via replay.
                        if msg_cursor > 0 && msg_cursor <= last_sent_cursor {
                            continue;
                        }

                        if !should_deliver_event(&msg, wallet_filter, pool_filter) {
                            continue;
                        }
                        if msg.len() > MAX_MESSAGE_SIZE {
                            tracing::warn!(
                                message_size = msg.len(),
                                max_size = MAX_MESSAGE_SIZE,
                                "WebSocket message exceeds size limit, dropping"
                            );
                            continue;
                        }
                        if socket.send(Message::Text(msg)).await.is_err() {
                            break;
                        }
                        if msg_cursor > last_sent_cursor {
                            last_sent_cursor = msg_cursor;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(missed)) => {
                        let frame = gap_slow_client_lagged(missed);
                        if socket.send(Message::Text(frame)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = socket.recv() => {
                let msg = match msg {
                    Some(Ok(m)) => m,
                    _ => break,
                };

                match &msg {
                    Message::Text(text) => {
                        if text.len() > MAX_MESSAGE_SIZE {
                            tracing::warn!(
                                message_size = text.len(),
                                max_size = MAX_MESSAGE_SIZE,
                                "Incoming WebSocket text message exceeds size limit, closing connection"
                            );
                            let _ = socket.send(Message::Close(None)).await;
                            break;
                        }
                    }
                    Message::Binary(data) => {
                        if data.len() > MAX_MESSAGE_SIZE {
                            tracing::warn!(
                                message_size = data.len(),
                                max_size = MAX_MESSAGE_SIZE,
                                "Incoming WebSocket binary message exceeds size limit, closing connection"
                            );
                            let _ = socket.send(Message::Close(None)).await;
                            break;
                        }
                    }
                    Message::Ping(payload) => {
                        let _ = socket.send(Message::Pong(payload.clone())).await;
                        continue;
                    }
                    Message::Pong(_) => continue,
                    Message::Close(_) => break,
                }

                if !rate_limiter.check() {
                    tracing::warn!("WebSocket connection rate limited");
                    let _ = socket.send(Message::Close(None)).await;
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jwt::{sign_jwt_for_test, sign_jwt_with_type, verify_jwt_token};

    // ── existing delivery / auth tests ───────────────────────────────────────

    #[test]
    fn delivers_all_events_when_no_wallet_filter() {
        let json = r#"{"type":"prediction_placed","user_address":"GABC","pool_id":1}"#;
        assert!(should_deliver_event(json, None, None));
    }

    #[test]
    fn delivers_event_when_wallet_matches() {
        let json = r#"{"type":"prediction_placed","user_address":"GABC","pool_id":1}"#;
        assert!(should_deliver_event(json, Some("GABC"), None));
    }

    #[test]
    fn skips_event_when_wallet_mismatch() {
        let json = r#"{"type":"prediction_placed","user_address":"GABC","pool_id":1}"#;
        assert!(!should_deliver_event(json, Some("GXYZ"), None));
    }

    #[test]
    fn skips_malformed_json_when_filter_active() {
        assert!(!should_deliver_event("not-json", Some("GABC"), None));
    }

    #[test]
    fn skips_event_missing_user_address_when_filter_active() {
        let json = r#"{"type":"prediction_placed","pool_id":1}"#;
        assert!(!should_deliver_event(json, Some("GABC"), None));
    }

    #[test]
    fn ws_token_from_query_param_verifies_against_secret() {
        let secret = "predifi-dev-secret-do-not-use-in-production-32";
        let token = sign_jwt_for_test("GABC123", secret);
        let claims = verify_jwt_token(&token, secret).expect("valid token");
        assert_eq!(claims.sub, "GABC123");
    }

    #[test]
    fn connection_guard_tracks_and_decrements_atomic_counter() {
        let counter = Arc::new(AtomicUsize::new(0));
        assert_eq!(counter.load(Ordering::Relaxed), 0);

        {
            let _guard = ConnectionGuard::new(counter.clone());
            assert_eq!(counter.load(Ordering::Relaxed), 1);
        }

        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn extract_ws_token_prefers_authorization_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer header-jwt-token".parse().unwrap(),
        );

        let params = WsConnectParams {
            token: Some("query-param-token".to_string()),
            ..Default::default()
        };

        let token = extract_ws_token(&headers, &params, false);
        assert_eq!(token, Some("header-jwt-token".to_string()));
    }

    #[test]
    fn extract_ws_token_ignores_query_param_in_production() {
        let headers = HeaderMap::new();
        let params = WsConnectParams {
            token: Some("query-param-token".to_string()),
            ..Default::default()
        };
        assert!(extract_ws_token(&headers, &params, true).is_none());
    }

    #[test]
    fn validate_origin_allows_matching_origins_and_blocks_disallowed() {
        let mut config = Config::default_for_test();
        config.allowed_ws_origins = vec!["https://app.predifi.com".to_string()];

        let mut valid_headers = HeaderMap::new();
        valid_headers.insert("origin", "https://app.predifi.com".parse().unwrap());
        assert!(validate_origin(&valid_headers, &config));

        let mut invalid_headers = HeaderMap::new();
        invalid_headers.insert("origin", "https://malicious.com".parse().unwrap());
        assert!(!validate_origin(&invalid_headers, &config));

        let empty_headers = HeaderMap::new();
        assert!(!validate_origin(&empty_headers, &config));

        let mut null_origin = HeaderMap::new();
        null_origin.insert("origin", "null".parse().unwrap());
        assert!(!validate_origin(&null_origin, &config));
    }

    #[test]
    fn validate_origin_fails_closed_in_production_without_allow_list() {
        let mut config = Config::default_for_test();
        config.app_env = "production".to_string();
        config.allowed_ws_origins = Vec::new();
        let mut headers = HeaderMap::new();
        headers.insert("origin", "https://app.predifi.com".parse().unwrap());
        assert!(!validate_origin(&headers, &config));
    }

    #[test]
    fn connect_rate_limiter_blocks_after_burst() {
        let limiter = ConnectRateLimiter::new();
        for _ in 0..MAX_CONNECT_ATTEMPTS_PER_IP {
            assert!(limiter.check("1.2.3.4"));
        }
        assert!(!limiter.check("1.2.3.4"));
        assert!(limiter.check("5.6.7.8"), "other IPs are independent");
    }

    #[test]
    fn refresh_token_is_rejected_for_websocket_access() {
        let secret = "predifi-dev-secret-do-not-use-in-production-32";
        let token = sign_jwt_with_type("GABC123", secret, 1_800_000_000, "refresh", 0).unwrap();
        let error = verify_jwt_token_strict(&token, secret, "access", 0).unwrap_err();
        assert_eq!(error, crate::jwt::JwtVerifyError::WrongTokenType);
    }

    #[test]
    fn client_ip_prefers_forwarded_for() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "10.0.0.1, 10.0.0.2".parse().unwrap());
        headers.insert("x-real-ip", "9.9.9.9".parse().unwrap());
        assert_eq!(client_ip(&headers), "10.0.0.1");
    }

    // ── ReplayBuffer unit tests ───────────────────────────────────────────────

    fn make_event(cursor: u64, pool_id: u64, addr: &str) -> (u64, String, Option<u64>, Option<String>) {
        let payload = serde_json::json!({
            "cursor": cursor,
            "pool_id": pool_id,
            "user_address": addr,
        })
        .to_string();
        (cursor, payload, Some(pool_id), Some(addr.to_string()))
    }

    fn filled_buffer(n: usize) -> ReplayBuffer {
        let mut buf = ReplayBuffer::new(n + 10);
        for i in 1..=(n as u64) {
            let (c, p, pool, addr) = make_event(i, i, "GABC");
            buf.push(c, p, pool, addr);
        }
        buf
    }

    #[test]
    fn replay_buffer_empty_on_creation() {
        let buf = ReplayBuffer::new(100);
        assert_eq!(buf.oldest_cursor, 0);
        assert_eq!(buf.latest_cursor, 0);
        assert!(buf.events.is_empty());
    }

    #[test]
    fn replay_buffer_push_updates_cursors() {
        let mut buf = ReplayBuffer::new(10);
        let (c1, p1, pool1, addr1) = make_event(1, 1, "GABC");
        buf.push(c1, p1, pool1, addr1);
        assert_eq!(buf.oldest_cursor, 1);
        assert_eq!(buf.latest_cursor, 1);

        let (c2, p2, pool2, addr2) = make_event(2, 1, "GABC");
        buf.push(c2, p2, pool2, addr2);
        assert_eq!(buf.oldest_cursor, 1);
        assert_eq!(buf.latest_cursor, 2);
    }

    #[test]
    fn replay_buffer_evicts_oldest_when_full() {
        let mut buf = ReplayBuffer::new(3);
        for i in 1u64..=4 {
            let (c, p, pool, addr) = make_event(i, i, "G");
            buf.push(c, p, pool, addr);
        }
        assert_eq!(buf.events.len(), 3);
        assert_eq!(buf.oldest_cursor, 2);
        assert_eq!(buf.latest_cursor, 4);
    }

    #[test]
    fn replay_buffer_get_replay_empty_buffer_zero_cursor_returns_empty() {
        let buf = ReplayBuffer::new(10);
        assert!(matches!(
            buf.get_replay(0, None, None, MAX_REPLAY_SIZE),
            ReplayResult::Empty
        ));
    }

    #[test]
    fn replay_buffer_get_replay_empty_buffer_nonzero_cursor_returns_ahead() {
        let buf = ReplayBuffer::new(10);
        assert!(matches!(
            buf.get_replay(5, None, None, MAX_REPLAY_SIZE),
            ReplayResult::CursorAhead { latest_cursor: 0 }
        ));
    }

    #[test]
    fn replay_buffer_returns_events_after_cursor() {
        let buf = filled_buffer(5);
        match buf.get_replay(2, None, None, MAX_REPLAY_SIZE) {
            ReplayResult::Events { events, truncated, .. } => {
                assert!(!truncated);
                assert_eq!(events.len(), 3);
                assert_eq!(events[0].cursor, 3);
                assert_eq!(events[2].cursor, 5);
            }
            other => panic!("expected Events, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn replay_buffer_returns_empty_when_already_current() {
        let buf = filled_buffer(5);
        assert!(matches!(
            buf.get_replay(5, None, None, MAX_REPLAY_SIZE),
            ReplayResult::Empty
        ));
    }

    #[test]
    fn replay_buffer_cursor_ahead_of_latest() {
        let buf = filled_buffer(5);
        assert!(matches!(
            buf.get_replay(99, None, None, MAX_REPLAY_SIZE),
            ReplayResult::CursorAhead { latest_cursor: 5 }
        ));
    }

    #[test]
    fn replay_buffer_cursor_expired() {
        let mut buf = ReplayBuffer::new(3);
        for i in 10u64..=12 {
            let (c, p, pool, addr) = make_event(i, i, "G");
            buf.push(c, p, pool, addr);
        }
        // oldest_cursor is 10; last_seen=5 which is < 10-1=9 → expired
        match buf.get_replay(5, None, None, MAX_REPLAY_SIZE) {
            ReplayResult::CursorExpired { oldest_cursor, latest_cursor } => {
                assert_eq!(oldest_cursor, 10);
                assert_eq!(latest_cursor, 12);
            }
            other => panic!("expected CursorExpired, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn replay_buffer_cursor_one_before_oldest_is_not_expired() {
        let mut buf = ReplayBuffer::new(3);
        for i in 10u64..=12 {
            let (c, p, pool, addr) = make_event(i, i, "G");
            buf.push(c, p, pool, addr);
        }
        // oldest_cursor is 10; last_seen=9 which equals oldest_cursor-1 → not expired
        match buf.get_replay(9, None, None, MAX_REPLAY_SIZE) {
            ReplayResult::Events { events, .. } => {
                assert_eq!(events.len(), 3);
            }
            other => panic!("expected Events, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn replay_buffer_truncates_at_max_limit() {
        let buf = filled_buffer(10);
        match buf.get_replay(0, None, None, 3) {
            ReplayResult::Events { events, truncated, next_cursor, .. } => {
                assert_eq!(events.len(), 3);
                assert!(truncated);
                assert_eq!(next_cursor, Some(4));
            }
            other => panic!("expected Events, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn replay_buffer_wallet_filter_excludes_other_addresses() {
        let mut buf = ReplayBuffer::new(10);
        for i in 1u64..=6 {
            let addr = if i % 2 == 0 { "GABC" } else { "GXYZ" };
            let (c, p, pool, a) = make_event(i, 1, addr);
            buf.push(c, p, pool, a);
        }
        match buf.get_replay(0, Some("GABC"), None, MAX_REPLAY_SIZE) {
            ReplayResult::Events { events, .. } => {
                assert_eq!(events.len(), 3);
                for ev in &events {
                    assert_eq!(ev.user_address.as_deref(), Some("GABC"));
                }
            }
            other => panic!("expected Events, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn replay_buffer_pool_filter_excludes_other_pools() {
        let mut buf = ReplayBuffer::new(10);
        for i in 1u64..=6 {
            let pool = if i % 2 == 0 { 42 } else { 99 };
            let (c, p, _, a) = make_event(i, pool, "G");
            buf.push(c, p, Some(pool), a);
        }
        match buf.get_replay(0, None, Some(42), MAX_REPLAY_SIZE) {
            ReplayResult::Events { events, .. } => {
                assert_eq!(events.len(), 3);
                for ev in &events {
                    assert_eq!(ev.pool_id, Some(42));
                }
            }
            other => panic!("expected Events, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn replay_buffer_both_filters_applied_together() {
        let mut buf = ReplayBuffer::new(20);
        for i in 1u64..=8 {
            let addr = if i % 2 == 0 { "GABC" } else { "GXYZ" };
            let pool = if i <= 4 { 1 } else { 2 };
            let (c, p, _, a) = make_event(i, pool, addr);
            buf.push(c, p, Some(pool), a);
        }
        // Only GABC events in pool 1: cursors 2, 4
        match buf.get_replay(0, Some("GABC"), Some(1), MAX_REPLAY_SIZE) {
            ReplayResult::Events { events, .. } => {
                assert_eq!(events.len(), 2);
            }
            other => panic!("expected Events, got {:?}", std::mem::discriminant(&other)),
        }
    }

    // ── EventBus cursor & replay tests ──────────────────────────────────────

    #[test]
    fn event_bus_cursors_are_strictly_monotonic() {
        let bus = EventBus::new();
        assert_eq!(bus.latest_cursor(), 0); // nothing sent yet

        #[derive(Serialize)]
        struct Ev {
            v: u32,
        }

        bus.send(&Ev { v: 1 });
        bus.send(&Ev { v: 2 });
        bus.send(&Ev { v: 3 });

        let buf = bus.replay_buffer.read().unwrap();
        let cursors: Vec<u64> = buf.events.iter().map(|e| e.cursor).collect();
        assert_eq!(cursors, vec![1, 2, 3]);
    }

    #[test]
    fn event_bus_send_injects_cursor_into_json() {
        let bus = EventBus::new();
        #[derive(Serialize)]
        struct Ev {
            kind: &'static str,
        }
        bus.send(&Ev { kind: "test" });
        let buf = bus.replay_buffer.read().unwrap();
        let ev = &buf.events[0];
        let val: serde_json::Value = serde_json::from_str(&ev.payload).unwrap();
        assert_eq!(val["cursor"].as_u64(), Some(1));
        assert_eq!(val["kind"].as_str(), Some("test"));
    }

    #[test]
    fn event_bus_send_extracts_pool_id_and_user_address() {
        let bus = EventBus::new();
        #[derive(Serialize)]
        struct Ev {
            pool_id: u64,
            user_address: String,
        }
        bus.send(&Ev {
            pool_id: 42,
            user_address: "GABC".to_string(),
        });
        let buf = bus.replay_buffer.read().unwrap();
        let ev = &buf.events[0];
        assert_eq!(ev.pool_id, Some(42));
        assert_eq!(ev.user_address.as_deref(), Some("GABC"));
    }

    #[test]
    fn event_bus_send_extracts_user_field_as_fallback() {
        let bus = EventBus::new();
        #[derive(Serialize)]
        struct Ev {
            user: String,
        }
        bus.send(&Ev {
            user: "GFALLBACK".to_string(),
        });
        let buf = bus.replay_buffer.read().unwrap();
        let ev = &buf.events[0];
        assert_eq!(ev.user_address.as_deref(), Some("GFALLBACK"));
    }

    #[test]
    fn event_bus_with_retention_sets_custom_capacity() {
        let bus = EventBus::with_retention(10);
        #[derive(Serialize)]
        struct Ev {
            v: u32,
        }
        for i in 0..15u32 {
            bus.send(&Ev { v: i });
        }
        let buf = bus.replay_buffer.read().unwrap();
        assert_eq!(buf.events.len(), 10);
    }

    #[test]
    fn event_bus_latest_cursor_reflects_last_send() {
        let bus = EventBus::new();
        #[derive(Serialize)]
        struct Ev {
            v: u32,
        }
        bus.send(&Ev { v: 1 });
        assert_eq!(bus.latest_cursor(), 1);
        bus.send(&Ev { v: 2 });
        assert_eq!(bus.latest_cursor(), 2);
    }

    #[test]
    fn event_bus_oldest_retained_cursor_tracks_eviction() {
        let bus = EventBus::with_retention(3);
        #[derive(Serialize)]
        struct Ev {
            v: u32,
        }
        bus.send(&Ev { v: 1 });
        bus.send(&Ev { v: 2 });
        bus.send(&Ev { v: 3 });
        assert_eq!(bus.oldest_retained_cursor(), 1);
        bus.send(&Ev { v: 4 }); // evicts cursor 1
        assert_eq!(bus.oldest_retained_cursor(), 2);
    }

    // ── Gap frame JSON shape tests ───────────────────────────────────────────

    #[test]
    fn gap_cursor_expired_frame_has_correct_shape() {
        let frame = gap_cursor_expired(10, 99);
        let val: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(val["type"].as_str(), Some("gap"));
        assert_eq!(val["code"].as_str(), Some("CURSOR_EXPIRED"));
        assert_eq!(val["oldest_cursor"].as_u64(), Some(10));
        assert_eq!(val["latest_cursor"].as_u64(), Some(99));
    }

    #[test]
    fn gap_cursor_ahead_frame_has_correct_shape() {
        let frame = gap_cursor_ahead(42);
        let val: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(val["type"].as_str(), Some("gap"));
        assert_eq!(val["code"].as_str(), Some("CURSOR_AHEAD"));
        assert_eq!(val["latest_cursor"].as_u64(), Some(42));
    }

    #[test]
    fn gap_replay_truncated_frame_has_correct_shape() {
        let frame = gap_replay_truncated(251, 999);
        let val: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(val["type"].as_str(), Some("gap"));
        assert_eq!(val["code"].as_str(), Some("REPLAY_TRUNCATED"));
        assert_eq!(val["next_cursor"].as_u64(), Some(251));
        assert_eq!(val["latest_cursor"].as_u64(), Some(999));
    }

    #[test]
    fn gap_slow_client_lagged_frame_has_correct_shape() {
        let frame = gap_slow_client_lagged(7);
        let val: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(val["type"].as_str(), Some("gap"));
        assert_eq!(val["code"].as_str(), Some("SLOW_CLIENT_LAGGED"));
        assert_eq!(val["missed_count"].as_u64(), Some(7));
    }

    // ── WsConnectParams cursor fields ────────────────────────────────────────

    #[test]
    fn ws_connect_params_defaults_cursors_to_none() {
        let params = WsConnectParams::default();
        assert!(params.cursor.is_none());
        assert!(params.last_seen_cursor.is_none());
    }

    #[test]
    fn requested_cursor_prefers_cursor_over_last_seen() {
        let params = WsConnectParams {
            cursor: Some(10),
            last_seen_cursor: Some(5),
            ..Default::default()
        };
        let requested = params.cursor.or(params.last_seen_cursor);
        assert_eq!(requested, Some(10));
    }

    #[test]
    fn requested_cursor_falls_back_to_last_seen() {
        let params = WsConnectParams {
            cursor: None,
            last_seen_cursor: Some(7),
            ..Default::default()
        };
        let requested = params.cursor.or(params.last_seen_cursor);
        assert_eq!(requested, Some(7));
    }

    // ── Deterministic stream scenarios ──────────────────────────────────────

    #[test]
    fn replay_then_live_no_duplicate_at_boundary() {
        // Simulate: replayed up to cursor 5; live broadcast sends cursor 5 again.
        // Duplicate suppression must skip cursor 5.
        let bus = EventBus::with_retention(20);
        #[derive(Serialize)]
        struct Ev {
            pool_id: u64,
            user_address: &'static str,
        }

        for _ in 0..5 {
            bus.send(&Ev { pool_id: 1, user_address: "GABC" });
        }

        // Replaying from cursor 0 gives all 5 events; last_sent_cursor becomes 5.
        let result = bus
            .replay_buffer
            .read()
            .unwrap()
            .get_replay(0, None, None, MAX_REPLAY_SIZE);

        let highest = match result {
            ReplayResult::Events { ref events, .. } => events.last().map(|e| e.cursor).unwrap_or(0),
            _ => panic!("expected Events"),
        };
        assert_eq!(highest, 5);

        // Cursor 5 from live broadcast should be suppressed (msg_cursor <= last_sent_cursor).
        let live_msg_cursor = 5u64;
        assert!(live_msg_cursor <= highest, "duplicate should be suppressed");

        // Cursor 6 from live broadcast should be delivered.
        let live_msg_cursor_new = 6u64;
        assert!(live_msg_cursor_new > highest, "new event should be delivered");
    }

    #[test]
    fn constants_have_expected_values() {
        assert_eq!(DEFAULT_RETENTION_CAPACITY, 5_000);
        assert_eq!(MAX_REPLAY_SIZE, 250);
    }

    #[test]
    fn replay_buffer_zero_cursor_returns_all_events() {
        let buf = filled_buffer(5);
        match buf.get_replay(0, None, None, MAX_REPLAY_SIZE) {
            ReplayResult::Events { events, truncated, .. } => {
                assert_eq!(events.len(), 5);
                assert!(!truncated);
            }
            other => panic!("expected Events, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn replay_buffer_filters_do_not_cause_false_truncation() {
        // 10 events, only 2 match the filter; max_limit = 3 → no truncation
        let mut buf = ReplayBuffer::new(20);
        for i in 1u64..=10 {
            let addr = if i <= 2 { "GABC" } else { "GOTHER" };
            let (c, p, pool, a) = make_event(i, 1, addr);
            buf.push(c, p, pool, a);
        }
        match buf.get_replay(0, Some("GABC"), None, 3) {
            ReplayResult::Events { events, truncated, .. } => {
                assert_eq!(events.len(), 2);
                assert!(!truncated, "only 2 match the filter, truncation should not trigger");
            }
            other => panic!("expected Events, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn replay_buffer_next_cursor_points_to_first_excluded_event() {
        let buf = filled_buffer(10);
        match buf.get_replay(0, None, None, 4) {
            ReplayResult::Events { next_cursor, .. } => {
                assert_eq!(next_cursor, Some(5));
            }
            other => panic!("expected Events, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn disconnect_and_reconnect_replays_missed_events() {
        let bus = EventBus::with_retention(50);
        #[derive(Serialize)]
        struct Ev {
            pool_id: u64,
            user_address: &'static str,
            amount: u64,
        }

        for i in 1..=3 {
            bus.send(&Ev { pool_id: 1, user_address: "GABC", amount: i * 10 });
        }

        let last_seen_cursor = bus.latest_cursor();
        assert_eq!(last_seen_cursor, 3);

        for i in 4..=7 {
            bus.send(&Ev { pool_id: 1, user_address: "GABC", amount: i * 10 });
        }

        let replay = bus.replay_buffer.read().unwrap().get_replay(
            last_seen_cursor,
            Some("GABC"),
            Some(1),
            MAX_REPLAY_SIZE,
        );

        match replay {
            ReplayResult::Events { events, truncated, latest_cursor, .. } => {
                assert!(!truncated);
                assert_eq!(latest_cursor, 7);
                assert_eq!(events.len(), 4);
                let cursors: Vec<u64> = events.iter().map(|e| e.cursor).collect();
                assert_eq!(cursors, vec![4, 5, 6, 7]);
            }
            _ => panic!("expected ReplayResult::Events"),
        }
    }

    #[test]
    fn expired_cursor_on_reconnect_emits_gap_response() {
        let bus = EventBus::with_retention(5);
        #[derive(Serialize)]
        struct Ev {
            id: u64,
        }

        for i in 1..=10 {
            bus.send(&Ev { id: i });
        }

        assert_eq!(bus.oldest_retained_cursor(), 6);
        assert_eq!(bus.latest_cursor(), 10);

        let replay = bus.replay_buffer.read().unwrap().get_replay(
            2,
            None,
            None,
            MAX_REPLAY_SIZE,
        );

        match replay {
            ReplayResult::CursorExpired { oldest_cursor, latest_cursor } => {
                assert_eq!(oldest_cursor, 6);
                assert_eq!(latest_cursor, 10);
                let gap_frame = gap_cursor_expired(oldest_cursor, latest_cursor);
                let parsed: serde_json::Value = serde_json::from_str(&gap_frame).unwrap();
                assert_eq!(parsed["type"], "gap");
                assert_eq!(parsed["code"], "CURSOR_EXPIRED");
                assert_eq!(parsed["oldest_cursor"], 6);
                assert_eq!(parsed["latest_cursor"], 10);
            }
            _ => panic!("expected ReplayResult::CursorExpired"),
        }
    }

    #[test]
    fn replay_size_cap_enforced_with_gap_truncated() {
        let bus = EventBus::with_retention(300);
        #[derive(Serialize)]
        struct Ev {
            id: u64,
        }

        for i in 1..=300 {
            bus.send(&Ev { id: i });
        }

        let replay = bus.replay_buffer.read().unwrap().get_replay(
            0,
            None,
            None,
            MAX_REPLAY_SIZE,
        );

        match replay {
            ReplayResult::Events { events, truncated, next_cursor, latest_cursor } => {
                assert!(truncated);
                assert_eq!(events.len(), MAX_REPLAY_SIZE);
                assert_eq!(events[0].cursor, 1);
                assert_eq!(events[events.len() - 1].cursor, 250);
                assert_eq!(next_cursor, Some(251));
                assert_eq!(latest_cursor, 300);

                let gap_frame = gap_replay_truncated(next_cursor.unwrap(), latest_cursor);
                let parsed: serde_json::Value = serde_json::from_str(&gap_frame).unwrap();
                assert_eq!(parsed["type"], "gap");
                assert_eq!(parsed["code"], "REPLAY_TRUNCATED");
                assert_eq!(parsed["next_cursor"], 251);
                assert_eq!(parsed["latest_cursor"], 300);
            }
            _ => panic!("expected ReplayResult::Events with truncated == true"),
        }
    }

    #[test]
    fn slow_client_backpressure_emits_gap_frame() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        #[derive(Serialize)]
        struct Ev {
            id: usize,
        }

        for i in 0..300 {
            bus.send(&Ev { id: i });
        }

        match rx.try_recv() {
            Err(broadcast::error::TryRecvError::Lagged(missed)) => {
                assert!(missed > 0);
                let frame = gap_slow_client_lagged(missed);
                let parsed: serde_json::Value = serde_json::from_str(&frame).unwrap();
                assert_eq!(parsed["type"], "gap");
                assert_eq!(parsed["code"], "SLOW_CLIENT_LAGGED");
                assert_eq!(parsed["missed_count"], missed);
            }
            other => panic!("expected TryRecvError::Lagged, got {:?}", other),
        }
    }
}
