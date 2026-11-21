//! Portfolio routes: normalized wallet activity (#309), centralized P&L
//! (#310), checkpoint-gated snapshots (#311), bounded asynchronous exports
//! (#312), and historical market-value time series (#313).
//!
//! Handlers here share [`crate::routes::v1::AppState`] with the rest of the
//! v1 API; they are mounted from `crate::routes::v1::router`.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::db::{
    calculate_portfolio_pnl, calculate_position_pnl, cancel_export, create_export,
    generate_portfolio_snapshot, get_export_artifact, get_export_status, get_market_history,
    get_wallet_activity, ActivityCursor, ArtifactResult, BucketWidth, CancelOutcome,
    MarketHistoryError, PoolLifecycle, PositionInputs, SnapshotError, DEFAULT_ACTIVITY_PAGE_SIZE,
    MAX_ACTIVITY_PAGE_SIZE,
};
use crate::response::{error_codes, ApiResponse};
use crate::routes::v1::AppState;
use crate::validated_types::StellarAddress;

// ── Shared helpers ────────────────────────────────────────────────────────

fn invalid_address(err: impl std::fmt::Display) -> Response {
    ApiResponse::<()>::error(StatusCode::BAD_REQUEST, error_codes::INVALID_INPUT, err.to_string())
        .into_response()
}

fn database_unavailable() -> Response {
    ApiResponse::<()>::error(
        StatusCode::SERVICE_UNAVAILABLE,
        error_codes::DATABASE_UNAVAILABLE,
        "database not available",
    )
    .into_response()
}

fn internal_error(err: impl std::fmt::Display) -> Response {
    ApiResponse::<()>::error(StatusCode::INTERNAL_SERVER_ERROR, error_codes::INTERNAL_ERROR, err.to_string())
        .into_response()
}

/// Require a `Bearer` JWT whose subject matches `wallet_address`, so only
/// the wallet's own owner can trigger a wallet-scoped mutation (exports).
/// Follows the same extract-and-verify pattern used by
/// `routes::v1::logout_handler` and `idempotency::wallet`.
fn authorize_wallet_action(
    state: &AppState,
    headers: &HeaderMap,
    wallet_address: &str,
) -> Result<(), Response> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(crate::jwt::extract_bearer_token)
        .ok_or_else(|| {
            ApiResponse::<()>::error(
                StatusCode::UNAUTHORIZED,
                error_codes::UNAUTHORIZED,
                "missing bearer token",
            )
            .into_response()
        })?;

    let claims = crate::jwt::verify_jwt_token_strict(
        token,
        &state.config.secret_key,
        "access",
        state.config.jwt_key_version,
    )
    .map_err(|e| crate::errors::AppError::from(e).into_response())?;

    if claims.sub != wallet_address {
        return Err(ApiResponse::<()>::error(
            StatusCode::FORBIDDEN,
            error_codes::FORBIDDEN,
            "token does not authorize this wallet",
        )
        .into_response());
    }
    Ok(())
}

// ── #309: wallet activity ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    pub after: Option<String>,
    pub limit: Option<i64>,
}

/// `GET /v1/wallets/:address/activity` — normalized wallet activity feed.
///
/// Public: on-chain activity is public data, so no auth is required. The
/// address is still validated as a well-formed Stellar address.
pub async fn get_wallet_activity_handler(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Query(params): Query<ActivityQuery>,
) -> Response {
    let wallet = match StellarAddress::new(address) {
        Ok(w) => w,
        Err(e) => return invalid_address(e),
    };
    let Some(db) = &state.db else {
        return database_unavailable();
    };

    let after = match params.after.as_deref() {
        Some(raw) => match ActivityCursor::decode(raw) {
            Some(cursor) => Some(cursor),
            None => {
                return ApiResponse::<()>::error(
                    StatusCode::BAD_REQUEST,
                    error_codes::INVALID_INPUT,
                    "invalid cursor",
                )
                .into_response()
            }
        },
        None => None,
    };
    let limit = params
        .limit
        .unwrap_or(DEFAULT_ACTIVITY_PAGE_SIZE)
        .clamp(1, MAX_ACTIVITY_PAGE_SIZE);

    match get_wallet_activity(db, wallet.as_str(), after, limit).await {
        Ok(page) => ApiResponse::success(page).into_response(),
        Err(e) => internal_error(e),
    }
}

// ── #310: centralized P&L ────────────────────────────────────────────────

/// `GET /v1/wallets/:address/pnl` — centralized, typed portfolio P&L.
///
/// Public, same as `activity`: a wallet's P&L is derived entirely from
/// public on-chain data.
pub async fn get_wallet_pnl_handler(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Response {
    let wallet = match StellarAddress::new(address) {
        Ok(w) => w,
        Err(e) => return invalid_address(e),
    };
    let Some(db) = &state.db else {
        return database_unavailable();
    };

    let claims = match crate::profile::get_user_claim_status(db, wallet.as_str()).await {
        Ok(rows) => rows,
        Err(e) => return internal_error(e),
    };

    let mut by_pool = Vec::with_capacity(claims.len());
    for row in &claims {
        let pool_state = PoolLifecycle::from_db_state(&row.pool_state);
        let winning_outcome = row.pool_result.as_deref().and_then(|s| s.parse::<i32>().ok());

        // The pool-total/winning-stake lookup is only needed to estimate an
        // unclaimed winning position; a loss, an unresolved pool, or a
        // claimed position never needs it.
        let (pool_total_stake, winning_outcome_stake) = if !row.claimed
            && pool_state == PoolLifecycle::Settled
            && winning_outcome == Some(row.outcome)
        {
            let total = crate::db::get_pool_by_id(db, row.pool_id)
                .await
                .ok()
                .flatten()
                .map(|p| p.total_stake)
                .unwrap_or(0);
            let winning = crate::db::get_pool_outcome_stakes(db, row.pool_id)
                .await
                .ok()
                .unwrap_or_default()
                .into_iter()
                .find(|(outcome, _)| Some(*outcome) == winning_outcome)
                .map(|(_, stake)| stake)
                .unwrap_or(0);
            (total, winning)
        } else {
            (0, 0)
        };

        by_pool.push((
            row.pool_id,
            PositionInputs {
                stake: row.amount,
                pool_total_stake,
                winning_outcome_stake,
                user_outcome: row.outcome,
                winning_outcome,
                pool_state,
                treasury_fee_bps: state.config.treasury_fee_bps,
                claimed: row.claimed,
                claimed_amount: row.claimed_amount,
            },
        ));
    }

    let positions: Vec<serde_json::Value> = by_pool
        .iter()
        .map(|(pool_id, inputs)| {
            serde_json::json!({
                "pool_id": pool_id,
                "pnl": calculate_position_pnl(inputs),
            })
        })
        .collect();
    let inputs_only: Vec<PositionInputs> = by_pool.iter().map(|(_, inputs)| *inputs).collect();
    let portfolio = calculate_portfolio_pnl(&inputs_only);

    ApiResponse::success(serde_json::json!({
        "address": wallet.as_str(),
        "positions": positions,
        "portfolio": portfolio,
    }))
    .into_response()
}

// ── #311: checkpoint-gated snapshot ──────────────────────────────────────

/// `GET /v1/wallets/:address/snapshot` — portfolio snapshot generated (or
/// regenerated) against the most recently committed indexer checkpoint.
pub async fn get_wallet_snapshot_handler(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Response {
    let wallet = match StellarAddress::new(address) {
        Ok(w) => w,
        Err(e) => return invalid_address(e),
    };
    let Some(db) = &state.db else {
        return database_unavailable();
    };

    match generate_portfolio_snapshot(db, wallet.as_str()).await {
        Ok(snapshot) => ApiResponse::success(snapshot).into_response(),
        Err(SnapshotError::CheckpointNotReady(reason)) => ApiResponse::<()>::error(
            StatusCode::SERVICE_UNAVAILABLE,
            error_codes::SERVICE_UNAVAILABLE,
            format!("indexer checkpoint not ready: {reason}"),
        )
        .into_response(),
        Err(SnapshotError::Database(e)) => internal_error(e),
    }
}

// ── #312: bounded asynchronous exports ───────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct CreateExportRequest {
    #[serde(default)]
    pub row_limit: Option<i64>,
    #[serde(default)]
    pub time_limit_secs: Option<i64>,
}

/// `POST /v1/wallets/:address/exports` — create a bounded portfolio export
/// job. Requires a bearer token authorizing `address` (wallet-owned
/// action); the job is processed asynchronously by the background worker.
pub async fn create_export_handler(
    State(state): State<AppState>,
    Path(address): Path<String>,
    headers: HeaderMap,
    body: Option<Json<CreateExportRequest>>,
) -> Response {
    let wallet = match StellarAddress::new(address) {
        Ok(w) => w,
        Err(e) => return invalid_address(e),
    };
    if let Err(resp) = authorize_wallet_action(&state, &headers, wallet.as_str()) {
        return resp;
    }
    let Some(db) = &state.db else {
        return database_unavailable();
    };

    let body = body.map(|Json(b)| b).unwrap_or_default();
    match create_export(db, wallet.as_str(), body.row_limit, body.time_limit_secs).await {
        Ok(view) => (StatusCode::ACCEPTED, Json(ApiResponseSuccess::new(view))).into_response(),
        Err(e) => internal_error(e),
    }
}

/// `GET /v1/portfolio/exports/:id` — export status/progress. Requires a
/// bearer token authorizing the export's owning wallet.
pub async fn get_export_status_handler(
    State(state): State<AppState>,
    Path(export_id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    let Some(db) = &state.db else {
        return database_unavailable();
    };
    match get_export_status(db, export_id).await {
        Ok(Some((owner, view))) => {
            if let Err(resp) = authorize_wallet_action(&state, &headers, &owner) {
                return resp;
            }
            ApiResponse::success(view).into_response()
        }
        Ok(None) => {
            ApiResponse::<()>::error(StatusCode::NOT_FOUND, error_codes::NOT_FOUND, "export not found")
                .into_response()
        }
        Err(e) => internal_error(e),
    }
}

/// `GET /v1/portfolio/exports/:id/download` — download the finished
/// artifact. Returns `404` if not ready, `410` if expired, never the
/// expired file itself.
pub async fn download_export_handler(
    State(state): State<AppState>,
    Path(export_id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    let Some(db) = &state.db else {
        return database_unavailable();
    };
    match get_export_artifact(db, export_id).await {
        Ok(Some(ArtifactResult::Available { owner, content })) => {
            if let Err(resp) = authorize_wallet_action(&state, &headers, &owner) {
                return resp;
            }
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "text/csv; charset=utf-8")],
                content,
            )
                .into_response()
        }
        Ok(Some(ArtifactResult::NotReady)) => ApiResponse::<()>::error(
            StatusCode::NOT_FOUND,
            error_codes::NOT_FOUND,
            "export is not ready yet",
        )
        .into_response(),
        Ok(Some(ArtifactResult::Expired)) => ApiResponse::<()>::error(
            StatusCode::GONE,
            error_codes::RESOURCE_EXPIRED,
            "export artifact has expired",
        )
        .into_response(),
        Ok(None) => {
            ApiResponse::<()>::error(StatusCode::NOT_FOUND, error_codes::NOT_FOUND, "export not found")
                .into_response()
        }
        Err(e) => internal_error(e),
    }
}

/// `POST /v1/portfolio/exports/:id/cancel` — cancel a pending/running
/// export. Requires a bearer token authorizing the export's owning wallet.
pub async fn cancel_export_handler(
    State(state): State<AppState>,
    Path(export_id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    let Some(db) = &state.db else {
        return database_unavailable();
    };
    let owner = match get_export_status(db, export_id).await {
        Ok(Some((owner, _))) => owner,
        Ok(None) => {
            return ApiResponse::<()>::error(StatusCode::NOT_FOUND, error_codes::NOT_FOUND, "export not found")
                .into_response()
        }
        Err(e) => return internal_error(e),
    };
    if let Err(resp) = authorize_wallet_action(&state, &headers, &owner) {
        return resp;
    }

    match cancel_export(db, export_id, &owner).await {
        Ok(CancelOutcome::Cancelled) => {
            ApiResponse::success(serde_json::json!({ "status": "cancelled" })).into_response()
        }
        Ok(CancelOutcome::AlreadyTerminal) => ApiResponse::<()>::error(
            StatusCode::CONFLICT,
            error_codes::CONFLICT,
            "export already reached a terminal state",
        )
        .into_response(),
        Ok(CancelOutcome::NotFound) => {
            ApiResponse::<()>::error(StatusCode::NOT_FOUND, error_codes::NOT_FOUND, "export not found")
                .into_response()
        }
        Ok(CancelOutcome::Forbidden) => {
            ApiResponse::<()>::error(StatusCode::FORBIDDEN, error_codes::FORBIDDEN, "not authorized")
                .into_response()
        }
        Err(e) => internal_error(e),
    }
}

/// Minimal wrapper so `create_export_handler` can answer `202 Accepted`
/// (the job is accepted for async processing, not yet complete) while
/// reusing the standard success envelope shape.
#[derive(Serialize)]
struct ApiResponseSuccess<T: Serialize> {
    status: &'static str,
    data: T,
}

impl<T: Serialize> ApiResponseSuccess<T> {
    fn new(data: T) -> Self {
        Self {
            status: "success",
            data,
        }
    }
}

// ── #313/#314: historical market values ──────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct MarketHistoryQuery {
    /// Bucket width: `1m` | `5m` | `1h` | `1d`.
    pub bucket: Option<String>,
    /// Inclusive start of the range, RFC 3339.
    pub from: Option<String>,
    /// Exclusive end of the range, RFC 3339.
    pub to: Option<String>,
}

fn parse_rfc3339_utc(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// `GET /v1/markets/:id/history` — bucketed historical market-value
/// samples for a pool (#313), sourced from the canonical
/// `portfolio_activity_ledger` projection, with transparent hot/cold tiered
/// storage (#314). See `crate::db::market_history` for the bucket-width,
/// timestamp and missing-sample contract.
///
/// Public: like the other market/pool read endpoints, historical market
/// value is derived entirely from public on-chain activity.
pub async fn get_market_history_handler(
    State(state): State<AppState>,
    Path(pool_id): Path<i64>,
    Query(params): Query<MarketHistoryQuery>,
) -> Response {
    let Some(width) = params.bucket.as_deref().and_then(BucketWidth::parse) else {
        return ApiResponse::<()>::error(
            StatusCode::BAD_REQUEST,
            error_codes::INVALID_INPUT,
            "bucket must be one of 1m, 5m, 1h, 1d",
        )
        .into_response();
    };
    let Some(from) = params.from.as_deref().and_then(parse_rfc3339_utc) else {
        return ApiResponse::<()>::error(
            StatusCode::BAD_REQUEST,
            error_codes::INVALID_INPUT,
            "from must be an RFC3339 timestamp",
        )
        .into_response();
    };
    let Some(to) = params.to.as_deref().and_then(parse_rfc3339_utc) else {
        return ApiResponse::<()>::error(
            StatusCode::BAD_REQUEST,
            error_codes::INVALID_INPUT,
            "to must be an RFC3339 timestamp",
        )
        .into_response();
    };

    let Some(db) = &state.db else {
        return database_unavailable();
    };

    match get_market_history(db, pool_id, width, from, to).await {
        Ok(page) => ApiResponse::success(page).into_response(),
        Err(MarketHistoryError::InvalidRange) => ApiResponse::<()>::error(
            StatusCode::BAD_REQUEST,
            error_codes::INVALID_INPUT,
            "from must be before to",
        )
        .into_response(),
        Err(MarketHistoryError::RangeTooLarge {
            max_buckets,
            requested_buckets,
        }) => ApiResponse::<()>::error(
            StatusCode::BAD_REQUEST,
            error_codes::INVALID_INPUT,
            format!(
                "requested range spans {requested_buckets} buckets, exceeding the maximum of {max_buckets}"
            ),
        )
        .into_response(),
        Err(MarketHistoryError::CheckpointNotReady(reason)) => ApiResponse::<()>::error(
            StatusCode::SERVICE_UNAVAILABLE,
            error_codes::SERVICE_UNAVAILABLE,
            format!("indexer checkpoint not ready: {reason}"),
        )
        .into_response(),
        Err(MarketHistoryError::Database(e)) => internal_error(e),
    }
}
