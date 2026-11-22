use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

use crate::{
    error::AppError,
    state::AppState,
};

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/portfolio/:address/exposure", get(get_exposure))
        .route(
            "/v1/portfolio/:address/claimable-rewards",
            get(get_claimable_rewards),
        )
        .route(
            "/v1/portfolio/:address/claimable-rewards/simulate",
            post(post_simulate_claims),
        )
        .route(
            "/v1/portfolio/:address/claims/:tx/reconciliation",
            get(get_claim_reconciliation),
        )
        .route("/v1/portfolio/:address/activity.csv", get(get_activity_csv))
}

// ---------------------------------------------------------------------------
// Exposure (#261)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ExposureQuery {
    /// Optional filter: only include open positions.
    #[serde(default)]
    pub open_only: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ExposureRow {
    pub market_id: String,
    pub market_title: Option<String>,
    pub outcome: String,
    /// Absolute amount held (decimal string, exact).
    pub amount: String,
    /// Share of the indexed portfolio, 0..=1 as decimal string.
    pub share: String,
    /// "estimate" or "claimable".
    pub kind: String,
    /// True when price data was missing for this market/outcome.
    pub price_missing: bool,
}

#[derive(Debug, Serialize)]
pub struct ExposureResponse {
    pub address: String,
    pub total_indexed_value: String,
    pub rows: Vec<ExposureRow>,
}

async fn get_exposure(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Query(query): Query<ExposureQuery>,
) -> Result<Json<ExposureResponse>, AppError> {
    let address = normalize_address(&address)?;
    let open_only = query.open_only.unwrap_or(true);

    let rows = state
        .db
        .portfolio_exposure_by_market_outcome(&address, open_only)
        .await
        .map_err(AppError::from_db)?;

    let total = state
        .db
        .portfolio_indexed_total(&address)
        .await
        .map_err(AppError::from_db)?;

    let mapped = rows
        .into_iter()
        .map(|r| ExposureRow {
            market_id: r.market_id,
            market_title: r.market_title,
            outcome: r.outcome,
            amount: r.amount,
            share: r.share,
            kind: r.kind,
            price_missing: r.price_missing,
        })
        .collect();

    Ok(Json(ExposureResponse {
        address,
        total_indexed_value: total,
        rows: mapped,
    }))
}

// ---------------------------------------------------------------------------
// Claimable rewards queue (#262)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ClaimableQuery {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ClaimableRewardRow {
    pub claim_id: String,
    pub market_id: String,
    pub market_title: Option<String>,
    pub outcome: String,
    pub estimated_amount: String,
    pub status: String,
    pub tx_link: Option<String>,
    pub simulation_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ClaimableRewardsResponse {
    pub address: String,
    pub rows: Vec<ClaimableRewardRow>,
    pub next_cursor: Option<String>,
}

const MAX_CLAIM_LIMIT: u32 = 100;
const DEFAULT_CLAIM_LIMIT: u32 = 25;

async fn get_claimable_rewards(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Query(query): Query<ClaimableQuery>,
) -> Result<Json<ClaimableRewardsResponse>, AppError> {
    let address = normalize_address(&address)?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_CLAIM_LIMIT)
        .clamp(1, MAX_CLAIM_LIMIT);

    let page = state
        .db
        .portfolio_claimable_rewards(&address, limit, query.cursor.as_deref())
        .await
        .map_err(AppError::from_db)?;

    let rows = page
        .rows
        .into_iter()
        .map(|r| ClaimableRewardRow {
            claim_id: r.claim_id,
            market_id: r.market_id,
            market_title: r.market_title,
            outcome: r.outcome,
            estimated_amount: r.estimated_amount,
            status: r.status,
            tx_link: r.tx_link,
            simulation_error: r.simulation_error,
        })
        .collect();

    Ok(Json(ClaimableRewardsResponse {
        address,
        rows,
        next_cursor: page.next_cursor,
    }))
}

#[derive(Debug, Deserialize)]
pub struct SimulateClaimsRequest {
    /// Bounded selection of claim ids to simulate.
    pub claim_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SimulateClaimsResponse {
    pub address: String,
    pub simulated: Vec<SimulatedClaim>,
    pub failures: Vec<SimulatedClaimFailure>,
}

#[derive(Debug, Serialize)]
pub struct SimulatedClaim {
    pub claim_id: String,
    pub market_id: String,
    pub outcome: String,
    pub estimated_amount: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct SimulatedClaimFailure {
    pub claim_id: String,
    pub error: String,
}

const MAX_SIMULATE_BATCH: usize = 50;

async fn post_simulate_claims(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Json(body): Json<SimulateClaimsRequest>,
) -> Result<Json<SimulateClaimsResponse>, AppError> {
    let address = normalize_address(&address)?;

    if body.claim_ids.is_empty() {
        return Err(AppError::bad_request("claim_ids must not be empty"));
    }
    if body.claim_ids.len() > MAX_SIMULATE_BATCH {
        return Err(AppError::bad_request(format!(
            "claim_ids exceeds maximum batch size of {}",
            MAX_SIMULATE_BATCH
        )));
    }

    let result = state
        .db
        .portfolio_simulate_claims(&address, &body.claim_ids)
        .await
        .map_err(AppError::from_db)?;

    let simulated = result
        .simulated
        .into_iter()
        .map(|r| SimulatedClaim {
            claim_id: r.claim_id,
            market_id: r.market_id,
            outcome: r.outcome,
            estimated_amount: r.estimated_amount,
            status: r.status,
        })
        .collect();

    let failures = result
        .failures
        .into_iter()
        .map(|f| SimulatedClaimFailure {
            claim_id: f.claim_id,
            error: f.error,
        })
        .collect();

    Ok(Json(SimulateClaimsResponse {
        address,
        simulated,
        failures,
    }))
}

// ---------------------------------------------------------------------------
// Claim reconciliation (#263)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ClaimReconciliationResponse {
    pub address: String,
    pub claim_tx: String,
    pub market_id: Option<String>,
    pub outcome: Option<String>,
    pub payout_amount: Option<String>,
    pub source_event_id: Option<String>,
    /// "reconciled", "pending", or "delayed".
    pub status: String,
    pub position_id: Option<String>,
    pub settlement_event_id: Option<String>,
}

async fn get_claim_reconciliation(
    State(state): State<Arc<AppState>>,
    Path((address, tx)): Path<(String, String)>,
) -> Result<Json<ClaimReconciliationResponse>, AppError> {
    let address = normalize_address(&address)?;
    let tx = normalize_tx(&tx)?;

    let record = state
        .db
        .portfolio_claim_reconciliation(&address, &tx)
        .await
        .map_err(AppError::from_db)?
        .ok_or_else(|| AppError::not_found("claim transaction not found"))?;

    Ok(Json(ClaimReconciliationResponse {
        address,
        claim_tx: record.claim_tx,
        market_id: record.market_id,
        outcome: record.outcome,
        payout_amount: record.payout_amount,
        source_event_id: record.source_event_id,
        status: record.status,
        position_id: record.position_id,
        settlement_event_id: record.settlement_event_id,
    }))
}

// ---------------------------------------------------------------------------
// Activity CSV export (#264)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ActivityCsvQuery {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
}

const MAX_CSV_LIMIT: u32 = 1000;
const DEFAULT_CSV_LIMIT: u32 = 500;
const CSV_SCHEMA_VERSION: &str = "1";

async fn get_activity_csv(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Query(query): Query<ActivityCsvQuery>,
) -> Result<Response, AppError> {
    let address = normalize_address(&address)?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_CSV_LIMIT)
        .clamp(1, MAX_CSV_LIMIT);

    let page = state
        .db
        .portfolio_activity_export(&address, limit, query.cursor.as_deref())
        .await
        .map_err(AppError::from_db)?;

    let network = state.config.network.clone();

    let mut out = String::new();
    // Schema header comment lines (documented schema version + network).
    out.push_str(&format!("# schema_version={}\n", CSV_SCHEMA_VERSION));
    out.push_str(&format!("# network={}\n", csv_escape(&network)));
    out.push_str(
        "schema_version,network,timestamp,event_type,market_id,outcome,asset,amount,decimals,tx_hash,position_id,source_event_id\n",
    );

    for row in page.rows {
        let fields = [
            CSV_SCHEMA_VERSION.to_string(),
            network.clone(),
            row.timestamp,
            row.event_type,
            row.market_id,
            row.outcome,
            row.asset,
            row.amount,
            row.decimals.to_string(),
            row.tx_hash,
            row.position_id,
            row.source_event_id,
        ];
        let line = fields
            .iter()
            .map(|f| csv_escape(f))
            .collect::<Vec<_>>()
            .join(",");
        out.push_str(&line);
        out.push('\n');
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"portfolio-activity.csv\""),
    );
    if let Some(cursor) = page.next_cursor {
        if let Ok(v) = HeaderValue::from_str(&cursor) {
            headers.insert("x-next-cursor", v);
        }
    }

    Ok((StatusCode::OK, headers, out).into_response())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn normalize_address(address: &str) -> Result<String, AppError> {
    let trimmed = address.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("address must not be empty"));
    }
    if trimmed.len() > 128 {
        return Err(AppError::bad_request("address is too long"));
    }
    Ok(trimmed.to_string())
}

fn normalize_tx(tx: &str) -> Result<String, AppError> {
    let trimmed = tx.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("transaction hash must not be empty"));
    }
    if trimmed.len() > 128 {
        return Err(AppError::bad_request("transaction hash is too long"));
    }
    Ok(trimmed.to_string())
}

/// Escape a CSV cell, preventing formula injection and quoting as needed.
fn csv_escape(value: &str) -> String {
    // Prevent formula injection: prefix dangerous leading characters.
    let needs_formula_guard = matches!(
        value.chars().next(),
        Some('=') | Some('+') | Some('-') | Some('@') | Some('\t') | Some('\r')
    );
    let guarded = if needs_formula_guard {
        format!("'{}", value)
    } else {
        value.to_string()
    };

    let needs_quotes = guarded
        .chars()
        .any(|c| c == ',' || c == '"' || c == '\n' || c == '\r');
    if needs_quotes {
        let escaped = guarded.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        guarded
    }
}

// Keep json import used for potential future structured errors.
#[allow(dead_code)]
fn _unused() -> serde_json::Value {
    json!({})
}