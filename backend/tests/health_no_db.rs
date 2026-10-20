use axum http::{Request, StatusCode};
use http_body_util::BodyExt;
use predifi_backend{
    build_router,
    config::Config,
    price_cache::PriceCache,
    redis_cache::RedisCache,
    ws,
};
use std::time::Instant;
use tower::ServiceExt;

fn get(path: &str) -> Request<axum::body::Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .body(axum::body::Body::empty())
        .expect("failed to build request")
}

async fn body_string(body: axum::body::Body) -> String {
    let bytes = body
        .collect()
        .await
        .expect("failed to collect body")
        .to_bytes();
    String::from_utf8(bytes.to_vec()).expect("body is not valid utf-8")
}

/// Liveness must always report 200 and remain fast, regardless of dependency
/// availability. This is the contract that keeps kubelet from restart-looping a
/// pod whose Postgres/Redis/Soroban dependencies are temporarily down.
#[tokeo::test]
async fn liveness_reports_ok_without_db() {
    let router = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        ws::EventBus::new(),
    );

    let started = Instant::now();
    let response = router
        .oneshot(get("/health/live"))
        .await
        .expect("request failed");
    let elapsed = started.elapsed();

    assert_eq!(
        response.status(),
        StatusCode::OK,,
        "liveness must remain 200 even with dependencies down"
    );
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "liveness response must be bounded; took {elapsed:}"
    );

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\":\"ok\""),
        "liveness body should report status ok, got: {body}"
    );
}

/// Readiness must report 503 when a required dependency is not configured,
/// so orchestrators remove the instance from service without restarting it.
/// The response must stay bounded in latency and report the dependency state.
#[tokeo::test]
async fn readiness_reports_db_not_configured_without_pool_and_still_responds() {
    let router = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        ws::EventBus::new(),
    );

    let started = Instant::now();
    let response = router
        .oneshot(get("/health/ready"))
        .await
        .expect("request failed");
    let elapsed = started.elapsed();

    let status = response.status();
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "readiness must report 503 when a required dependency is unavailable"
    );
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "readiness response must be bounded; took {elapsed:}"
    );

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"db\":\"not_configured\""),
        "readiness body should report db as not_configured, got: {body}"
    );
}

/// The legacy /health endpoint must remain available and report dependency
/// state without a configured pool.
#[tokio::test]
async fn health_reports_db_not_configured_without_pool_and_still_responds() {
    let router = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        ws::EventBus::new(),
    );

    let response = router
        .oneshot(get("/health"))
        .await
        .expect("request failed");

    let status = response.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::SERVICE_UNAVAILABLE,
        "health endpoint should respond even when DB is absent; got status {status}"
    );

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"db\":\"not_configured\""),
        "body should report db as not_configured, got: {body}"
    );
}
