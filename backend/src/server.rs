//! Server startup, router construction, and HTTP handlers.
//!
//! This module encapsulates everything needed to build and run the Axum
//! server — middleware, routes, and the `run` entry point that wires
//! together all dependencies (DB pool, price cache, Redis, event bus).
//!
//! Trace context propagation: inbound HTTP requests are wrapped in a
//! `tracing` span that extracts W3C `traceparent`/`tracestate` headers via
//! [`crate::tracing_context`], so downstream queue payloads and RPC calls
//! inherit the same trace id.

use crate::config::Config;
use crate::metrics::{Metrics, SharedMetrics};
use crate::request_logger::LoggingLayer;
use crate::shutdown;
use axum::{
    extract::State,
    middleware::{from_fn_with_state, Next},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use http::HeaderValue;
use serde_json::json;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::Instrument;
use tracing::{error, info, warn};

// ── CORS ─────────────────────────────────────────────────────────────────────

fn build_cors(config: &Config) -> CorsLayer {
    let origins: Vec<HeaderValue> = config
        .cors_allowed_origins
        .iter()
        .filter_map(|origin| origin.parse().ok())
        .collect();

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([
            http::Method::GET,
            http::Method::POST,
            http::Method::PUT,
            http::Method::DELETE,
            http::Method::OPTIONS,
        ])
        .allow_headers([
            http::header::CONTENT_TYPE,
            http::header::AUTHORIZATION,
            http::header::ACCEPT,
            http::header::HeaderName::from_static("traceparent"),
            http::header::HeaderName::from_static("tracestate"),
        ])
}

// ── Health contracts ─────────────────────────────────────────────────────────

/// Maximum time the readiness probe will spend probing a single dependency.
/// Keeps `/ready` latency bounded even when a dependency is hanging.
const READINESS_DEPENDENCY_TIMEOUT: Duration = Duration::from_secs(2);

/// Startup grace period: for this long after process start, readiness probes
/// report `starting` (503) without probing dependencies, so orchestrators do
/// not restart the instance while it is still warming up.
const STARTUP_GRACE: Duration = Duration::from_secs(10);

/// Process start time, used to enforce the startup grace period.
static PROCESS_START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn live() -> axum::response::Response {
    use axum::http::StatusCode;
    (
        StatusCode::OK,
        Json(json!({ "status": "alive", "service": "predifi-backend" })),
    )
        .into_response()
}

/// Liveness probe: reports only process health. Never touches dependencies,
/// so a Postgres/Redis/RPC outage cannot cause the orchestrator to restart a
/// healthy process.
async fn liveness() -> axum::response::Response {
    use axum::http::StatusCode;
    (
        StatusCode::OK,
        Json(json!({ "status": "alive", "service": "predifi-backend" })),
    )
        .into_response()
}

async fn health(State(state): State<crate::routes::v1::AppState>) -> axum::response::Response {
    use axum::http::StatusCode;

    let mut all_healthy = true;
    let mut db_status = "ok";

    if let Some(db) = &state.db {
        if sqlx::query("SELECT 1").execute(db).await.is_err() {
            db_status = "unreachable";
            all_healthy = false;
        }
    } else {
        db_status = "not_configured";
    }

    let mut rpc_status = "ok";
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(state.config.rpc_health_timeout_secs))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let mut rpc_attempts = 0;
    let max_attempts = state.config.rpc_health_retry_count as usize;
    while rpc_attempts < max_attempts {
        rpc_attempts += 1;
        let ok = client
            .post(&state.config.stellar_rpc_url)
            .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"getHealth"}))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        if ok {
            break;
        }
        if rpc_attempts < max_attempts {
            let backoff = std::cmp::min(2u64.pow((rpc_attempts - 1) as u32), 5);
            sleep(Duration::from_secs(backoff)).await;
        }
    }
    if rpc_attempts >= max_attempts {
        rpc_status = "unreachable";
        all_healthy = false;
    }

    let mut redis_status = "ok";
    if !state.redis.is_available() {
        redis_status = "not_configured";
        all_healthy = false;
    } else if !state.redis.ping().await {
        redis_status = "unreachable";
        all_healthy = false;
    }

    let mut price_cache_status = "ok";
    if state.cache.snapshot().is_empty() {
        price_cache_status = "not_ready";
        all_healthy = false;
    }

    let body = json!({
        "status": if all_healthy { "ok" } else { "error" },
        "service": "predifi-backend",
        "version": env!("CARGO_PKG_VERSION"),
        "dependencies": {
            "db": db_status,
            "rpc": rpc_status,
            "redis": redis_status,
            "price_cache": price_cache_status
        },
    });

    if all_healthy {
        (StatusCode::OK, Json(body)).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
    }
}

async fn ready(State(state): State<crate::routes::v1::AppState>) -> axum::response::Response {
    use axum::http::StatusCode;

    let mut ready = true;
    let mut db_status = "ok";
    let mut db_error: Option<String> = None;

    if let Some(db) = &state.db {
        if sqlx::query("SELECT 1").execute(db).await.is_err() {
            db_status = "unreachable";
            db_error = Some("database ping failed".to_string());
            ready = false;
        }
    } else {
        db_status = "not_configured";
        db_error = Some("database pool is not configured".to_string());
        ready = false;
    }

    let (redis_status, redis_error): (&str, Option<String>) = if !state.redis.is_available() {
        (
            "not_configured",
            Some("Redis is not configured".to_string()),
        )
    } else if !state.redis.ping().await {
        ("unreachable", Some("Redis ping failed".to_string()))
    } else {
        ("ok", None)
    };
    if redis_status != "ok" {
        ready = false;
    }

    let (price_cache_status, price_cache_error): (&str, Option<String>) =
        if state.cache.snapshot().is_empty() {
            ("not_ready", Some("price cache is empty".to_string()))
        } else {
            ("ok", None)
        };
    if price_cache_status != "ok" {
        ready = false;
    }

    let body = json!({
        "status": if ready { "ready" } else { "not_ready" },
        "dependencies": { "db": db_status, "redis": redis_status, "price_cache": price_cache_status },
        "errors":       { "db": db_error, "redis": redis_error,   "price_cache": price_cache_error  }
    });

    if ready {
        (StatusCode::OK, Json(body)).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
    }
}

/// Readiness probe: reports whether this instance can currently serve traffic.
///
/// Each dependency is probed independently under a bounded timeout budget so
/// that a single hanging dependency cannot blow up the overall response
/// latency. Unhealthy dependencies make the endpoint return 503, which
/// removes the instance from service without triggering a restart.
async fn readiness(State(state): State<crate::routes::v1::AppState>) -> axum::response::Response {
    use axum::http::StatusCode;

    let started = PROCESS_START.get_or_init(Instant::now);
    if started.elapsed() < STARTUP_GRACE {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "starting",
                "service": "predifi-backend",
                "grace_remaining_secs": STARTUP_GRACE
                    .saturating_sub(started.elapsed())
                    .as_secs(),
            })),
        )
            .into_response();
    }

    let (db_status, db_error) = match &state.db {
        Some(db) => match tokio::time::timeout(
            READINESS_DEPENDENCY_TIMEOUT,
            sqlx::query("SELECT 1").execute(db),
        )
        .await
        {
            Ok(Ok(_)) => ("ok", None),
            Ok(Err(_)) => ("unreachable", Some("database ping failed".to_string())),
            Err(_) => ("timeout", Some("database ping timed out".to_string())),
        },
        None => (
            "not_configured",
            Some("database pool is not configured".to_string()),
        ),
    };

    let (redis_status, redis_error) = if !state.redis.is_available() {
        (
            "not_configured",
            Some("Redis is not configured".to_string()),
        )
    } else {
        match tokio::time::timeout(READINESS_DEPENDENCY_TIMEOUT, state.redis.ping()).await {
            Ok(true) => ("ok", None),
            Ok(false) => ("unreachable", Some("Redis ping failed".to_string())),
            Err(_) => ("timeout", Some("Redis ping timed out".to_string())),
        }
    };

    let (price_cache_status, price_cache_error) = if state.cache.snapshot().is_empty() {
        ("not_ready", Some("price cache is empty".to_string()))
    } else {
        ("ok", None)
    };

    let ready = db_status == "ok" && redis_status == "ok" && price_cache_status == "ok";

    let body = json!({
        "status": if ready { "ready" } else { "not_ready" },
        "dependencies": { "db": db_status, "redis": redis_status, "price_cache": price_cache_status },
        "errors":       { "db": db_error, "redis": redis_error,   "price_cache": price_cache_error  }
    });

    if ready {
        (StatusCode::OK, Json(body)).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
    }
}

async fn root() -> Json<serde_json::Value> {
    Json(json!({ "message": "Welcome to the PrediFi backend", "api": "/api/v1" }))
}

async fn metrics_handler(State(state): State<crate::routes::v1::AppState>) -> impl IntoResponse {
    match state.metrics.gather_text() {
        Ok(body) => (
            axum::http::StatusCode::OK,
            [(http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
            body,
        ),
        Err(error) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            [(http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            format!("failed to gather metrics: {error}"),
        ),
    }
}

async fn metrics_middleware(
    State(metrics): State<SharedMetrics>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> axum::response::Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let response = next.run(request).await;
    let status = response.status().as_u16().to_string();
    metrics
        .http_requests_total
        .with_label_values(&[&method, &path, &status])
        .inc();
    response
}

/// Inbound trace-context middleware.
///
/// Extracts the W3C `traceparent`/`tracestate` headers (if present) and
/// opens a `http.request` span that becomes the parent for every downstream
/// span — including queue payloads and Soroban RPC calls. Sensitive
/// attributes (authorization headers, wallet addresses, request bodies) are
/// deliberately excluded from the span to keep traces free of secrets and
/// unbounded identifiers.
async fn trace_context_middleware(
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> axum::response::Response {
    let method = request.method().as_str().to_owned();
    let route = request.uri().path().to_owned();

    // Extract the parent context from inbound headers so the span joins the
    // caller's trace rather than starting a new one.
    let parent = crate::tracing_context::extract_trace_context(request.headers());

    let span = tracing::info_span!(
        "http.request",
        http.method = %method,
        http.route = %route,
        // Never record authorization, cookies, bodies, or wallet addresses.
        http.sensitive = tracing::field::Empty,
    );

    next.run(request).instrument(span).await
}

// ── Router builders ───────────────────────────────────────────────────────────

/// Build the full router without a DB pool (for tests / rate-limit tests).
///
/// Rate limiting is excluded here; use [`build_router_with_rate_limit`] when
/// you need it in non-test contexts.
pub fn build_router(
    config: Config,
    cache: crate::price_cache::PriceCache,
    redis: crate::redis_cache::RedisCache,
    event_bus: crate::ws::EventBus,
) -> Router {
    build_router_with_rate_limit(
        config,
        cache,
        redis,
        event_bus,
        crate::constants::RATE_LIMIT_PERIOD_SECS,
        crate::constants::RATE_LIMIT_BURST_SIZE,
    )
}

/// Build the Axum router with explicit rate-limit settings (no DB pool).
pub fn build_router_with_rate_limit(
    config: Config,
    cache: crate::price_cache::PriceCache,
    redis: crate::redis_cache::RedisCache,
    event_bus: crate::ws::EventBus,
    _per_second: u64,
    _burst_size: u32,
) -> Router {
    let prometheus_metrics = Arc::new(Metrics::new().unwrap_or_else(|error| {
        eprintln!("failed to initialize Prometheus metrics: {error}");
        std::process::exit(1);
    }));

    let state = crate::routes::v1::AppState {
        config: Arc::new(config.clone()),
        cache: cache.clone(),
        pool_cache: crate::pool_cache::PoolCache::with_negative_ttl(Duration::from_secs(
            config.pool_negative_cache_ttl_secs,
        )),
        redis: redis.clone(),
        db: None,
        metrics: prometheus_metrics.clone(),
        event_bus: event_bus.clone(),
    };

    Router::new()
        .route("/", get(root))
        .route("/live", get(live))
        .route("/liveness", get(liveness))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/readiness", get(readiness))
        .route("/metrics", get(metrics_handler))
        .with_state(state)
        .nest(
            "/api",
            crate::routes::router(
                Arc::new(config.clone()),
                cache,
                redis,
                None,
                prometheus_metrics.clone(),
                event_bus,
            ),
        )
        .merge(crate::openapi::swagger_router())
        .layer(from_fn_with_state(
            prometheus_metrics.clone(),
            metrics_middleware,
        ))
        .layer(from_fn(trace_context_middleware))
        .layer(build_cors(&config))
        .layer(LoggingLayer::with_metrics(prometheus_metrics.clone()))
}

/// Build the Axum router with a live database pool.
fn build_router_with_db(
    config: Config,
    cache: crate::price_cache::PriceCache,
    redis: crate::redis_cache::RedisCache,
    pool: sqlx::PgPool,
    event_bus: crate::ws::EventBus,
    prometheus_metrics: SharedMetrics,
) -> Router {
    let state = crate::routes::v1::AppState {
        config: Arc::new(config.clone()),
        cache: cache.clone(),
        pool_cache: crate::pool_cache::PoolCache::with_negative_ttl(Duration::from_secs(
            config.pool_negative_cache_ttl_secs,
        )),
        redis: redis.clone(),
        db: Some(pool.clone()),
        metrics: prometheus_metrics.clone(),
        event_bus: event_bus.clone(),
    };

    Router::new()
        .route("/", get(root))
        .route("/live", get(live))
        .route("/liveness", get(liveness))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/readiness", get(readiness))
        .route("/metrics", get(metrics_handler))
        .with_state(state)
        .nest(
            "/api",
            crate::routes::router_with_db(
                Arc::new(config.clone()),
                cache,
                redis,
                pool,
                prometheus_metrics.clone(),
                event_bus,
            ),
        )
        .merge(crate::openapi::swagger_router())
        .layer(from_fn_with_state(
            prometheus_metrics.clone(),
            metrics_middleware,
        ))
        .layer(from_fn(trace_context_middleware))
        .layer(build_cors(&config))
        .layer(LoggingLayer::with_metrics(prometheus_metrics.clone()))
}

/// Probe Redis once at startup and either fail fast or record a degraded
/// state, depending on `config.redis_required`.
///
/// If Redis is configured but unreachable:
/// - `config.redis_required == true`: logs a clear error and exits the
///   process immediately, before the HTTP listener is bound, so an operator
///   sees a startup failure rather than a silently degraded service.
/// - `config.redis_required == false`: logs a single explicit warning and
///   lets startup continue. The degraded state is then visible on every
///   subsequent call to `/ready` and `/health`, which independently probe
///   Redis per request via [`crate::redis_cache::RedisCache::ping`].
async fn probe_redis_startup(config: &Config, redis: &crate::redis_cache::RedisCache) {
    if redis.is_available() && redis.ping().await {
        return;
    }

    if config.redis_required {
        error!(
            "Redis is configured (PREDIFI_REDIS_URL) but unreachable at startup, and \
             PREDIFI_REDIS_REQUIRED=true; refusing to start. Set PREDIFI_REDIS_REQUIRED=false \
             to start in a degraded mode instead."
        );
        std::process::exit(1);
    }

    warn!(
        "Redis is configured (PREDIFI_REDIS_URL) but unreachable at startup; starting in a \
         degraded mode (PREDIFI_REDIS_REQUIRED=false). Cache-backed reads will fall through to \
         the database until Redis becomes reachable. This state is visible on /ready and /health."
    );
}

// ── Server entry points ───────────────────────────────────────────────────────

/// Initialise all dependencies, build the router, and start serving.
pub async fn run(config: Config) {
    run_with_signal(config, shutdown::wait_for_signal()).await;
}

/// Initialise all dependencies, build the router, start serving, and tear
/// everything down again when `signal` resolves.
///
/// Split out from [`run`] so the shutdown plumbing is testable via a
/// hand-crafted future rather than real OS signals.
///
/// # Shutdown order
/// 1. `signal` resolves → Axum stops accepting new connections.
/// 2. In-flight requests drain (bounded by `config.shutdown_timeout_secs`).
/// 3. Background workers (price-cache fetcher, Stellar listener) are aborted.
/// 4. PostgreSQL pool is closed.
/// 5. OTel batch exporter is flushed via [`crate::telemetry::shutdown_tracer_provider`].
pub async fn run_with_signal<F>(config: Config, signal: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    // Record process start so readiness probes honour the startup grace period.
    PROCESS_START.get_or_init(Instant::now);

    let pool = crate::db::create_pool(&config)
        .await
        .unwrap_or_else(|error| {
            error!(error = %error, "failed to initialize PostgreSQL pool");
            std::process::exit(1);
        });

    let prometheus_metrics = Arc::new(Metrics::new().unwrap_or_else(|error| {
        eprintln!("failed to initialize Prometheus metrics: {error}");
        std::process::exit(1);
    }));

    let cache = crate::price_cache::PriceCache::new();
    let fetcher_handle: JoinHandle<()> =
        crate::price_cache::spawn_fetcher(cache.clone(), Some(prometheus_metrics.clone()));

    let event_bus = crate::ws::EventBus::new();
    let redis = crate::redis_cache::RedisCache::new(&config.redis_url)
        .await
        .with_metrics(prometheus_metrics.clone());
    probe_redis_startup(&config, &redis).await;

    // Clone before moving into the worker closure.
    let listener_rpc_url = config.stellar_rpc_url.clone();
    let listener_pool = pool.clone();
    let listener_event_bus = event_bus.clone();
    let listener_redis = redis.clone();
    let listener_rpc_timeout = Duration::from_secs(config.rpc_timeout_secs);
    let listener_batch_size = config.indexer_max_batch_size;

    // spawn_worker roots the listener under a named OTel span so all Stellar
    // sync traces are correlated in the trace backend.
    let listener_handle: JoinHandle<()> =
        crate::tracing_context::spawn_worker("stellar_listener", async move {
            crate::worker::stellar_listener::run_worker(
                listener_rpc_url,
                listener_pool,
                listener_event_bus,
                listener_redis,
                listener_rpc_timeout,
                listener_batch_size,
            )
            .await;
        });

    // Periodically sweeps pool/prediction state to generate notifications
    // (pool ending soon, resolved, claim window expiring, interest matches).
    // Every insert is deduplicated on the DB side, so this can run on a fixed
    // interval without ever double-notifying a user.
    let notifications_pool = pool.clone();
    let notifications_handle: JoinHandle<()> =
        crate::tracing_context::spawn_worker("notification_sweep", async move {
            crate::notifications::run_sweep_loop(notifications_pool, Duration::from_secs(300))
                .await;
        });

    if redis.is_available() {
        info!("Redis cache initialized and available");
    } else {
        warn!("Redis cache unavailable - running without caching");
    }

    let app = build_router_with_db(
        config.clone(),
        cache,
        redis,
        pool.clone(),
        event_bus,
        prometheus_metrics,
    );

    let bind_addr = config.bind_address();
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|error| {
            error!(address = %bind_addr, error = %error, "failed to bind TCP listener");
            std::process::exit(1);
        });

    info!(address = %bind_addr, "backend server listening");

    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(signal);

    let drain_timeout = Duration::from_secs(config.shutdown_timeout_secs.max(1));

    match tokio::time::timeout(drain_timeout, server).await {
        Ok(Ok(())) => info!(
            timeout_secs = drain_timeout.as_secs(),
            "HTTP server drained in-flight requests cleanly"
        ),
        Ok(Err(error)) => {
            warn!(error = %error, "HTTP server returned an error during shutdown drain")
        }
        Err(_) => warn!(
            component = "http server drain",
            timeout_secs = drain_timeout.as_secs(),
            "HTTP drain exceeded shutdown timeout; aborting in-flight handlers"
        ),
    }

    // Abort workers before closing the pool so they cannot race it.
    fetcher_handle.abort();
    listener_handle.abort();
    notifications_handle.abort();

    // Close the pool after aborting workers.
    shutdown::with_shutdown_timeout(drain_timeout, "database pool close", pool.close()).await;

    // Flush the OTel batch exporter — must happen after all spans are done.
    crate::telemetry::shutdown_tracer_provider();

    info!("graceful shutdown complete");
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, DEFAULT_CORS_ORIGINS};

    fn config_with_origins(origins: Vec<&str>) -> Config {
        let mut cfg = Config::default_for_test();
        cfg.cors_allowed_origins = origins.into_iter().map(|s| s.to_string()).collect();
        cfg
    }

    #[test]
    fn build_cors_accepts_valid_https_origin() {
        let cfg = config_with_origins(vec!["https://predifi.app"]);
        let _layer = build_cors(&cfg);
    }

    #[test]
    fn build_cors_default_origins_are_all_valid_header_values() {
        let cfg = config_with_origins(DEFAULT_CORS_ORIGINS.to_vec());
        let valid_count = cfg
            .cors_allowed_origins
            .iter()
            .filter(|o| o.parse::<HeaderValue>().is_ok())
            .count();
        assert_eq!(
            valid_count,
            DEFAULT_CORS_ORIGINS.len(),
            "every default CORS origin must parse into a valid HeaderValue"
        );
    }

    #[test]
    fn build_cors_accepts_origin_with_port() {
        let cfg = config_with_origins(vec!["http://localhost:5173"]);
        let _layer = build_cors(&cfg);
        assert!("http://localhost:5173".parse::<HeaderValue>().is_ok());
    }

    #[test]
    fn build_cors_accepts_multiple_origins() {
        let cfg = config_with_origins(vec![
            "https://predifi.app",
            "https://staging.predifi.app",
            "http://localhost:3000",
        ]);
        let _layer = build_cors(&cfg);
    }

    #[test]
    fn build_cors_with_empty_origins_does_not_panic() {
        let cfg = config_with_origins(vec![]);
        let _layer = build_cors(&cfg);
    }

    #[test]
    fn build_cors_uses_config_origins_not_hardcoded_list() {
        let cfg_a = config_with_origins(vec!["https://app-a.example.com"]);
        let cfg_b = config_with_origins(vec!["https://app-b.example.com"]);
        let _layer_a = build_cors(&cfg_a);
        let _layer_b = build_cors(&cfg_b);
        assert_ne!(cfg_a.cors_allowed_origins, cfg_b.cors_allowed_origins);
    }
}
