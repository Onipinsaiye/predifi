//! Helpers for detecting drift between the OpenAPI spec and live routes (#52).
//!
//! Used by `tests/openapi_drift.rs`:
//!
//! * [`scan_routes`] extracts `(METHOD, path)` pairs from router source, so a
//!   route added to `src/routes` without documentation fails CI even though
//!   Axum offers no route introspection;
//! * [`validate`] checks a JSON value against an OpenAPI 3.1 schema (the
//!   subset utoipa emits: `$ref`, `type`, `properties`, `required`, `items`,
//!   `oneOf`/`anyOf`/`allOf`, `enum`), so live responses can be compared with
//!   the documented shapes.

use std::collections::BTreeSet;

use serde_json::Value;

const METHODS: [&str; 5] = ["get", "post", "put", "patch", "delete"];

/// Extract the routes registered with `.route("path", <method router>)` in
/// `source`, with `prefix` prepended to each path. Code after the first
/// `#[cfg(test)]` is ignored.
///
/// Methods are recognised as calls to `get(`, `post(`, `put(`, `patch(` or
/// `delete(` anywhere in the route's method-router argument, which covers
/// chains (`get(a).post(b)`) and wrappers (`guard(post(h))`).
pub fn scan_routes(source: &str, prefix: &str) -> BTreeSet<(String, String)> {
    let source = source.split("#[cfg(test)]").next().unwrap_or(source);
    let bytes = source.as_bytes();
    let mut routes = BTreeSet::new();
    let mut cursor = 0;
    while let Some(found) = source[cursor..].find(".route(") {
        let open = cursor + found + ".route".len();
        let close = matching_paren(bytes, open).unwrap_or(source.len());
        let call = &source[open + 1..close];
        cursor = close.min(source.len());

        let Some(path) = first_string_literal(call) else {
            continue;
        };
        let full = match (prefix, path.as_str()) {
            (prefix, "/") if !prefix.is_empty() => prefix.to_string(),
            (prefix, path) => format!("{prefix}{path}"),
        };
        for method in methods_called(call) {
            routes.insert((method.to_uppercase(), full.clone()));
        }
    }
    routes
}

/// Index of the `)` matching the `(` at `open`, skipping string literals.
fn matching_paren(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn first_string_literal(text: &str) -> Option<String> {
    let start = text.find('"')? + 1;
    let end = start + text[start..].find('"')?;
    Some(text[start..end].to_string())
}

fn methods_called(text: &str) -> BTreeSet<&'static str> {
    let bytes = text.as_bytes();
    let mut found = BTreeSet::new();
    for method in METHODS {
        let needle = format!("{method}(");
        let mut from = 0;
        while let Some(pos) = text[from..].find(&needle) {
            let at = from + pos;
            let preceded_by_ident = at > 0 && {
                let c = bytes[at - 1];
                c.is_ascii_alphanumeric() || c == b'_'
            };
            if !preceded_by_ident {
                found.insert(method);
            }
            from = at + needle.len();
        }
    }
    found
}

/// Normalise a path for comparison: `:id`, `{pool_id}` → `{}`.
pub fn normalize_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.starts_with(':') || (segment.starts_with('{') && segment.ends_with('}')) {
                "{}"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Resolve a local `#/components/schemas/...` reference.
fn resolve<'a>(spec: &'a Value, schema: &'a Value) -> Result<&'a Value, String> {
    match schema.get("$ref").and_then(Value::as_str) {
        Some(reference) => {
            let pointer = reference
                .strip_prefix('#')
                .ok_or_else(|| format!("unsupported $ref {reference}"))?;
            let target = spec
                .pointer(pointer)
                .ok_or_else(|| format!("unresolvable $ref {reference}"))?;
            resolve(spec, target)
        }
        None => Ok(schema),
    }
}

fn type_matches(value: &Value, ty: &str) -> bool {
    match ty {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        _ => true,
    }
}

/// Validate `value` against `schema`, resolving references within `spec`.
/// The error names the JSON path of the first mismatch.
pub fn validate(spec: &Value, schema: &Value, value: &Value) -> Result<(), String> {
    validate_at(spec, schema, value, "$")
}

fn validate_at(spec: &Value, schema: &Value, value: &Value, at: &str) -> Result<(), String> {
    let schema = resolve(spec, schema)?;

    if let Some(types) = schema.get("type") {
        let ok = match types {
            Value::String(ty) => type_matches(value, ty),
            Value::Array(tys) => tys
                .iter()
                .filter_map(Value::as_str)
                .any(|ty| type_matches(value, ty)),
            _ => true,
        };
        if !ok {
            return Err(format!("{at}: expected type {types}, got {value}"));
        }
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        if !options.contains(value) {
            return Err(format!("{at}: {value} not in enum {options:?}"));
        }
    }
    if let Some(all) = schema.get("allOf").and_then(Value::as_array) {
        for sub in all {
            validate_at(spec, sub, value, at)?;
        }
    }
    for keyword in ["oneOf", "anyOf"] {
        if let Some(options) = schema.get(keyword).and_then(Value::as_array) {
            let errors: Vec<String> = options
                .iter()
                .filter_map(|sub| validate_at(spec, sub, value, at).err())
                .collect();
            if errors.len() == options.len() {
                return Err(format!("{at}: matches no {keyword} option: {errors:?}"));
            }
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for field in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(field) {
                    return Err(format!("{at}: missing required field `{field}`"));
                }
            }
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (field, sub) in properties {
                if let Some(child) = object.get(field) {
                    validate_at(spec, sub, child, &format!("{at}.{field}"))?;
                }
            }
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (i, item) in array.iter().enumerate() {
            validate_at(spec, items, item, &format!("{at}[{i}]"))?;
        }
    }
    Ok(())
}

/// The documented response object for `method path status`, if any.
/// `path` is matched after [`normalize_path`].
pub fn documented_response<'a>(
    spec: &'a Value,
    method: &str,
    path: &str,
    status: u16,
) -> Option<&'a Value> {
    let wanted = normalize_path(path);
    let (_, item) = spec["paths"]
        .as_object()?
        .iter()
        .find(|(p, _)| normalize_path(p) == wanted)?;
    item.get(method.to_lowercase())?
        .get("responses")?
        .get(status.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scans_routes_chains_wrappers_and_prefix() {
        let source = r#"
            let r = Router::new()
                .route("/", get(index))
                .route("/pools/:id", get(get_pool_by_id_handler))
                .route(
                    "/users/:address/interests",
                    get(get_user_interests_handler)
                        .put(set_user_interests_handler)
                        .layer(DefaultBodyLimit::max(c::MAX)),
                )
                .route(
                    "/indexer/claim",
                    idempotent("claims.submit", post(ingest_claim_handler)),
                )
                .route("/prices", get(crate::price_cache::get_prices));
            #[cfg(test)]
            mod tests { fn x() { Router::new().route("/test-only", get(h)); } }
        "#;
        let routes = scan_routes(source, "/api/v1");
        let expected: BTreeSet<(String, String)> = [
            ("GET", "/api/v1"),
            ("GET", "/api/v1/pools/:id"),
            ("GET", "/api/v1/users/:address/interests"),
            ("PUT", "/api/v1/users/:address/interests"),
            ("POST", "/api/v1/indexer/claim"),
            ("GET", "/api/v1/prices"),
        ]
        .into_iter()
        .map(|(m, p)| (m.to_string(), p.to_string()))
        .collect();
        assert_eq!(routes, expected);
    }

    #[test]
    fn normalizes_parameter_names() {
        assert_eq!(normalize_path("/api/v1/pools/:id"), "/api/v1/pools/{}");
        assert_eq!(normalize_path("/api/v1/pools/{pool_id}"), "/api/v1/pools/{}");
    }

    #[test]
    fn validates_refs_types_required_and_one_of() {
        let spec = json!({
            "components": {"schemas": {
                "Err": {
                    "type": "object",
                    "required": ["status", "error"],
                    "properties": {
                        "status": {"type": "string"},
                        "error": {"type": "object", "required": ["code"],
                                  "properties": {"code": {"type": "string"}}}
                    }
                },
                "Limit": {
                    "type": "object",
                    "required": ["status", "error"],
                    "properties": {"status": {"type": "string"}, "error": {"type": "string"}}
                },
                "Either": {"oneOf": [{"$ref": "#/components/schemas/Err"},
                                     {"$ref": "#/components/schemas/Limit"}]},
                "Opt": {"type": "object", "properties": {"n": {"type": ["integer", "null"]}}}
            }}
        });
        let either = json!({"$ref": "#/components/schemas/Either"});
        assert!(validate(&spec, &either, &json!({"status": "error", "error": {"code": "X"}})).is_ok());
        assert!(validate(&spec, &either, &json!({"status": "error", "error": "limited"})).is_ok());
        assert!(validate(&spec, &either, &json!({"status": "error", "error": 3})).is_err());
        let err = validate(&spec, &json!({"$ref": "#/components/schemas/Err"}), &json!({"error": {}}));
        assert!(err.unwrap_err().contains("missing required field `status`"));
        let opt = json!({"$ref": "#/components/schemas/Opt"});
        assert!(validate(&spec, &opt, &json!({"n": null})).is_ok());
        assert!(validate(&spec, &opt, &json!({"n": "x"})).is_err());
    }
}
