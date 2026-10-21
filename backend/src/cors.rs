//! CORS configuration and middleware distinguishing public read routes from credentialed routes.
//!
//! - **Public read routes** (e.g. `/health`, `/metrics`, `/api/v1/pools`, `/api/v1/prices`):
//!   Serve public, non-sensitive data. They permit access from configured allowed origins
//!   without sending `Access-Control-Allow-Credentials`.
//! - **Credentialed routes** (e.g. `/api/v1/auth/*`, `/api/v1/users/*`, `/api/v1/admin/*`, `/indexer/*`):
//!   Serve authenticated or mutating actions. They strictly require credentials
//!   (`Access-Control-Allow-Credentials: true`) and ONLY permit explicitly configured,
//!   non-wildcard origins. Wildcard `*` and empty origin configurations fail closed and never
//!   expose credentialed routes to arbitrary sites.

use crate::config::Config;
use http::HeaderValue;
use std::task::{Context, Poll};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_layer::Layer;
use tower_service::Service;

/// Returns true if the HTTP method and path constitute a public read route.
pub fn is_public_read_route(method: &http::Method, path: &str) -> bool {
    // Only safe/read methods can ever be public read routes.
    if method != http::Method::GET && method != http::Method::HEAD && method != http::Method::OPTIONS {
        return false;
    }

    // Root and system health/probe routes
    if path == "/"
        || path == "/live"
        || path == "/liveness"
        || path == "/health"
        || path == "/ready"
        || path == "/readiness"
        || path == "/metrics"
        || path == "/api/healthz"
        || path == "/api/readyz"
        || path.starts_with("/swagger-ui")
        || path.starts_with("/api-docs")
    {
        return true;
    }

    // Public read endpoints under /api/v1
    if path == "/api/v1"
        || path == "/api/v1/"
        || path == "/api/v1/health"
        || path == "/api/v1/fees"
        || path == "/api/v1/prices"
        || path == "/api/v1/stats"
        || path == "/api/v1/leaderboard"
        || path == "/api/v1/tags"
        || path == "/api/v1/pool-templates"
    {
        return true;
    }

    // /api/v1/pools and /api/v1/pools/:id and /api/v1/pools/:id/leaderboard are public read,
    // but mutating/credentialed subroutes like /pools/:id/tags or /pools/:id/pay-creator-incentive are not.
    if path == "/api/v1/pools" || path.starts_with("/api/v1/pools/") {
        if path.ends_with("/tags") || path.ends_with("/pay-creator-incentive") {
            return false;
        }
        return true;
    }

    if path.starts_with("/api/v1/referrals/") {
        return true;
    }

    if path.starts_with("/api/v1/markets/") && path.ends_with("/predictions") {
        return true;
    }

    if path.starts_with("/api/v1/creators/") && path.ends_with("/stats") {
        return true;
    }

    false
}

/// Determine whether an inbound request targets a public read route.
pub fn is_request_public_read<B>(req: &http::Request<B>) -> bool {
    let method = if req.method() == http::Method::OPTIONS {
        if let Some(req_method) = req
            .headers()
            .get(http::header::ACCESS_CONTROL_REQUEST_METHOD)
            .and_then(|v| v.to_str().ok())
            .and_then(|m| http::Method::from_bytes(m.as_bytes()).ok())
        {
            req_method
        } else {
            http::Method::OPTIONS
        }
    } else {
        req.method().clone()
    };

    is_public_read_route(&method, req.uri().path())
}

/// Build the CORS middleware layer for public read routes.
///
/// Public read routes allow access from configured origins without exposing credentials.
pub fn build_public_cors(config: &Config) -> CorsLayer {
    let origins: Vec<HeaderValue> = config
        .cors_allowed_origins
        .iter()
        .filter_map(|origin| origin.parse().ok())
        .collect();

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([
            http::Method::GET,
            http::Method::HEAD,
            http::Method::OPTIONS,
        ])
        .allow_headers([
            http::header::CONTENT_TYPE,
            http::header::ACCEPT,
            http::header::HeaderName::from_static("traceparent"),
            http::header::HeaderName::from_static("tracestate"),
        ])
}

/// Build the CORS middleware layer for credentialed/authenticated routes.
///
/// Credentialed routes require explicit allowed origins and send
/// `Access-Control-Allow-Credentials: true`. Wildcards (`*`) and empty origin lists
/// are strictly forbidden and will never allow arbitrary sites.
pub fn build_credentialed_cors(config: &Config) -> CorsLayer {
    let origins: Vec<HeaderValue> = config
        .cors_allowed_origins
        .iter()
        .filter(|origin| {
            let o = origin.trim();
            o != "*" && !o.contains('*') && !o.eq_ignore_ascii_case("null")
        })
        .filter_map(|origin| origin.parse().ok())
        .collect();

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_credentials(true)
        .allow_methods([
            http::Method::GET,
            http::Method::POST,
            http::Method::PUT,
            http::Method::DELETE,
            http::Method::PATCH,
            http::Method::OPTIONS,
        ])
        .allow_headers([
            http::header::CONTENT_TYPE,
            http::header::AUTHORIZATION,
            http::header::ACCEPT,
            http::header::HeaderName::from_static("traceparent"),
            http::header::HeaderName::from_static("tracestate"),
            http::header::HeaderName::from_static(crate::idempotency::IDEMPOTENCY_KEY_HEADER),
        ])
}

/// Backward-compatible general CORS layer builder.
pub fn build_cors(config: &Config) -> CorsLayer {
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
            http::Method::PATCH,
            http::Method::OPTIONS,
        ])
        .allow_headers([
            http::header::CONTENT_TYPE,
            http::header::AUTHORIZATION,
            http::header::ACCEPT,
            http::header::HeaderName::from_static("traceparent"),
            http::header::HeaderName::from_static("tracestate"),
            http::header::HeaderName::from_static(crate::idempotency::IDEMPOTENCY_KEY_HEADER),
        ])
}

/// Dynamic CORS layer that applies public read CORS or credentialed CORS based
/// on the target endpoint.
#[derive(Clone)]
pub struct DynamicCorsLayer {
    public_cors: CorsLayer,
    credentialed_cors: CorsLayer,
}

impl DynamicCorsLayer {
    pub fn new(config: &Config) -> Self {
        Self {
            public_cors: build_public_cors(config),
            credentialed_cors: build_credentialed_cors(config),
        }
    }
}

impl<S: Clone> Layer<S> for DynamicCorsLayer {
    type Service = DynamicCorsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        DynamicCorsService {
            public_service: self.public_cors.layer(inner.clone()),
            credentialed_service: self.credentialed_cors.layer(inner),
        }
    }
}

#[derive(Clone)]
pub struct DynamicCorsService<S> {
    public_service: tower_http::cors::Cors<S>,
    credentialed_service: tower_http::cors::Cors<S>,
}

impl<S, ReqBody> Service<http::Request<ReqBody>> for DynamicCorsService<S>
where
    S: Service<http::Request<ReqBody>> + Clone,
    tower_http::cors::Cors<S>: Service<http::Request<ReqBody>>,
{
    type Response = <tower_http::cors::Cors<S> as Service<http::Request<ReqBody>>>::Response;
    type Error = <tower_http::cors::Cors<S> as Service<http::Request<ReqBody>>>::Error;
    type Future = <tower_http::cors::Cors<S> as Service<http::Request<ReqBody>>>::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.public_service.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        if is_request_public_read(&req) {
            self.public_service.call(req)
        } else {
            self.credentialed_service.call(req)
        }
    }
}
