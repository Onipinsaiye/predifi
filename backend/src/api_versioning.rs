//! API version lifecycle and deprecation signalling (#53).
//!
//! The policy itself lives in `docs/API_VERSIONING_POLICY.md`. This module is
//! its single machine-readable source:
//!
//! * [`API_VERSIONS`] lists every public API version with its lifecycle
//!   status, deprecation date, sunset date and successor.
//! * [`DEPRECATED_ROUTES`] lists individual operations that are deprecated
//!   inside a version that is otherwise still active.
//! * [`version_headers`] adds the headers clients rely on:
//!   - `API-Version: v1` on every `/api/<version>` response;
//!   - `Deprecation: @<unix-seconds>` (RFC 9745) once a version or route is
//!     deprecated;
//!   - `Sunset: <HTTP-date>` (RFC 8594) once a removal date is set;
//!   - `Link: <...>; rel="deprecation"` pointing to the policy, and
//!     `rel="successor-version"` when a replacement exists.
//!   After the sunset date the version or route answers
//!   `410 API_VERSION_SUNSET` in the standard error envelope instead of
//!   disappearing silently.
//! * [`versions_document`] is served by `GET /api/v1` and documented in the
//!   OpenAPI spec so clients can discover version status at runtime.

use axum::{
    extract::Request,
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::Serialize;

/// Where the deprecation policy is published (used in `Link` headers).
pub const POLICY_URL: &str =
    "https://github.com/Onipinsaiye/predifi/blob/main/docs/API_VERSIONING_POLICY.md";

/// Minimum notice between `Deprecation` and `Sunset` for a whole version.
pub const MIN_VERSION_DEPRECATION_WINDOW_DAYS: i64 = 180;

/// Minimum notice between `Deprecation` and `Sunset` for a single route
/// inside a supported version.
pub const MIN_ROUTE_DEPRECATION_WINDOW_DAYS: i64 = 90;

/// Lifecycle status of an API version or route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStatus {
    /// Fully supported; only backward-compatible changes.
    Active,
    /// Still served and supported, but scheduled for removal.
    Deprecated,
    /// Past its sunset date; requests receive `410 Gone`.
    Sunset,
}

/// One public API version.
#[derive(Debug, Clone, Copy)]
pub struct ApiVersion {
    /// Path segment, e.g. `"v1"` for `/api/v1`.
    pub name: &'static str,
    /// Unix seconds when deprecation was announced, if it has been.
    pub deprecated_at: Option<i64>,
    /// Unix seconds after which the version is removed.
    pub sunset_at: Option<i64>,
    /// Version clients should migrate to.
    pub successor: Option<&'static str>,
}

/// One deprecated operation inside an otherwise supported version.
#[derive(Debug, Clone, Copy)]
pub struct DeprecatedRoute {
    /// Upper-case HTTP method, e.g. `"GET"`.
    pub method: &'static str,
    /// Full path pattern with `:param` segments, e.g. `/api/v1/pools/:id`.
    pub path: &'static str,
    pub deprecated_at: i64,
    pub sunset_at: Option<i64>,
    /// Path of the replacement operation, e.g. `/api/v1/markets/:id`.
    pub replacement: Option<&'static str>,
}

/// Every public API version. Add new versions here; never delete an entry
/// until its sunset date has passed and the routes are removed.
pub const API_VERSIONS: &[ApiVersion] = &[ApiVersion {
    name: "v1",
    deprecated_at: None,
    sunset_at: None,
    successor: None,
}];

/// Deprecated operations. Each entry must give at least
/// [`MIN_ROUTE_DEPRECATION_WINDOW_DAYS`] of notice before `sunset_at`.
pub const DEPRECATED_ROUTES: &[DeprecatedRoute] = &[];

impl ApiVersion {
    pub fn status(&self, now: i64) -> LifecycleStatus {
        lifecycle(self.deprecated_at, self.sunset_at, now)
    }
}

impl DeprecatedRoute {
    pub fn status(&self, now: i64) -> LifecycleStatus {
        lifecycle(Some(self.deprecated_at), self.sunset_at, now)
    }

    fn matches(&self, method: &Method, path: &str) -> bool {
        if self.method != method.as_str() {
            return false;
        }
        let mut pattern = self.path.trim_end_matches('/').split('/');
        let mut actual = path.trim_end_matches('/').split('/');
        loop {
            match (pattern.next(), actual.next()) {
                (None, None) => return true,
                (Some(p), Some(a)) if p.starts_with(':') && !a.is_empty() => {}
                (Some(p), Some(a)) if p == a => {}
                _ => return false,
            }
        }
    }
}

fn lifecycle(deprecated_at: Option<i64>, sunset_at: Option<i64>, now: i64) -> LifecycleStatus {
    match (deprecated_at, sunset_at) {
        (_, Some(sunset)) if now >= sunset => LifecycleStatus::Sunset,
        (Some(deprecated), _) if now >= deprecated => LifecycleStatus::Deprecated,
        _ => LifecycleStatus::Active,
    }
}

/// Look up a version by its path segment.
pub fn find_version(name: &str) -> Option<&'static ApiVersion> {
    API_VERSIONS.iter().find(|v| v.name == name)
}

fn find_route(method: &Method, path: &str) -> Option<&'static DeprecatedRoute> {
    DEPRECATED_ROUTES.iter().find(|r| r.matches(method, path))
}

/// Extract `v1` from `/api/v1/...`.
fn version_segment(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/api/")?;
    let segment = rest.split('/').next()?;
    let is_version = segment.len() > 1
        && segment.starts_with('v')
        && segment[1..].bytes().all(|b| b.is_ascii_digit());
    is_version.then_some(segment)
}

/// `Sunset` header format (IMF-fixdate, RFC 9110).
fn http_date(unix: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp(unix, 0)
        .map(|d| d.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
}

fn rfc3339(unix: Option<i64>) -> Option<String> {
    unix.and_then(|u| DateTime::<Utc>::from_timestamp(u, 0))
        .map(|d| d.to_rfc3339())
}

fn insert_lifecycle_headers(
    headers: &mut HeaderMap,
    deprecated_at: Option<i64>,
    sunset_at: Option<i64>,
    successor_link: Option<String>,
    now: i64,
) {
    let Some(deprecated_at) = deprecated_at.filter(|d| now >= *d) else {
        return;
    };
    if let Ok(v) = HeaderValue::from_str(&format!("@{deprecated_at}")) {
        headers.insert("deprecation", v);
    }
    if let Some(v) = sunset_at
        .and_then(http_date)
        .and_then(|d| HeaderValue::from_str(&d).ok())
    {
        headers.insert("sunset", v);
    }
    let mut link = format!("<{POLICY_URL}>; rel=\"deprecation\"; type=\"text/html\"");
    if let Some(successor) = successor_link {
        link.push_str(&format!(", <{successor}>; rel=\"successor-version\""));
    }
    if let Ok(v) = HeaderValue::from_str(&link) {
        headers.append(axum::http::header::LINK, v);
    }
}

fn sunset_response(what: &str) -> Response {
    crate::response::ApiResponse::<()>::error(
        StatusCode::GONE,
        crate::response::error_codes::API_VERSION_SUNSET,
        format!("{what} has been retired; see {POLICY_URL}"),
    )
    .into_response()
}

fn now_unix() -> i64 {
    Utc::now().timestamp()
}

/// Middleware adding `API-Version`, `Deprecation`, `Sunset` and `Link`
/// headers, and answering `410` for sunset versions and routes. Must wrap the
/// top-level router so it sees the full `/api/<version>/...` path.
pub async fn version_headers(req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let Some(segment) = version_segment(&path).map(str::to_string) else {
        return next.run(req).await;
    };
    let now = now_unix();
    let version = find_version(&segment);
    let route = find_route(req.method(), &path);

    if let Some(v) = version {
        if v.status(now) == LifecycleStatus::Sunset {
            return sunset_response(&format!("API version {}", v.name));
        }
    }
    if let Some(r) = route {
        if r.status(now) == LifecycleStatus::Sunset {
            return sunset_response(&format!("{} {}", r.method, r.path));
        }
    }

    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&segment) {
        headers.insert("api-version", v);
    }

    // A deprecated route inside an active version takes precedence; when the
    // whole version is deprecated, its dates apply to every route.
    match (route, version) {
        (Some(r), _) => insert_lifecycle_headers(
            headers,
            Some(r.deprecated_at),
            r.sunset_at,
            r.replacement.map(|s| s.to_string()),
            now,
        ),
        (None, Some(v)) => insert_lifecycle_headers(
            headers,
            v.deprecated_at,
            v.sunset_at,
            v.successor.map(|s| format!("/api/{s}")),
            now,
        ),
        (None, None) => {}
    }
    response
}

/// Runtime view of one API version, served in `GET /api/v1`.
#[derive(Debug, Serialize)]
pub struct VersionInfo {
    pub version: &'static str,
    pub status: LifecycleStatus,
    pub deprecated_at: Option<String>,
    pub sunset_at: Option<String>,
    pub successor: Option<&'static str>,
}

/// Runtime view of one deprecated route.
#[derive(Debug, Serialize)]
pub struct DeprecatedRouteInfo {
    pub method: String,
    pub path: &'static str,
    pub status: LifecycleStatus,
    pub deprecated_at: Option<String>,
    pub sunset_at: Option<String>,
    pub replacement: Option<&'static str>,
}

/// Version status document for `GET /api/<version>`.
#[derive(Debug, Serialize)]
pub struct VersionsDocument {
    pub versions: Vec<VersionInfo>,
    pub deprecated_routes: Vec<DeprecatedRouteInfo>,
    pub policy: &'static str,
}

pub fn versions_document() -> VersionsDocument {
    let now = now_unix();
    VersionsDocument {
        versions: API_VERSIONS
            .iter()
            .map(|v| VersionInfo {
                version: v.name,
                status: v.status(now),
                deprecated_at: rfc3339(v.deprecated_at),
                sunset_at: rfc3339(v.sunset_at),
                successor: v.successor,
            })
            .collect(),
        deprecated_routes: DEPRECATED_ROUTES
            .iter()
            .map(|r| DeprecatedRouteInfo {
                method: r.method.to_string(),
                path: r.path,
                status: r.status(now),
                deprecated_at: rfc3339(Some(r.deprecated_at)),
                sunset_at: rfc3339(r.sunset_at),
                replacement: r.replacement,
            })
            .collect(),
        policy: POLICY_URL,
    }
}

/// Apply the API-wide policies to a top-level router: version/deprecation
/// headers, the error envelope for every failure, and an enveloped 404
/// fallback.
pub fn with_api_policies(router: axum::Router) -> axum::Router {
    router
        .fallback(crate::response::not_found_fallback)
        .layer(axum::middleware::from_fn(version_headers))
        .layer(axum::middleware::from_fn(
            crate::response::error_envelope_layer,
        ))
}
