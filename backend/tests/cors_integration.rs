use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use predifi_backend::{
    build_router,
    config::{Config, ConfigError},
    price_cache::PriceCache,
    redis_cache::RedisCache,
    ws::EventBus,
};
use std::collections::HashMap;
use tower::ServiceExt;

fn test_router(config: Config) -> axum::Router {
    build_router(
        config,
        PriceCache::new(),
        RedisCache::disabled(),
        EventBus::new(),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Wildcard Tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn cors_wildcard_rejected_at_startup_in_production() {
    let vars = HashMap::from([
        (String::from("PREDIFI_APP_ENV"), String::from("production")),
        (
            String::from("PREDIFI_SECRET_KEY"),
            String::from("production-secret-key-that-is-at-least-32-chars!"),
        ),
        (
            String::from("PREDIFI_WS_ALLOWED_ORIGINS"),
            String::from("https://app.predifi.com"),
        ),
        (
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("*"),
        ),
    ]);

    let error = Config::from_map(&vars).expect_err("wildcard origin must be rejected in production");
    assert!(
        matches!(error, ConfigError::InvalidValue { key: "PREDIFI_CORS_ALLOWED_ORIGINS", .. }),
        "expected InvalidValue for PREDIFI_CORS_ALLOWED_ORIGINS, got: {error}"
    );
    let msg = error.to_string();
    assert!(
        msg.contains("wildcard"),
        "error message should clearly state wildcards are not permitted, got: {msg}"
    );
}

#[tokio::test]
async fn cors_credentialed_route_never_allows_wildcard() {
    let mut config = Config::default_for_test();
    // Even if config origins had wildcard or arbitrary origin:
    config.cors_allowed_origins = vec![String::from("*")];

    let app = test_router(config);

    // Test a credentialed route: /api/v1/auth/refresh
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/auth/refresh")
                .header(header::ORIGIN, "https://arbitrary-site.evil.com")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"refresh_token":"abc"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    let allow_origin = response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN);
    assert!(
        allow_origin.is_none(),
        "credentialed route must never allow arbitrary sites via wildcard; got: {allow_origin:?}"
    );
    let allow_credentials = response.headers().get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS);
    assert!(
        allow_credentials.is_none(),
        "credentialed route must never send allow-credentials for wildcard/arbitrary sites"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Empty Origin Tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn cors_empty_origins_rejected_at_startup_in_production() {
    let vars = HashMap::from([
        (String::from("PREDIFI_APP_ENV"), String::from("production")),
        (
            String::from("PREDIFI_SECRET_KEY"),
            String::from("production-secret-key-that-is-at-least-32-chars!"),
        ),
        (
            String::from("PREDIFI_WS_ALLOWED_ORIGINS"),
            String::from("https://app.predifi.com"),
        ),
        (
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("   "),
        ),
    ]);

    let error = Config::from_map(&vars).expect_err("empty CORS origin list must stop startup in production");
    assert!(
        matches!(error, ConfigError::InvalidValue { key: "PREDIFI_CORS_ALLOWED_ORIGINS", .. }),
        "expected InvalidValue error, got: {error}"
    );
}

#[tokio::test]
async fn cors_empty_origins_fails_closed_at_runtime() {
    let mut config = Config::default_for_test();
    config.cors_allowed_origins = vec![];

    let app = test_router(config);

    // Request from any origin when origin list is empty
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/auth/refresh")
                .header(header::ORIGIN, "https://app.predifi.com")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"refresh_token":"abc"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
        "empty origin configuration must fail closed and return no allow-origin header"
    );
    assert!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS).is_none(),
        "empty origin configuration must fail closed and return no allow-credentials header"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Valid Origin Tests (Public Read Routes vs Disallowed Origins)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cors_valid_origin_allowed_on_public_read_routes_without_credentials() {
    let mut config = Config::default_for_test();
    config.cors_allowed_origins = vec![String::from("https://app.predifi.com")];

    let app = test_router(config);

    // GET /health is a public read route
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/health")
                .header(header::ORIGIN, "https://app.predifi.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
        "https://app.predifi.com"
    );
    // Public read routes must NOT send allow-credentials: true
    assert!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS).is_none(),
        "public read route must not expose Access-Control-Allow-Credentials"
    );
}

#[tokio::test]
async fn cors_disallowed_origin_rejected_on_public_read_routes() {
    let mut config = Config::default_for_test();
    config.cors_allowed_origins = vec![String::from("https://app.predifi.com")];

    let app = test_router(config);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/health")
                .header(header::ORIGIN, "https://evil.untrusted.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
        "disallowed origin must NOT receive Access-Control-Allow-Origin header"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Credentialed Route Tests
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cors_valid_origin_receives_credentials_on_credentialed_routes() {
    let mut config = Config::default_for_test();
    config.cors_allowed_origins = vec![String::from("https://app.predifi.com")];

    let app = test_router(config);

    // POST /api/v1/auth/refresh is a credentialed route
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/auth/refresh")
                .header(header::ORIGIN, "https://app.predifi.com")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"refresh_token":"abc"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
        "https://app.predifi.com"
    );
    assert_eq!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS).unwrap(),
        "true",
        "credentialed routes must send Access-Control-Allow-Credentials: true"
    );
}

#[tokio::test]
async fn cors_disallowed_origin_rejected_on_credentialed_routes() {
    let mut config = Config::default_for_test();
    config.cors_allowed_origins = vec![String::from("https://app.predifi.com")];

    let app = test_router(config);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/auth/refresh")
                .header(header::ORIGIN, "https://evil.untrusted.com")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"refresh_token":"abc"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
        "disallowed origin must not receive Access-Control-Allow-Origin on credentialed routes"
    );
    assert!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS).is_none(),
        "disallowed origin must not receive Access-Control-Allow-Credentials"
    );
}

#[tokio::test]
async fn cors_preflight_on_credentialed_routes_sets_credentials_header() {
    let mut config = Config::default_for_test();
    config.cors_allowed_origins = vec![String::from("https://app.predifi.com")];

    let app = test_router(config);

    // OPTIONS preflight for POST /api/v1/auth/refresh
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/v1/auth/refresh")
                .header(header::ORIGIN, "https://app.predifi.com")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization, content-type")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
        "https://app.predifi.com"
    );
    assert_eq!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS).unwrap(),
        "true",
        "preflight response for credentialed route must set Access-Control-Allow-Credentials: true"
    );
}

#[tokio::test]
async fn cors_preflight_disallowed_origin_on_credentialed_routes_rejected() {
    let mut config = Config::default_for_test();
    config.cors_allowed_origins = vec![String::from("https://app.predifi.com")];

    let app = test_router(config);

    // OPTIONS preflight from disallowed origin
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/v1/auth/refresh")
                .header(header::ORIGIN, "https://evil.untrusted.com")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
        "preflight from disallowed origin must not return Access-Control-Allow-Origin"
    );
}
