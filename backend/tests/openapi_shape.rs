use serde_json::value::Value;
use utoipa::OpenApi;

use predifi_backend::openapi::ApiDoc;

/// Validate the OpenAPI document produced by the backend has the
/// shape required for client generation and contract testing.
///
/// This test is intentionally stricter than a smoke test: it checks that
/// the spec is stable enough to drive a client generator (components
/// schemas, response codes, and request bodies) and that every path
/// declares at least one response so runtime responses can be compared
/// against the published schema.
#[test]
fn openapi_document_has_required_shape() {
    let spec: Value =
        serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI spec must serialize to JSON");

    assert!(
        spec["info"]["title"]
            .as_str()
            .is_some_and(|title| !title.trim().is_empty()),
        "OpenAPI info.title must be non-empty, got: {spec:?}"
    );

    assert!(
        spec["info"]["version"]
            .as_str()
            .is_some_and(|version| !version.trim().is_empty()),
        "OpenAPI info.version must be non-empty, got: {spec:?}"
    );

    assert!(
        spec["paths"]
            .as_object()
            .is_some_and(|paths| !paths.is_empty()),
        "OpenAPI paths must contain at least one endpoint, got: {spec:?}"
    );

    // Client generators require a stable components section to derive
    // typed request/response models. A missing or empty section would
    // silently produce `unknown` types in the generated client.
    assert!(
        spec["components"]["schemas"]
            .as_object()
            .is_some_and(|components| !components.is_empty()),
        "OpenAPI components.schemas must be non-empty, got: {spec:?}"
    );

    // The quote endpoint must be published so clients can request a
    // deterministic estimate before signing.
    let paths = spec["paths"].as_object().expect("paths must be an object");
    let quote_path = paths
        .keys()
        .find(|path| path.contains("quote"))
        .unwrap_or_else(|| panic!("OpenAPI paths must include a quote endpoint, got: {spec:?}"));
    let quote_item = paths[quote_path]
        .as_object()
        .unwrap_or_else(|| panic!("quote path {quote_path} must be an object"));
    let quote_op = quote_item
        .get("get")
        .unwrap_or_else(|| panic!("quote path {quote_path} must declare a GET operation"));
    assert!(
        quote_op["responses"]
            .as_object()
            .is_some_and(|responses| !responses.is_empty()),
        "quote operation must declare responses, got: {quote_op:?}"
    );

    // The quote request/response models must be published as components so
    // generated clients can derive typed models for them.
    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("components.schemas must be an object");
    for required in ["QuoteRequest", "QuoteResponse"] {
        assert!(
            schemas.contains_key(required),
            "OpenAPI components.schemas must include {required}, got: {spec:?}"
        );
    }

    // Every path must declare at least one operation and each operation
    // must declare responses so runtime responses can be validated against
    // the published schema.
    for (path, item) in paths {
        let item_obj = item
            .as_object()
            .unwrap_or_else(|| panic!("path {path} must be an object, got: {item:?}"));
        let operations: Vec<(&String, &Value)> = item_obj
            .iter()
            .filter(|(method, _)| {
                matches!(
                    method.as_str(),
                    "get" | "put" | "post" | "delete" | "options" | "head" | "patch" | "trace"
                )
            })
            .collect();
        assert!(
            !operations.is_empty(),
            "path {path} must declare at least one HTTP operation, got: {item:?}"
        );
        for (method, op) in operations {
            let responses = op["responses"].as_object().unwrap_or_else(|| {
                panic!(
                    "operation {} {} must declare responses, got: {op:?}",
                    method.to_uppercase(),
                    path
                )
            });
            assert!(
                !responses.is_empty(),
                "operation {} {} must declare at least one response, got: {op:?}",
                method.to_uppercase(),
                path
            );
        }
    }
}
