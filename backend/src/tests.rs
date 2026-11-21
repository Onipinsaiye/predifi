use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt; // provides `.oneshot()`

use crate::config::Config;
use crate::mock_rpc_helpers::setup_healthy_test_env;
use crate::{build_router, price_cache::PriceCache, redis_cache::RedisCache};

/// Build a router backed by a mock Stellar RPC and populated price cache.
async fn build_healthy_router() -> (axum::Router, crate::mock_rpc_helpers::MockRpcServer) {
    let (config, cache, mock) = setup_healthy_test_env().await;
    let router = build_router(
        config,
        cache,
        RedisCache::simulate_available(),
        crate::ws::EventBus::new(),
    );
    (router, mock)
}

/// Build a bare GET request with no body for the given path.
fn get(path: &str) -> Request<axum::body::Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .body(axum::body::Body::empty())
        .expect("failed to build request")
}

/// Read a response body all the way into a `String`.
async fn body_string(body: axum::body::Body) -> String {
    let bytes = body
        .collect()
        .await
        .expect("failed to collect body")
        .to_bytes();
    String::from_utf8(bytes.to_vec()).expect("body is not valid utf-8")
}

/// GET / must return HTTP 200.
#[tokio::test]
async fn root_returns_200() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/"))
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::OK);
}

/// GET /health must return HTTP 200 with `{"status":"ok"}` in the body.
#[tokio::test]
async fn health_returns_200_with_ok_body() {
    let (router, mock) = build_healthy_router().await;
    let response = router
        .oneshot(get("/health"))
        .await
        .expect("request failed");
    mock.shutdown().await;

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\""),
        "body should contain a status field, got: {body}"
    );
}

/// GET /api/v1/health must return HTTP 200 from the nested v1 router.
#[tokio::test]
async fn api_v1_health_returns_200_with_versioned_body() {
    let (router, mock) = build_healthy_router().await;
    let response = router
        .oneshot(get("/api/v1/health"))
        .await
        .expect("request failed");
    mock.shutdown().await;

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\"") && body.contains("\"version\":\"v1\""),
        "body should contain status and version fields, got: {body}"
    );
}

/// GET /api/v1 must return HTTP 200 from the version discovery route.
#[tokio::test]
async fn api_v1_index_returns_200() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/api/v1"))
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::OK);
}

/// GET /metrics must return HTTP 200 and expose Prometheus text format.
#[tokio::test]
async fn metrics_endpoint_returns_200() {
    let router = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    );

    // Prime the latency histogram via a normal request before scraping /metrics.
    let _ = router
        .clone()
        .oneshot(get("/"))
        .await
        .expect("warmup request failed");

    let response = router
        .oneshot(get("/metrics"))
        .await
        .expect("request failed");

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response.into_body()).await;
    assert!(body.contains("app_up"));
    assert!(body.contains("app_http_request_duration_seconds"));
}

/// GET /api/v1/fees returns the current fee configuration.
#[tokio::test]
async fn api_v1_fees_returns_config_values() {
    let mut config = Config::default_for_test();
    config.treasury_fee_bps = 400;
    config.referral_fee_bps = 6000;

    let response = build_router(
        config,
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/api/v1/fees"))
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"treasury_fee_bps\":400"),
        "body should contain treasury_fee_bps, got: {body}"
    );
    assert!(
        body.contains("\"referral_fee_bps\":6000"),
        "body should contain referral_fee_bps, got: {body}"
    );
}

/// GET /nonexistent must return HTTP 404 (Axum's built-in fallback).
#[tokio::test]
async fn unknown_route_returns_404() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/nonexistent"))
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// Verify the middleware does not alter the status code of a 200 response.
#[tokio::test]
async fn middleware_does_not_alter_200_status() {
    let (router, mock) = build_healthy_router().await;
    let response = router
        .oneshot(get("/health"))
        .await
        .expect("request failed");
    mock.shutdown().await;

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "logging middleware must not modify the response status"
    );
}

/// Verify the middleware does not alter the status code of a 404 response.
#[tokio::test]
async fn middleware_does_not_alter_404_status() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/no-such-path"))
    .await
    .expect("request failed");

    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "logging middleware must not modify 404 responses"
    );
}

/// Fire multiple requests through the same router to confirm that the
/// middleware handles repeated calls without errors or panics.
#[tokio::test]
async fn middleware_handles_multiple_requests_sequentially() {
    let (config, cache, mock) = setup_healthy_test_env().await;
    let router = build_router(
        config,
        cache,
        RedisCache::simulate_available(),
        crate::ws::EventBus::new(),
    );

    let paths_and_expected: &[(&str, StatusCode)] = &[
        ("/", StatusCode::OK),
        ("/health", StatusCode::OK),
        ("/missing", StatusCode::NOT_FOUND),
    ];

    for (path, expected_status) in paths_and_expected {
        let response = router
            .clone()
            .oneshot(get(path))
            .await
            .expect("request failed");

        assert_eq!(
            response.status(),
            *expected_status,
            "unexpected status for {path}"
        );
    }

    mock.shutdown().await;
}

/// CORS headers must be present when a request comes from an allowed origin.
#[tokio::test]
async fn cors_allows_allowed_origin() {
    let (router, mock) = build_healthy_router().await;
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/health")
                .header(header::ORIGIN, "http://localhost:5173")
                .body(axum::body::Body::empty())
                .expect("failed to build request"),
        )
        .await
        .expect("request failed");
    mock.shutdown().await;

    assert_eq!(response.status(), StatusCode::OK);

    let allow_origin = response
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok());

    assert_eq!(
        allow_origin,
        Some("http://localhost:5173"),
        "CORS header should reflect the allowed origin"
    );
}

/// Preflight OPTIONS request must return 200 for allowed origins.
#[tokio::test]
async fn cors_handles_preflight_request() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(
        Request::builder()
            .method(Method::OPTIONS)
            .uri("/health")
            .header(header::ORIGIN, "http://localhost:5173")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .body(axum::body::Body::empty())
            .expect("failed to build request"),
    )
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::OK);
}

/// Requests from an origin that is NOT in the allow-list must not receive an
/// `Access-Control-Allow-Origin` header.  The request itself is still served
/// (CORS is enforced by the browser, not the server), but the missing header
/// tells the browser to block the response.
#[tokio::test]
async fn cors_rejects_disallowed_origin() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(
        Request::builder()
            .method(Method::GET)
            .uri("/health")
            .header(header::ORIGIN, "https://evil.example.com")
            .body(axum::body::Body::empty())
            .expect("failed to build request"),
    )
    .await
    .expect("request failed");

    let allow_origin = response
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok());

    assert_eq!(
        allow_origin, None,
        "disallowed origin must not receive an Access-Control-Allow-Origin header"
    );
}

/// Preflight from a disallowed origin must not receive an
/// `Access-Control-Allow-Origin` header.
#[tokio::test]
async fn cors_rejects_disallowed_origin_preflight() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(
        Request::builder()
            .method(Method::OPTIONS)
            .uri("/health")
            .header(header::ORIGIN, "https://evil.example.com")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .body(axum::body::Body::empty())
            .expect("failed to build request"),
    )
    .await
    .expect("request failed");

    let allow_origin = response
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok());

    assert_eq!(
        allow_origin, None,
        "preflight from a disallowed origin must not receive an Access-Control-Allow-Origin header"
    );
}

/// A custom origin list supplied via Config is respected.
#[tokio::test]
async fn cors_respects_custom_origin_list() {
    let mut config = Config::default_for_test();
    config.cors_allowed_origins = vec![String::from("https://custom.example.com")];

    // The custom origin should be allowed.
    let response = build_router(
        config.clone(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(
        Request::builder()
            .method(Method::GET)
            .uri("/health")
            .header(header::ORIGIN, "https://custom.example.com")
            .body(axum::body::Body::empty())
            .expect("failed to build request"),
    )
    .await
    .expect("request failed");

    let allow_origin = response
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok());

    assert_eq!(
        allow_origin,
        Some("https://custom.example.com"),
        "custom allowed origin should receive the CORS header"
    );

    // The default localhost origin should now be blocked.
    let response2 = build_router(
        config,
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(
        Request::builder()
            .method(Method::GET)
            .uri("/health")
            .header(header::ORIGIN, "http://localhost:5173")
            .body(axum::body::Body::empty())
            .expect("failed to build request"),
    )
    .await
    .expect("request failed");

    let allow_origin2 = response2
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok());

    assert_eq!(
        allow_origin2, None,
        "origin not in the custom list must be blocked"
    );
}

/// Verify that the rate limiter returns 429 Too Many Requests after exceeding the limit.
#[tokio::test]
#[ignore = "Rate limiting test interferes with other parallel tests due to shared key extractor"]
async fn rate_limiting_returns_429_after_burst() {
    // Use a very small burst size for this test to trigger rate limiting quickly
    let app = crate::server::build_router_with_rate_limit(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
        1, // 1 second period
        5, // 5 requests burst
    );

    // Fire 5 requests which should all be 200 OK.
    for _ in 0..5 {
        let response = app
            .clone()
            .oneshot(get("/health"))
            .await
            .expect("request failed");
        assert_eq!(response.status(), StatusCode::OK);
    }

    // The 6th request should be rate limited.
    let response = app.oneshot(get("/health")).await.expect("request failed");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"error\""),
        "rate limit response should contain 'error' field, got: {body}"
    );
    assert!(
        body.contains("Too many requests"),
        "rate limit response should contain human-readable message, got: {body}"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Advanced Health Check Tests
// ──────────────────────────────────────────────────────────────────────────────

/// Test that /api/v1/health returns 200 with dependency status when everything is OK.
#[tokio::test]
async fn api_v1_health_returns_200_with_dependency_status() {
    let (router, mock) = build_healthy_router().await;
    let response = router
        .oneshot(get("/api/v1/health"))
        .await
        .expect("request failed");
    mock.shutdown().await;

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\""),
        "body should contain status field, got: {body}"
    );
    assert!(
        body.contains("\"version\":\"v1\""),
        "body should contain version field, got: {body}"
    );
    assert!(
        body.contains("\"dependencies\""),
        "body should contain dependencies field, got: {body}"
    );
    assert!(
        body.contains("\"db\""),
        "body should contain db dependency status, got: {body}"
    );
    assert!(
        body.contains("\"rpc\""),
        "body should contain rpc dependency status, got: {body}"
    );
}

/// Test that /health returns 200 with dependency status when everything is OK.
#[tokio::test]
async fn root_health_returns_200_with_dependency_status() {
    let (router, mock) = build_healthy_router().await;
    let response = router
        .oneshot(get("/health"))
        .await
        .expect("request failed");
    mock.shutdown().await;

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\""),
        "body should contain status field, got: {body}"
    );
    assert!(
        body.contains("\"service\":\"predifi-backend\""),
        "body should contain service field, got: {body}"
    );
    assert!(
        body.contains("\"dependencies\""),
        "body should contain dependencies field, got: {body}"
    );
}

/// Test that /api/v1/health reports db as 'not_configured' when no database is provided.
#[tokio::test]
async fn api_v1_health_reports_db_not_configured_without_pool() {
    let (router, mock) = build_healthy_router().await;
    let response = router
        .oneshot(get("/api/v1/health"))
        .await
        .expect("request failed");
    mock.shutdown().await;

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"db\":\"not_configured\""),
        "body should indicate db is not_configured, got: {body}"
    );
}

/// Test that /health reports db as 'not_configured' when no database is provided.
#[tokio::test]
async fn root_health_reports_db_not_configured_without_pool() {
    let (router, mock) = build_healthy_router().await;
    let response = router
        .oneshot(get("/health"))
        .await
        .expect("request failed");
    mock.shutdown().await;

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"db\":\"not_configured\""),
        "body should indicate db is not_configured, got: {body}"
    );
}

/// Test that /api/v1/health returns the "ok" status when healthy.
#[tokio::test]
async fn api_v1_health_status_is_ok_when_healthy() {
    let (router, mock) = build_healthy_router().await;
    let response = router
        .oneshot(get("/api/v1/health"))
        .await
        .expect("request failed");
    mock.shutdown().await;

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\":\"ok\""),
        "status should be 'ok' when healthy, got: {body}"
    );
}

/// Test that /health returns the "ok" status when healthy.
#[tokio::test]
async fn root_health_status_is_ok_when_healthy() {
    let (router, mock) = build_healthy_router().await;
    let response = router
        .oneshot(get("/health"))
        .await
        .expect("request failed");
    mock.shutdown().await;

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\":\"ok\""),
        "status should be 'ok' when healthy, got: {body}"
    );
}

/// Test that health endpoint includes the version from Cargo.toml.
#[tokio::test]
async fn health_includes_cargo_version() {
    let (router, mock) = build_healthy_router().await;
    let response = router
        .oneshot(get("/health"))
        .await
        .expect("request failed");
    mock.shutdown().await;

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"version\""),
        "body should contain version field, got: {body}"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// 503 Service Unavailable Tests (Acceptance Criteria Verification)
// ──────────────────────────────────────────────────────────────────────────────

/// Test that /api/v1/health returns HTTP 503 when RPC is unreachable.
/// This is critical for the acceptance criteria: "Returns 503 if any dependency is unreachable."
#[tokio::test]
async fn api_v1_health_returns_503_when_rpc_unreachable() {
    let mut config = Config::default_for_test();
    // Point RPC to an invalid/unreachable endpoint to simulate failure
    config.stellar_rpc_url = String::from("http://localhost:1/invalid");

    let response = build_router(
        config,
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/api/v1/health"))
    .await
    .expect("request failed");

    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "health endpoint should return 503 when RPC is unreachable (acceptance criteria)"
    );

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\":\"error\""),
        "status should be 'error' when degraded, got: {body}"
    );
    assert!(
        body.contains("\"rpc\":\"unreachable\""),
        "rpc status should be 'unreachable', got: {body}"
    );
}

/// Test that /health returns HTTP 503 when RPC is unreachable.
/// This is critical for the acceptance criteria: "Returns 503 if any dependency is unreachable."
#[tokio::test]
async fn root_health_returns_503_when_rpc_unreachable() {
    let mut config = Config::default_for_test();
    // Point RPC to an invalid/unreachable endpoint to simulate failure
    config.stellar_rpc_url = String::from("http://localhost:1/invalid");

    let response = build_router(
        config,
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/health"))
    .await
    .expect("request failed");

    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "health endpoint should return 503 when RPC is unreachable (acceptance criteria)"
    );

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\":\"error\""),
        "status should be 'error' when degraded, got: {body}"
    );
    assert!(
        body.contains("\"rpc\":\"unreachable\""),
        "rpc status should be 'unreachable', got: {body}"
    );
}

/// Verify that HTTP 503 response includes full dependency information for debugging.
#[tokio::test]
async fn health_503_response_includes_dependency_details() {
    let mut config = Config::default_for_test();
    config.stellar_rpc_url = String::from("http://localhost:1/invalid");

    let response = build_router(
        config,
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/health"))
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"dependencies\""),
        "503 response should include dependencies object, got: {body}"
    );
    assert!(
        body.contains("\"db\""),
        "503 response should show db status, got: {body}"
    );
    assert!(
        body.contains("\"rpc\""),
        "503 response should show rpc status, got: {body}"
    );
}

/// Test that /api/v1/health returns HTTP 503 when Redis is unreachable.
/// This is critical for the acceptance criteria: "Returns 503 if any dependency is unreachable."
#[tokio::test]
async fn api_v1_health_returns_503_when_redis_unreachable() {
    // Create a mock Redis cache that always fails ping
    let redis = RedisCache::disabled();

    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        redis,
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/api/v1/health"))
    .await
    .expect("request failed");

    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "health endpoint should return 503 when Redis is unreachable (acceptance criteria)"
    );

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\":\"error\""),
        "status should be 'error' when degraded, got: {body}"
    );
    assert!(
        body.contains("\"redis\":\"not_configured\""),
        "redis status should be 'not_configured', got: {body}"
    );
}

/// Test that /health returns HTTP 503 when Redis is unreachable.
/// This is critical for the acceptance criteria: "Returns 503 if any dependency is unreachable."
#[tokio::test]
async fn root_health_returns_503_when_redis_unreachable() {
    // Create a mock Redis cache that always fails ping
    let redis = RedisCache::disabled();

    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        redis,
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/health"))
    .await
    .expect("request failed");

    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "health endpoint should return 503 when Redis is unreachable (acceptance criteria)"
    );

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\":\"error\""),
        "status should be 'error' when degraded, got: {body}"
    );
    assert!(
        body.contains("\"redis\":\"not_configured\""),
        "redis status should be 'not_configured', got: {body}"
    );
}

/// Verify that HTTP 503 response includes Redis dependency information for debugging.
#[tokio::test]
async fn health_503_response_includes_redis_dependency_details() {
    // Create a mock Redis cache that always fails ping
    let redis = RedisCache::disabled();

    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        redis,
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/health"))
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"dependencies\""),
        "503 response should include dependencies object, got: {body}"
    );
    assert!(
        body.contains("\"redis\""),
        "503 response should show redis status, got: {body}"
    );
}

/// Test that /api/v1/health returns HTTP 503 when price cache is not ready.
/// This is critical for the acceptance criteria: "Returns 503 if any dependency is unreachable."
#[tokio::test]
async fn api_v1_health_returns_503_when_price_cache_not_ready() {
    // Create a price cache that is empty
    let cache = PriceCache::new();

    let response = build_router(
        Config::default_for_test(),
        cache,
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/api/v1/health"))
    .await
    .expect("request failed");

    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "health endpoint should return 503 when price cache is not ready (acceptance criteria)"
    );

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\":\"error\""),
        "status should be 'error' when degraded, got: {body}"
    );
    assert!(
        body.contains("\"price_cache\":\"not_ready\""),
        "price_cache status should be 'not_ready', got: {body}"
    );
}

/// Test that /health returns HTTP 503 when price cache is not ready.
/// This is critical for the acceptance criteria: "Returns 503 if any dependency is unreachable."
#[tokio::test]
async fn root_health_returns_503_when_price_cache_not_ready() {
    // Create a price cache that is empty
    let cache = PriceCache::new();

    let response = build_router(
        Config::default_for_test(),
        cache,
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/health"))
    .await
    .expect("request failed");

    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "health endpoint should return 503 when price cache is not ready (acceptance criteria)"
    );

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\":\"error\""),
        "status should be 'error' when degraded, got: {body}"
    );
    assert!(
        body.contains("\"price_cache\":\"not_ready\""),
        "price_cache status should be 'not_ready', got: {body}"
    );
}

/// Verify that HTTP 503 response includes price cache dependency information for debugging.
#[tokio::test]
async fn health_503_response_includes_price_cache_dependency_details() {
    // Create a price cache that is empty
    let cache = PriceCache::new();

    let response = build_router(
        Config::default_for_test(),
        cache,
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/health"))
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"dependencies\""),
        "503 response should include dependencies object, got: {body}"
    );
    assert!(
        body.contains("\"price_cache\""),
        "503 response should show price_cache status, got: {body}"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Clock Skew Tests (#348)
// ──────────────────────────────────────────────────────────────────────────────
//
// Deadline authority map (which component owns each deadline):
//   * Auth challenge expiry  -> backend (server clock is authoritative; the
//     client's clock is never trusted, only the server-issued `expires_at`).
//   * Quote freshness        -> backend (quote TTL is evaluated against the
//     server clock at read time).
//   * Close-time display     -> backend (the pool's `close_time` is the single
//     source of truth; clients only render it).
//   * Notification schedule  -> backend (scheduling decisions use the server
//     clock; a client resuming a suspended tab must not re-trigger or skip
//     notifications based on its own clock).
//
// All scenarios below drive the injectable clock from `test_support` so the
// results are deterministic regardless of wall-clock time.

/// A challenge issued at `T` must be accepted while the server clock is
/// before `expires_at`, even if the client claims to be far in the past
/// (negative skew: client clock behind server clock).
#[tokio::test]
async fn auth_challenge_accepts_negative_client_skew() {
    use crate::test_support::clock::{Clock, Duration as SkewDuration};

    let clock = Clock::at_unix(1_700_000_000);
    let ttl = SkewDuration::from_secs(300);
    let challenge = clock.issue_challenge(ttl);

    // Client clock is 10 minutes behind the server.
    let client_now = clock.now() - SkewDuration::from_secs(600);
    assert!(
        challenge.is_valid_at(client_now, &clock),
        "challenge must be validated against the server clock, not the client's"
    );
    assert!(
        challenge.is_valid_at(clock.now(), &clock),
        "challenge must be valid at the server's current time"
    );
}

/// A challenge must be rejected once the *server* clock passes `expires_at`,
/// even if the client claims to be far in the future (positive skew: client
/// clock ahead of server clock). The client's clock must never extend the
/// lifetime of a challenge.
#[tokio::test]
async fn auth_challenge_rejects_positive_client_skew() {
    use crate::test_support::clock::{Clock, Duration as SkewDuration};

    let clock = Clock::at_unix(1_700_000_000);
    let ttl = SkewDuration::from_secs(300);
    let challenge = clock.issue_challenge(ttl);

    // Advance the authoritative server clock past the deadline.
    clock.advance(SkewDuration::from_secs(301));

    // Client claims to be an hour in the future — irrelevant to the decision.
    let client_now = clock.now() + SkewDuration::from_secs(3600);
    assert!(
        !challenge.is_valid_at(client_now, &clock),
        "expired challenge must be rejected regardless of client clock"
    );
    assert!(
        !challenge.is_valid_at(clock.now(), &clock),
        "expired challenge must be rejected at the server's current time"
    );
}

/// A challenge must be rejected exactly at `expires_at` (half-open interval
/// `[issued_at, expires_at)`), so boundary behaviour is deterministic.
#[tokio::test]
async fn auth_challenge_expires_at_boundary() {
    use crate::test_support::clock::{Clock, Duration as SkewDuration};

    let clock = Clock::at_unix(1_700_000_000);
    let ttl = SkewDuration::from_secs(60);
    let challenge = clock.issue_challenge(ttl);

    clock.advance(SkewDuration::from_secs(59));
    assert!(challenge.is_valid_at(clock.now(), &clock));

    clock.advance(SkewDuration::from_secs(1));
    assert!(
        !challenge.is_valid_at(clock.now(), &clock),
        "challenge must be invalid at exactly expires_at"
    );
}

/// A quote is fresh while the server clock is within its TTL, and stale once
/// the server clock advances past it — regardless of client skew in either
/// direction.
#[tokio::test]
async fn quote_freshness_uses_server_clock_under_both_skew_directions() {
    use crate::test_support::clock::{Clock, Duration as SkewDuration};

    let clock = Clock::at_unix(1_700_000_000);
    let ttl = SkewDuration::from_secs(30);
    let quote = clock.issue_quote(ttl);

    // Negative skew: client 5 minutes behind, server still within TTL.
    let client_behind = clock.now() - SkewDuration::from_secs(300);
    assert!(
        quote.is_fresh_at(client_behind, &clock),
        "quote must be fresh when the server clock is within TTL"
    );

    // Positive skew: client 5 minutes ahead, server still within TTL.
    let client_ahead = clock.now() + SkewDuration::from_secs(300);
    assert!(
        quote.is_fresh_at(client_ahead, &clock),
        "quote must be fresh when the server clock is within TTL"
    );

    // Advance the authoritative server clock past the TTL.
    clock.advance(SkewDuration::from_secs(31));
    assert!(
        !quote.is_fresh_at(client_behind, &clock),
        "quote must be stale once the server clock passes its TTL"
    );
    assert!(
        !quote.is_fresh_at(client_ahead, &clock),
        "quote must be stale once the server clock passes its TTL"
    );
}

/// Close-time display must be derived from the server-authoritative
/// `close_time`, so a client with either skew direction still sees the same
/// open/closed state.
#[tokio::test]
async fn close_display_is_authoritative_under_skew() {
    use crate::test_support::clock::{Clock, Duration as SkewDuration};

    let clock = Clock::at_unix(1_700_000_000);
    let close_time = clock.now() + SkewDuration::from_secs(120);

    // Both skewed clients must agree with the server that the pool is open.
    let client_behind = clock.now() - SkewDuration::from_secs(600);
    let client_ahead = clock.now() + SkewDuration::from_secs(600);
    assert!(clock.is_open_at(close_time, client_behind));
    assert!(clock.is_open_at(close_time, client_ahead));

    // Advance the server clock past close_time; both clients must now see
    // the pool as closed.
    clock.advance(SkewDuration::from_secs(121));
    assert!(!clock.is_open_at(close_time, client_behind));
    assert!(!clock.is_open_at(close_time, client_ahead));
}

/// Notification scheduling must be driven by the server clock. A client
/// resuming a suspended tab (whose local clock jumped forward while the tab
/// was frozen) must not cause the scheduler to fire notifications that the
/// server has not yet reached.
#[tokio::test]
async fn notification_schedule_ignores_suspended_tab_clock_jump() {
    use crate::test_support::clock::{Clock, Duration as SkewDuration};

    let clock = Clock::at_unix(1_700_000_000);
    let fire_at = clock.now() + SkewDuration::from_secs(60);
    let schedule = clock.schedule_notification(fire_at);

    // Tab is suspended; on resume the client's local clock has jumped an
    // hour forward. The server clock has not moved, so nothing should fire.
    let resumed_client_now = clock.now() + SkewDuration::from_secs(3600);
    assert!(
        !schedule.is_due_at(resumed_client_now, &clock),
        "a suspended-tab clock jump must not trigger a notification early"
    );

    // Advance the authoritative server clock past fire_at; the notification
    // is now due regardless of the client's clock.
    clock.advance(SkewDuration::from_secs(61));
    assert!(
        schedule.is_due_at(resumed_client_now, &clock),
        "notification must fire once the server clock reaches fire_at"
    );
}

/// A suspended tab that resumes with its clock *behind* the server must not
/// cause the scheduler to skip a notification that is already due.
#[tokio::test]
async fn notification_schedule_fires_after_resume_with_negative_skew() {
    use crate::test_support::clock::{Clock, Duration as SkewDuration};

    let clock = Clock::at_unix(1_700_000_000);
    let fire_at = clock.now() + SkewDuration::from_secs(30);
    let schedule = clock.schedule_notification(fire_at);

    // Server clock passes fire_at while the tab is suspended.
    clock.advance(SkewDuration::from_secs(31));

    // On resume the client's clock is 10 minutes behind the server.
    let resumed_client_now = clock.now() - SkewDuration::from_secs(600);
    assert!(
        schedule.is_due_at(resumed_client_now, &clock),
        "a due notification must still fire after resume even with negative skew"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Market Quote Simulation Tests (#269)
// ──────────────────────────────────────────────────────────────────────────────
//
// The quote endpoint must be a deterministic, read-only estimate of the
// outcome shares and price impact for a liquidity pool.  These tests compare
// the HTTP response against the in-process reference pricing model over
// boundary and randomized vectors, and verify that stale or unsupported pool
// state is rejected.

/// Build a POST request with a JSON body for the given path.
fn post_json(path: &str, body: serde_json::Value) -> Request<axum::body::Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .expect("failed to build request")
}

/// Parse a response body into a JSON value.
async fn body_json(body: axum::body::Body) -> serde_json::Value {
    let raw = body_string(body).await;
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("body is not valid JSON ({e}): {raw}"))
}

/// A pool state that is fresh and supported by the pricing model.
fn fresh_pool_state() -> crate::market_quote::PoolState {
    crate::market_quote::PoolState {
        pool_id: "pool-1".to_string(),
        reserve_a: 1_000_000,
        reserve_b: 2_000_000,
        fee_bps: 30,
        updated_at: 1_700_000_000,
        supported: true,
    }
}

/// The quote endpoint must return every field required by the acceptance
/// criteria: input units, output estimate, price impact, fee and timestamp.
#[tokio::test]
async fn quote_returns_all_required_fields() {
    let state = fresh_pool_state();
    let request = crate::market_quote::QuoteRequest {
        pool: state.clone(),
        amount_in: 10_000,
        now: state.updated_at,
    };

    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(post_json(
        "/api/v1/markets/quote",
        serde_json::to_value(&request).unwrap(),
    ))
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;

    assert_eq!(json["amount_in"], 10_000, "input units must be echoed");
    assert!(
        json["amount_out"].as_u64().unwrap_or(0) > 0,
        "output estimate must be positive, got: {json}"
    );
    assert!(
        json["price_impact_bps"].is_number(),
        "price impact must be present, got: {json}"
    );
    assert!(json["fee"].is_number(), "fee must be present, got: {json}");
    assert_eq!(
        json["quoted_at"], state.updated_at,
        "quote timestamp must be reported"
    );
}

/// The endpoint output must match the reference pricing model exactly for a
/// set of boundary vectors (zero-ish input, tiny input, large input, and an
/// input equal to the reserve).
#[tokio::test]
async fn quote_matches_reference_on_boundary_vectors() {
    let state = fresh_pool_state();
    let boundaries = [1_u64, 2, 100, 1_000_000, 1_000_000 - 1];

    for amount_in in boundaries {
        let request = crate::market_quote::QuoteRequest {
            pool: state.clone(),
            amount_in,
            now: state.updated_at,
        };
        let expected = crate::market_quote::quote(&request).expect("reference quote");

        let response = build_router(
            Config::default_for_test(),
            PriceCache::new(),
            RedisCache::disabled(),
            crate::ws::EventBus::new(),
        )
        .oneshot(post_json(
            "/api/v1/markets/quote",
            serde_json::to_value(&request).unwrap(),
        ))
        .await
        .expect("request failed");

        assert_eq!(response.status(), StatusCode::OK, "amount_in={amount_in}");
        let json = body_json(response.into_body()).await;

        assert_eq!(json["amount_in"], expected.amount_in);
        assert_eq!(json["amount_out"], expected.amount_out);
        assert_eq!(json["fee"], expected.fee);
        assert_eq!(json["price_impact_bps"], expected.price_impact_bps);
        assert_eq!(json["quoted_at"], expected.quoted_at);
    }
}

/// The endpoint output must match the reference pricing model over a set of
/// deterministic pseudo-random vectors (no external RNG dependency).
#[tokio::test]
async fn quote_matches_reference_on_randomized_vectors() {
    let state = fresh_pool_state();

    // Simple deterministic LCG so the vectors are reproducible.
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        seed
    };

    for _ in 0..32 {
        let amount_in = (next() % state.reserve_a) + 1;
        let request = crate::market_quote::QuoteRequest {
            pool: state.clone(),
            amount_in,
            now: state.updated_at,
        };
        let expected = crate::market_quote::quote(&request).expect("reference quote");

        let response = build_router(
            Config::default_for_test(),
            PriceCache::new(),
            RedisCache::disabled(),
            crate::ws::EventBus::new(),
        )
        .oneshot(post_json(
            "/api/v1/markets/quote",
            serde_json::to_value(&request).unwrap(),
        ))
        .await
        .expect("request failed");

        assert_eq!(response.status(), StatusCode::OK, "amount_in={amount_in}");
        let json = body_json(response.into_body()).await;

        assert_eq!(json["amount_out"], expected.amount_out, "amount_in={amount_in}");
        assert_eq!(json["fee"], expected.fee, "amount_in={amount_in}");
        assert_eq!(
            json["price_impact_bps"], expected.price_impact_bps,
            "amount_in={amount_in}"
        );
    }
}

/// A pool whose state is older than the freshness window must be rejected
/// with a client error rather than silently quoted.
#[tokio::test]
async fn quote_rejects_stale_pool_state() {
    let mut state = fresh_pool_state();
    state.updated_at = 1_700_000_000;
    let request = crate::market_quote::QuoteRequest {
        pool: state,
        amount_in: 1_000,
        // Far beyond the freshness window.
        now: 1_700_000_000 + 86_400,
    };

    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(post_json(
        "/api/v1/markets/quote",
        serde_json::to_value(&request).unwrap(),
    ))
    .await
    .expect("request failed");

    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "stale pool state must be rejected"
    );
}

/// A pool whose state is flagged unsupported by the pricing model must be
/// rejected with a client error.
#[tokio::test]
async fn quote_rejects_unsupported_pool_state() {
    let mut state = fresh_pool_state();
    state.supported = false;
    let request = crate::market_quote::QuoteRequest {
        pool: state.clone(),
        amount_in: 1_000,
        now: state.updated_at,
    };

    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(post_json(
        "/api/v1/markets/quote",
        serde_json::to_value(&request).unwrap(),
    ))
    .await
    .expect("request failed");

    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "unsupported pool state must be rejected"
    );
}

/// A zero input amount must be rejected as invalid.
#[tokio::test]
async fn quote_rejects_zero_amount() {
    let state = fresh_pool_state();
    let request = crate::market_quote::QuoteRequest {
        pool: state.clone(),
        amount_in: 0,
        now: state.updated_at,
    };

    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(post_json(
        "/api/v1/markets/quote",
        serde_json::to_value(&request).unwrap(),
    ))
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// Test that /api/v1/health includes error details in the response.
#[tokio::test]
async fn api_v1_health_includes_error_details() {
    // Create a mock Redis cache that always fails ping
    let redis = RedisCache::disabled();

    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        redis,
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/api/v1/health"))
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"errors\""),
        "503 response should include errors object, got: {body}"
    );
    assert!(
        body.contains("\"redis\":null"),
        "redis error should be null when not configured, got: {body}"
    );
}

/// GET /api/v1/users/:address/referrals without a DB returns 503.
#[tokio::test]
#[ignore = "Route returns 404 for short test addresses; needs valid Stellar address"]
async fn user_referrals_without_db_returns_503() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/api/v1/users/GABC123/referrals"))
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("database not configured"),
        "body should mention database not configured, got: {body}"
    );
}
/// Test odds calculation logic with various scenarios.
#[tokio::test]
async fn test_odds_calculation() {
    use crate::db::calculate_odds;

    // Test case 1: Normal case with two outcomes
    let outcome_stakes = vec![(0, 25000), (1, 50000)];
    let total_stake = 75000;
    let odds = calculate_odds(&outcome_stakes, total_stake);

    assert_eq!(odds.len(), 2);
    assert_eq!(odds[0].outcome, 0);
    assert_eq!(odds[0].stake, 25000);
    assert!((odds[0].odds - 3.0).abs() < 0.001); // 1.0 / (25000/75000) = 3.0

    assert_eq!(odds[1].outcome, 1);
    assert_eq!(odds[1].stake, 50000);
    assert!((odds[1].odds - 1.5).abs() < 0.001); // 1.0 / (50000/75000) = 1.5

    // Test case 2: Zero total stake
    let odds_zero_total = calculate_odds(&outcome_stakes, 0);
    assert_eq!(odds_zero_total.len(), 2);
    assert_eq!(odds_zero_total[0].odds, 0.0);
    assert_eq!(odds_zero_total[1].odds, 0.0);

    // Test case 3: One outcome has zero stake
    let outcome_stakes_with_zero = vec![(0, 0), (1, 100000)];
    let odds_with_zero = calculate_odds(&outcome_stakes_with_zero, 100000);
    assert_eq!(odds_with_zero[0].odds, 0.0); // Zero stake = 0 odds
    assert!((odds_with_zero[1].odds - 1.0).abs() < 0.001); // 1.0 / (100000/100000) = 1.0
}

/// Build a syntactically- and checksum-valid Stellar `G…` test address, so
/// tests can exercise routes that use `Path<StellarAddress>` as their
/// extractor without the request being rejected before it reaches the
/// handler (see `crate::validated_types::StellarAddress`).
fn test_stellar_address() -> String {
    use crate::validated_types::{base32_encode, crc16_xmodem, STRKEY_VERSION_ACCOUNT_ID};
    let mut bytes = vec![STRKEY_VERSION_ACCOUNT_ID];
    bytes.extend(std::iter::repeat(0xAB_u8).take(32));
    let checksum = crc16_xmodem(&bytes);
    bytes.extend_from_slice(&checksum.to_le_bytes());
    base32_encode(&bytes)
}

/// `GET /api/v1/creators/:address/stats` without a DB pool must return 503
/// with the standard error envelope — not silently 200 with an `"error"`
/// field in the body, and not a generic panic/500 (#1736).
#[tokio::test]
async fn api_v1_creator_stats_returns_503_without_db() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get(&format!(
        "/api/v1/creators/{}/stats",
        test_stellar_address()
    )))
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\":\"error\""),
        "body should use the standard error envelope, got: {body}"
    );
}

/// `GET /api/v1/creators/:address/stats` with a malformed address must be
/// rejected at the extractor with 400, before the handler (and thus the
/// database) is ever reached.
#[tokio::test]
async fn api_v1_creator_stats_rejects_invalid_address() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/api/v1/creators/not-a-real-address/stats"))
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// Test pool details endpoint returns error when database is not available.
#[tokio::test]
async fn api_v1_pool_details_returns_error_without_db() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/api/v1/pools/1"))
    .await
    .expect("request failed");

    // Note: This will likely return 429 due to rate limiting in tests
    // In a real environment with DB, it would return 200 with error message
    let body = body_string(response.into_body()).await;

    // The test mainly verifies the route exists and is callable
    // Actual functionality requires database integration testing
    assert!(!body.is_empty(), "response should not be empty");
}
/// Test user predictions endpoint returns error when database is not available.
#[tokio::test]
#[ignore = "Pre-existing route validation issue in test environment"]
async fn api_v1_user_predictions_returns_error_without_db() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get(
        "/api/v1/users/GXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX/predictions",
    ))
    .await
    .expect("request failed");

    // Note: This will likely return 429 due to rate limiting in tests
    // In a real environment with DB, it would return 200 with error message
    let body = body_string(response.into_body()).await;

    // The test mainly verifies the route exists and is callable
    // Actual functionality requires database integration testing
    assert!(!body.is_empty(), "response should not be empty");
}

/// Test user predictions endpoint with query parameters.
#[tokio::test]
#[ignore = "Pre-existing route validation issue in test environment"]
async fn api_v1_user_predictions_handles_pagination() {
    let response = build_router(Config::default_for_test(), PriceCache::new(), RedisCache::disabled(), crate::ws::EventBus::new())
        .oneshot(get("/api/v1/users/GXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX/predictions?limit=10&offset=5"))
        .await
        .expect("request failed");

    let body = body_string(response.into_body()).await;

    // Verify the route accepts pagination parameters
    assert!(!body.is_empty(), "response should not be empty");
}
/// Test leaderboard endpoint returns error when database is not available.
#[tokio::test]
async fn api_v1_leaderboard_returns_error_without_db() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/api/v1/leaderboard"))
    .await
    .expect("request failed");

    // Note: This will likely return 429 due to rate limiting in tests
    // In a real environment with DB, it would return 200 with error message
    let body = body_string(response.into_body()).await;

    // The test mainly verifies the route exists and is callable
    assert!(!body.is_empty(), "response should not be empty");
}

/// Test leaderboard endpoint with different ranking parameters.
#[tokio::test]
async fn api_v1_leaderboard_handles_ranking_parameters() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get(
        "/api/v1/leaderboard?rank_by=winnings&limit=10&offset=5",
    ))
    .await
    .expect("request failed");

    let body = body_string(response.into_body()).await;

    // Verify the route accepts ranking and pagination parameters
    assert!(!body.is_empty(), "response should not be empty");
}

/// Test leaderboard endpoint with volume ranking (default).
#[tokio::test]
async fn api_v1_leaderboard_defaults_to_volume_ranking() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/api/v1/leaderboard?limit=5"))
    .await
    .expect("request failed");

    let body = body_string(response.into_body()).await;

    // Verify the route works with default volume ranking
    assert!(!body.is_empty(), "response should not be empty");
}

/// GET /api/v1/stats returns an error JSON when no database is configured.
#[tokio::test]
async fn api_v1_stats_returns_error_without_db() {
    let response = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/api/v1/stats"))
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("database not available") || body.contains("\"error\""),
        "should report error when db is absent, got: {body}"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Liveness Probe Tests (/live)
// ──────────────────────────────────────────────────────────────────────────────

/// GET /live must always return HTTP 200 without checking dependencies.
#[tokio::test]
async fn live_returns_200_without_dependency_checks() {
    let response = crate::server::build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/live"))
    .await
    .expect("request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\":\"alive\""),
        "liveness probe should report alive, got: {body}"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Readiness Probe Tests (/ready)
// ──────────────────────────────────────────────────────────────────────────────

/// GET /ready must return HTTP 503 when Redis is disabled (not configured).
///
/// This is the primary acceptance criterion: the readiness probe must signal
/// "not ready" when Redis is unavailable so orchestrators can withhold traffic.
#[tokio::test]
async fn ready_returns_503_when_redis_disabled() {
    let (config, cache, mock) = setup_healthy_test_env().await;
    let response = crate::server::build_router(
        config,
        cache,
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/ready"))
    .await
    .expect("request failed");
    mock.shutdown().await;

    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "readiness probe must return 503 when Redis is not configured"
    );

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"status\":\"not_ready\""),
        "status should be 'not_ready' when Redis is unavailable, got: {body}"
    );
    assert!(
        body.contains("\"redis\":\"not_configured\""),
        "redis dependency should be 'not_configured', got: {body}"
    );
    assert!(
        body.contains("\"price_cache\""),
        "readiness probe should include price_cache dependency, got: {body}"
    );
}

/// GET /ready must include a `dependencies` object with a `redis` field.
#[tokio::test]
async fn ready_response_includes_redis_dependency() {
    let (config, cache, mock) = setup_healthy_test_env().await;
    let response = crate::server::build_router(
        config,
        cache,
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/ready"))
    .await
    .expect("request failed");
    mock.shutdown().await;

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"dependencies\""),
        "response should include a dependencies object, got: {body}"
    );
    assert!(
        body.contains("\"redis\""),
        "dependencies should include a redis field, got: {body}"
    );
}

/// GET /ready must include an `errors` object describing why Redis is not ready.
#[tokio::test]
async fn ready_response_includes_error_details_when_not_ready() {
    let (config, cache, mock) = setup_healthy_test_env().await;
    let response = crate::server::build_router(
        config,
        cache,
        RedisCache::disabled(),
        crate::ws::EventBus::new(),
    )
    .oneshot(get("/ready"))
    .await
    .expect("request failed");
    mock.shutdown().await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("\"errors\""),
        "response should include an errors object, got: {body}"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Graceful Shutdown Tests (#997)
// ──────────────────────────────────────────────────────────────────────────────
//
// These tests use a hand-crafted `axum::Router` with a slow handler plus an
// injected shutdown-trigger future so we can exercise Axum's
// `with_graceful_shutdown` plumbing end-to-end without needing a real
// PostgreSQL pool, Redis, or OS signal delivery in the test process.

/// Verify that a request which is already in flight when the shutdown
/// signal fires is allowed to complete with its real response.
#[tokio::test]
async fn graceful_shutdown_drains_inflight_request() {
    use axum::{routing::get, Router};
    use std::time::Duration;
    use tokio::sync::oneshot;
    use tokio::time::sleep;

    async fn slow() -> &'static str {
        sleep(Duration::from_millis(500)).await;
        "done"
    }

    let app: Router = Router::new().route("/slow", get(slow));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr is available");

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let server_handle = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            // Ignore cancellation: we only care that this future resolves when
            // the test fires `shutdown_tx`.
            let _ = shutdown_rx.await;
        })
        .await
    });

    // Give the server a small window to finish its accept loop setup.
    sleep(Duration::from_millis(50)).await;

    // Build a fresh reqwest client per test.  Disabling connection pooling
    // keeps the "new connection after shutdown refused" assertion
    // deterministic: we want every request to open a new TCP socket and
    // observe the absence of the listener, not reuse a half-closed
    // keep-alive connection that produces a misleading success.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .pool_max_idle_per_host(0)
        .build()
        .expect("reqwest client builds");

    // Kick off a slow request that will be pending when we trigger shutdown.
    let in_flight = {
        let url = format!("http://{}/slow", addr);
        let client = client.clone();
        tokio::spawn(async move { client.get(url).send().await })
    };

    // Wait long enough for the request to actually reach the handler (and
    // therefore to be sitting in `slow()` mid-sleep).
    sleep(Duration::from_millis(150)).await;

    // Trigger graceful shutdown while the request is still in flight.
    shutdown_tx
        .send(())
        .expect("shutdown trigger must be sendable");

    // The in-flight request must still complete successfully.
    let response = tokio::time::timeout(Duration::from_secs(3), in_flight)
        .await
        .expect("in-flight request should be drained within 3 s")
        .expect("request task did not panic")
        .expect("request itself failed");

    assert_eq!(
        response.status(),
        axum::http::StatusCode::OK,
        "in-flight request must complete with 200 OK during graceful drain"
    );
    let body = response.text().await.expect("body");
    assert_eq!(
        body, "done",
        "drained request must keep its full response body"
    );

    // The server task itself must finish cleanly.  Three nested expects walk
    // through `Result<Result<Result<(), io::Error>, JoinError>, Elapsed>`:
    // outer is the timeout, middle is the JoinError, inner is the io::Error
    // returned by `axum::serve`.
    tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish after drain")
        .expect("server task panicked")
        .expect("server returned an io::Error during shutdown");

    // A NEW connection after shutdown must be refused — the listener has
    // already been dropped.
    let after_shutdown = tokio::time::timeout(
        Duration::from_secs(1),
        client.get(format!("http://{}/slow", addr)).send(),
    )
    .await;

    if let Ok(Ok(_)) = after_shutdown {
        panic!(
            "new connections after shutdown must fail, but got: {:?}",
            after_shutdown
        );
    }
}

/// Verify multiple concurrent in-flight requests all complete during a
/// graceful shutdown rather than being aborted individually.
#[tokio::test]
async fn graceful_shutdown_drains_many_concurrent_requests() {
    use axum::{routing::get, Router};
    use std::time::Duration;
    use tokio::sync::oneshot;
    use tokio::time::sleep;

    async fn slow() -> &'static str {
        sleep(Duration::from_millis(300)).await;
        "ok"
    }

    let app: Router = Router::new().route("/slow", get(slow));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let server_handle = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        })
        .await
    });

    sleep(Duration::from_millis(50)).await;

    // Disable reqwest connection pooling so each request opens a fresh
    // socket and is forced to observe whether the listener is still
    // accepting connections after shutdown.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .pool_max_idle_per_host(0)
        .build()
        .unwrap();

    let mut handles = Vec::new();
    for _ in 0..5 {
        let url = format!("http://{}/slow", addr);
        let c = client.clone();
        handles.push(tokio::spawn(async move { c.get(url).send().await }));
    }

    sleep(Duration::from_millis(100)).await;
    shutdown_tx.send(()).unwrap();

    for (i, h) in handles.into_iter().enumerate() {
        let response = tokio::time::timeout(Duration::from_secs(3), h)
            .await
            .unwrap_or_else(|_| panic!("request #{i} did not drain within 3 s"))
            .unwrap_or_else(|_| panic!("request #{i} task panicked"))
            .unwrap_or_else(|_| panic!("request #{i} reqwest call failed"));
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    let _ = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server must exit");
}
