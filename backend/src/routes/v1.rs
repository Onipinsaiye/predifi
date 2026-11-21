use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{
    error::ApiError,
    state::AppState,
};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/portfolio/exposure", get(portfolio_exposure))
        .route("/portfolio/claimable", get(portfolio_claimable))
        .route("/portfolio/claims/:claim_id/reconcile", get(portfolio_claim_reconcile))
        .route("/portfolio/activity/export", get(portfolio_activity_export))
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

#[derive(Debug, Deserialize)]
pub struct ExposureQuery {
    pub wallet: Option<String>,
    pub network: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExposureOutcome {
    pub market: String,
    pub outcome: String,
    pub amount: String,
    pub share: String,
    pub is_estimate: bool,
}

#[derive(Debug, Serialize)]
pub struct ExposureResponse {
    pub wallet: String,
    pub network: String,
    pub total: String,
    pub outcomes: Vec<ExposureOutcome>,
}

async fn portfolio_exposure(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ExposureQuery>,
) -> Result<Json<ExposureResponse>, ApiError> {
    let wallet = query
        .wallet
        .ok_or_else(|| ApiError::bad_request("wallet query parameter is required"))?;
    let network = query.network.unwrap_or_else(|| "mainnet".to_string());

    let exposure = state
        .portfolio
        .exposure(&wallet, &network)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(exposure))
}

#[derive(Debug, Deserialize)]
pub struct ClaimableQuery {
    pub wallet: Option<String>,
    pub network: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ClaimableItem {
    pub claim_id: String,
    pub market: String,
    pub outcome: String,
    pub estimated_amount: String,
    pub status: String,
    pub transaction_link: Option<String>,
    pub simulation_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ClaimableResponse {
    pub wallet: String,
    pub network: String,
    pub items: Vec<ClaimableItem>,
}

async fn portfolio_claimable(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ClaimableQuery>,
) -> Result<Json<ClaimableResponse>, ApiError> {
    let wallet = query
        .wallet
        .ok_or_else(|| ApiError::bad_request("wallet query parameter is required"))?;
    let network = query.network.unwrap_or_else(|| "mainnet".to_string());
    let limit = query.limit.unwrap_or(50).min(200);

    let claimable = state
        .portfolio
        .claimable(&wallet, &network, limit)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(claimable))
}

#[derive(Debug, Serialize)]
pub struct ClaimReconcileResponse {
    pub claim_id: String,
    pub market: String,
    pub outcome: String,
    pub claim_transaction: Option<String>,
    pub payout_amount: Option<String>,
    pub source_event_id: Option<String>,
    pub status: String,
    pub reconciliation_status: String,
}

async fn portfolio_claim_reconcile(
    State(state): State<Arc<AppState>>,
    Path(claim_id): Path<String>,
) -> Result<Json<ClaimReconcileResponse>, ApiError> {
    let reconciliation = state
        .portfolio
        .reconcile_claim(&claim_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(reconciliation))
}

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    pub wallet: Option<String>,
    pub network: Option<String>,
    pub page: Option<u64>,
    pub page_size: Option<u64>,
}

async fn portfolio_activity_export(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, ApiError> {
    let wallet = query
        .wallet
        .ok_or_else(|| ApiError::bad_request("wallet query parameter is required"))?;
    let network = query.network.unwrap_or_else(|| "mainnet".to_string());
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(1000).clamp(1, 5000);

    let csv = state
        .portfolio
        .export_activity_csv(&wallet, &network, page, page_size)
        .await
        .map_err(ApiError::from)?;

    let headers = [
        (
            axum::http::header::CONTENT_TYPE,
            "text/csv; charset=utf-8".to_string(),
        ),
        (
            axum::http::header::CONTENT_DISPOSITION,
            format!(
                "attachment; filename=\"portfolio-activity-{}-{}-p{}.csv\"",
                network, wallet, page
            ),
        ),
    ];

    Ok((headers, csv).into_response())
}