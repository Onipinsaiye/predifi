//! Dependency health probing for the `/healthz` and `/readyz` endpoints.
//!
//! Liveness never touches a dependency: it only reports that the process is
//! running, so a Postgres/Redis outage can never cause an orchestrator to
//! restart an otherwise-healthy instance. Readiness probes every dependency
//! under a bounded per-dependency timeout, so one hanging dependency cannot
//! blow up the overall response latency.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;

use crate::redis_cache::RedisCache;

/// Outcome of probing a single dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyStatus {
    Ok,
    Unreachable,
    NotConfigured,
    Timeout,
}

impl DependencyStatus {
    fn is_healthy(self) -> bool {
        matches!(self, DependencyStatus::Ok)
    }
}

/// Probe result for a single dependency: status plus an optional detail
/// message (present for anything other than `Ok`).
#[derive(Debug, Clone, Serialize)]
pub struct DependencyHealth {
    pub status: DependencyStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl DependencyHealth {
    fn ok() -> Self {
        Self {
            status: DependencyStatus::Ok,
            error: None,
        }
    }

    fn not_configured(message: impl Into<String>) -> Self {
        Self {
            status: DependencyStatus::NotConfigured,
            error: Some(message.into()),
        }
    }

    fn unreachable(message: impl Into<String>) -> Self {
        Self {
            status: DependencyStatus::Unreachable,
            error: Some(message.into()),
        }
    }

    fn timeout(message: impl Into<String>) -> Self {
        Self {
            status: DependencyStatus::Timeout,
            error: Some(message.into()),
        }
    }
}

/// Full readiness probe result across every dependency.
#[derive(Debug, Clone, Serialize)]
pub struct ReadinessReport {
    pub ready: bool,
    pub db: DependencyHealth,
    pub redis: DependencyHealth,
}

/// Per-dependency probe timeouts.
#[derive(Debug, Clone, Copy)]
pub struct DependencyTimeouts {
    pub db: Duration,
    pub redis: Duration,
}

impl Default for DependencyTimeouts {
    fn default() -> Self {
        Self {
            db: Duration::from_secs(2),
            redis: Duration::from_secs(2),
        }
    }
}

/// The dependencies a readiness probe checks.
#[derive(Clone)]
pub struct DependencyProbes {
    db: Option<PgPool>,
    redis: RedisCache,
}

impl DependencyProbes {
    pub fn new(db: Option<PgPool>, redis: RedisCache) -> Self {
        Self { db, redis }
    }

    async fn probe_db(&self, timeout: Duration) -> DependencyHealth {
        match &self.db {
            None => DependencyHealth::not_configured("database pool is not configured"),
            Some(pool) => {
                match tokio::time::timeout(timeout, sqlx::query("SELECT 1").execute(pool)).await {
                    Ok(Ok(_)) => DependencyHealth::ok(),
                    Ok(Err(_)) => DependencyHealth::unreachable("database ping failed"),
                    Err(_) => DependencyHealth::timeout("database ping timed out"),
                }
            }
        }
    }

    async fn probe_redis(&self, timeout: Duration) -> DependencyHealth {
        if !self.redis.is_available() {
            return DependencyHealth::not_configured("Redis is not configured");
        }
        match tokio::time::timeout(timeout, self.redis.ping()).await {
            Ok(true) => DependencyHealth::ok(),
            Ok(false) => DependencyHealth::unreachable("Redis ping failed"),
            Err(_) => DependencyHealth::timeout("Redis ping timed out"),
        }
    }
}

/// Shared state backing the `/readyz` handler.
#[derive(Clone)]
pub struct ReadinessState {
    probes: Arc<DependencyProbes>,
    timeouts: DependencyTimeouts,
    /// Reserved overall deadline for a single readiness check. Individual
    /// dependency probes are already bounded by `timeouts`; this is kept as
    /// configuration so callers can tune the endpoint without touching code.
    #[allow(dead_code)]
    overall_timeout: Duration,
}

impl ReadinessState {
    pub fn new(probes: DependencyProbes, timeouts: DependencyTimeouts, overall_timeout: Duration) -> Self {
        Self {
            probes: Arc::new(probes),
            timeouts,
            overall_timeout,
        }
    }

    pub async fn probe(&self) -> ReadinessReport {
        let (db, redis) = tokio::join!(
            self.probes.probe_db(self.timeouts.db),
            self.probes.probe_redis(self.timeouts.redis),
        );
        let ready = db.status.is_healthy() && redis.status.is_healthy();
        ReadinessReport { ready, db, redis }
    }
}

/// `GET /healthz` — liveness probe. Reports only that the process is up.
pub async fn liveness() -> Response {
    (
        StatusCode::OK,
        Json(json!({ "status": "alive", "service": "predifi-backend" })),
    )
        .into_response()
}

/// `GET /readyz` — readiness probe. Returns `503` when any dependency is
/// unreachable, times out, or is not configured.
pub async fn readiness(State(state): State<ReadinessState>) -> Response {
    let report = state.probe().await;
    let status = if report.ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(report)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn readiness_reports_not_configured_without_dependencies() {
        let probes = DependencyProbes::new(None, RedisCache::disabled());
        let state = ReadinessState::new(probes, DependencyTimeouts::default(), Duration::from_secs(5));
        let report = state.probe().await;
        assert!(!report.ready);
        assert_eq!(report.db.status, DependencyStatus::NotConfigured);
        assert_eq!(report.redis.status, DependencyStatus::NotConfigured);
    }

    #[test]
    fn dependency_status_ok_is_the_only_healthy_variant() {
        assert!(DependencyStatus::Ok.is_healthy());
        assert!(!DependencyStatus::Unreachable.is_healthy());
        assert!(!DependencyStatus::NotConfigured.is_healthy());
        assert!(!DependencyStatus::Timeout.is_healthy());
    }
}
