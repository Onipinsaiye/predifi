//! CI gate: the OpenAPI spec must match live route behaviour (#52).
//!
//! Generated frontend types are only as good as `openapi.json`, so this suite
//! fails when:
//!
//! * a route registered in `src/routes` is neither documented nor listed in
//!   `openapi::UNDOCUMENTED_ROUTES` (or a documented route no longer exists);
//! * a documented operation is missing the error cases its shape implies
//!   (path params → 400, JSON bodies → 400/413/415/422, idempotent mutations
//!   → 401/409/422);
//! * a live response's status code is not documented for its operation, or
//!   its body does not match the documented schema for its content type;
//! * the committed `openapi.json` is not a byte-for-byte, reproducible
//!   regeneration of the spec.
//!
//! Regenerate the artifact with `cargo run --bin predifi-openapi -- --out openapi.json`.

use std::collections::BTreeSet;

use axum::body::Body;
use axum::http::{header, Request};
use serde_json::Value;
use tower::ServiceExt;

use predifi_backend::config::Config;
use predifi_backend::openapi::{
    openapi_artifact, openapi_json, OPENAPI_ARTIFACT_PATH, UNDOCUMENTED_ROUTES,
};
use predifi_backend::openapi_drift::{documented_response, normalize_path, scan_routes, validate};
use predifi_backend::{build_router, price_cache::PriceCache, redis_cache::RedisCache};

/// A syntactically valid Stellar account address (strkey checksum included).
const ADDRESS: &str = "GADQOBYHA4DQOBYHA4DQOBYHA4DQOBYHA4DQOBYHA4DQOBYHA4DQOZPI";

const HTTP_METHODS: [&str; 5] = ["get", "post", "put", "patch", "delete"];

/// `(METHOD, normalized path)` for every route registered under `/api`.
fn source_routes() -> BTreeSet<(String, String)> {
    let root = env!("CARGO_MANIFEST_DIR");
    let read = |file: &str| {
        std::fs::read_to_string(format!("{root}/src/routes/{file}"))
            .unwrap_or_else(|e| panic!("read src/routes/{file}: {e}"))
    };
    let mut routes = scan_routes(&read("v1.rs"), "/api/v1");
    routes.extend(scan_routes(&read("mod.rs"), "/api"));
    assert!(routes.len() > 20, "route scan found too few routes: {routes:?}");
    routes
        .into_iter()
        .map(|(m, p)| (m, normalize_path(&p)))
        .collect()
}

/// `(METHOD, normalized path)` for every operation in the spec.
fn documented_operations(spec: &Value) -> BTreeSet<(String, String)> {
    let mut ops = BTreeSet::new();
    for (path, item) in spec["paths"].as_object().expect("spec has paths") {
        for method in HTTP_METHODS {
            if item.get(method).is_some() {
                ops.insert((method.to_uppercase(), normalize_path(path)));
            }
        }
    }
    ops
}

fn allowlisted() -> BTreeSet<(String, String)> {
    UNDOCUMENTED_ROUTES
        .iter()
        .map(|(m, p, _)| (m.to_string(), normalize_path(p)))
        .collect()
}

#[test]
fn every_api_route_is_documented_or_allowlisted() {
    let documented = documented_operations(&openapi_json());
    let allowlisted = allowlisted();
    let undocumented: Vec<_> = source_routes()
        .into_iter()
        .filter(|r| !documented.contains(r) && !allowlisted.contains(r))
        .collect();
    assert!(
        undocumented.is_empty(),
        "routes missing from the OpenAPI spec (document them in src/openapi.rs \
         or justify them in UNDOCUMENTED_ROUTES): {undocumented:?}"
    );
}

#[test]
fn documented_and_allowlisted_routes_exist() {
    let routes = source_routes();
    let documented = documented_operations(&openapi_json());
    let stale: Vec<_> = documented
        .iter()
        .filter(|(_, path)| path.starts_with("/api/"))
        .filter(|r| !routes.contains(*r))
        .collect();
    assert!(stale.is_empty(), "documented operations with no route: {stale:?}");

    for entry in allowlisted() {
        assert!(routes.contains(&entry), "UNDOCUMENTED_ROUTES entry has no route: {entry:?}");
        assert!(!documented.contains(&entry), "{entry:?} is documented; drop it from UNDOCUMENTED_ROUTES");
    }
}

#[test]
fn operations_document_their_error_cases() {
    let spec = openapi_json();
    let mut problems = Vec::new();
    for (path, item) in spec["paths"].as_object().unwrap() {
        for method in HTTP_METHODS {
            let Some(op) = item.get(method) else { continue };
            let label = format!("{} {path}", method.to_uppercase());
            let responses = op["responses"].as_object().cloned().unwrap_or_default();

            if !responses.keys().any(|c| c.starts_with(['4', '5'])) {
                problems.push(format!("{label}: documents no error responses"));
            }
            let mut required = Vec::new();
            if path.contains('{') {
                required.push(("400", "path parameter extraction can fail"));
            }
            if op.get("requestBody").is_some() {
                required.push(("400", "malformed JSON body"));
                required.push(("413", "body over the route's size limit"));
                required.push(("415", "missing JSON content type"));
                required.push(("422", "body does not match the schema"));
            }
            let idempotent = op["parameters"]
                .as_array()
                .is_some_and(|ps| ps.iter().any(|p| p["name"] == "Idempotency-Key"));
            if idempotent {
                required.push(("401", "Idempotency-Key requires authentication"));
                required.push(("409", "concurrent request with the same key"));
                required.push(("422", "key reused with a different payload"));
            }
            for (code, why) in required {
                if !responses.contains_key(code) {
                    problems.push(format!("{label}: missing {code} ({why})"));
                }
            }
            for (code, response) in &responses {
                let content = response["content"].as_object();
                if code.starts_with(['4', '5']) && content.is_none_or(|c| c.is_empty()) {
                    problems.push(format!("{label}: {code} documents no body schema"));
                }
            }
        }
    }
    assert!(problems.is_empty(), "incomplete error documentation:\n{}", problems.join("\n"));
}

#[test]
fn generated_spec_is_reproducible_and_committed() {
    let first = openapi_artifact();
    let second = openapi_artifact();
    assert_eq!(first, second, "OpenAPI generation is not deterministic");

    let reparsed: Value = serde_json::from_str(&first).expect("artifact is valid JSON");
    assert_eq!(
        format!("{}\n", serde_json::to_string_pretty(&reparsed).unwrap()),
        first,
        "artifact is not in canonical form"
    );

    let path = format!("{}/{OPENAPI_ARTIFACT_PATH}", env!("CARGO_MANIFEST_DIR"));
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {path}: {e}"))
        .replace("\r\n", "\n");
    assert!(
        committed == first,
        "openapi.json is stale; run `cargo run --bin predifi-openapi -- --out openapi.json`"
    );
}

struct Case {
    method: &'static str,
    /// Concrete request path.
    uri: String,
    /// Documented path template the request exercises.
    template: &'static str,
    body: Option<&'static str>,
}

fn case(method: &'static str, uri: impl Into<String>, template: &'static str) -> Case {
    Case {
        method,
        uri: uri.into(),
        template,
        body: None,
    }
}

fn post(uri: &'static str, body: &'static str) -> Case {
    Case {
        method: "POST",
        uri: uri.to_string(),
        template: uri,
        body: Some(body),
    }
}

/// Representative requests against a router with no database and no Redis:
/// one success, malformed parameters (400), handler errors (503 envelope)
/// and fail-closed rate limiting (503 limiter body).
fn cases() -> Vec<Case> {
    vec![
        case("GET", "/api/v1/fees", "/api/v1/fees"),
        case("GET", "/api/v1", "/api/v1"),
        case("GET", "/api/v1/pools", "/api/v1/pools"),
        case("GET", "/api/v1/pools?limit=abc", "/api/v1/pools"),
        case("GET", "/api/v1/pools/1", "/api/v1/pools/{pool_id}"),
        case("GET", "/api/v1/pools/not-a-number", "/api/v1/pools/{pool_id}"),
        case("GET", "/api/v1/stats", "/api/v1/stats"),
        case("GET", "/api/v1/leaderboard", "/api/v1/leaderboard"),
        case("GET", "/api/v1/tags", "/api/v1/tags"),
        case("GET", "/api/v1/markets/1/predictions", "/api/v1/markets/{market_id}/predictions"),
        case("GET", "/api/v1/markets/abc/predictions", "/api/v1/markets/{market_id}/predictions"),
        case("GET", format!("/api/v1/referrals/{ADDRESS}"), "/api/v1/referrals/{address}"),
        case("GET", "/api/v1/referrals/not-an-address", "/api/v1/referrals/{address}"),
        case("GET", format!("/api/v1/users/{ADDRESS}/history"), "/api/v1/users/{address}/history"),
        case(
            "GET",
            format!("/api/v1/users/{ADDRESS}/predictions"),
            "/api/v1/users/{address}/predictions",
        ),
        case("GET", format!("/api/v1/creators/{ADDRESS}/stats"), "/api/v1/creators/{address}/stats"),
        post("/api/v1/indexer/pool-created", "{}"),
        post("/api/v1/indexer/prediction-placed", "{}"),
        post("/api/v1/indexer/claim", "{}"),
        post("/api/v1/auth/refresh", r#"{"refresh_token":"x"}"#),
    ]
}

#[tokio::test]
async fn live_responses_match_documented_shapes() {
    let spec = openapi_json();
    let app = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        predifi_backend::ws::EventBus::new(),
    );

    let mut failures = Vec::new();
    let mut statuses = BTreeSet::new();
    for case in cases() {
        let label = format!("{} {}", case.method, case.uri);
        let mut request = Request::builder().method(case.method).uri(&case.uri);
        if case.body.is_some() {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        let request = request
            .body(Body::from(case.body.unwrap_or_default()))
            .unwrap();
        let response = app.clone().oneshot(request).await.expect("router responds");
        let status = response.status().as_u16();
        statuses.insert(status);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or(v).trim().to_string())
            .unwrap_or_default();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();

        let Some(documented) = documented_response(&spec, case.method, case.template, status) else {
            failures.push(format!("{label}: status {status} is not documented"));
            continue;
        };
        let Some(media) = documented["content"].get(&content_type) else {
            failures.push(format!(
                "{label}: {status} returned `{content_type}`, documented: {}",
                documented["content"]
            ));
            continue;
        };
        let body: Value = if content_type == "application/json" {
            match serde_json::from_slice(&bytes) {
                Ok(json) => json,
                Err(e) => {
                    failures.push(format!("{label}: invalid JSON body: {e}"));
                    continue;
                }
            }
        } else {
            Value::String(String::from_utf8_lossy(&bytes).into_owned())
        };
        if let Err(e) = validate(&spec, &media["schema"], &body) {
            failures.push(format!("{label}: {status} body does not match schema: {e}\nbody: {body}"));
        }
    }

    assert!(failures.is_empty(), "spec drift:\n{}", failures.join("\n"));
    for expected in [200, 400, 503] {
        assert!(
            statuses.contains(&expected),
            "cases no longer exercise a {expected} response (got {statuses:?})"
        );
    }
}

#[tokio::test]
async fn served_spec_matches_generated_spec() {
    let app = build_router(
        Config::default_for_test(),
        PriceCache::new(),
        RedisCache::disabled(),
        predifi_backend::ws::EventBus::new(),
    );
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api-docs/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let served: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(served, openapi_json());
}
