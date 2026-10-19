use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use predifi_backend::{
    build_router, config::Config, price_cache::PriceCache, redis_cache::RedisCache, ws,
};
use tower::ServiceExt;

fn get(path: &str) -> Request<axum::body::Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .body(axum::body::Body::empty())
        .expect("failed to build request")
}

#[tokio::test]
async fn metrics_returns_prometheus_content_type_and_body() {
    let router = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        ws::EventBus::new(),
    );

    let response = router
        .oneshot(get("/metrics"))
        .await
        .expect("request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response
        .headers()
        .get(http::header::CONTENT_TYPE)
        .expect("content-type header must be present")
        .to_str()
        .expect("content-type header must be valid utf-8");
    assert_eq!(content_type, "text/plain; version=0.0.4");

    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("failed to collect body")
        .to_bytes();
    assert!(!bytes.is_empty(), "metrics body must not be empty");
}

#[tokio::test]
async fn high_cardinality_paths_do_not_create_unbounded_http_series() {
    let router = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        ws::EventBus::new(),
    );
    let mut addresses = Vec::new();

    for id in 0..64 {
        let address = format!("G{id:055}");
        addresses.push(address.clone());
        for path in [
            format!("/api/v1/pools/{}", id + 1),
            format!("/api/v1/users/{address}/profile"),
            format!("/unmatched/{id}/ledger/{}", id + 10_000),
        ] {
            router
                .clone()
                .oneshot(get(&path))
                .await
                .expect("request failed");
        }
    }

    let response = router
        .oneshot(get("/metrics"))
        .await
        .expect("metrics request failed");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("failed to collect metrics")
        .to_bytes();
    let text = String::from_utf8(bytes.to_vec()).expect("metrics must be utf-8");

    assert!(text.contains("path=\"/api/v1/pools/:id\""), "{text}");
    assert!(
        text.contains("path=\"/api/v1/users/:address/profile\""),
        "{text}"
    );
    assert!(
        !text.contains("/api/v1/pools/1"),
        "pool IDs leaked into labels"
    );
    for address in addresses {
        assert!(
            !text.contains(&address),
            "wallet address leaked into labels"
        );
    }
    assert!(
        !text.contains("/unmatched/"),
        "raw paths leaked into labels"
    );

    let request_series = text
        .lines()
        .filter(|line| line.starts_with("app_http_requests_total{"))
        .count();
    assert!(
        request_series <= 6,
        "64 IDs per route must produce a bounded series count, got {request_series}"
    );
}
