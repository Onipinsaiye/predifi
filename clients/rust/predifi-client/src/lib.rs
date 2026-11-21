//! Read-only asynchronous client for PrediFi's documented HTTP API.
//!
//! The client does not sign transactions, manage wallets, or store private
//! keys. Provide an API base URL (for example `http://localhost:3000`) and use
//! the typed query methods to read public market and portfolio data.

use reqwest::{header::RETRY_AFTER, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashSet, time::Duration};
use thiserror::Error;
use url::Url;

const API_PREFIX: &str = "api/v1/";

/// Client for the read-only PrediFi HTTP API.
#[derive(Clone, Debug)]
pub struct PredifiClient {
    http: reqwest::Client,
    base_url: Url,
}

/// Backend timestamp, represented as Unix seconds or an RFC 3339 string.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ApiTimestamp {
    UnixSeconds(i64),
    Rfc3339(String),
}

impl PredifiClient {
    /// Create a client for a backend base URL such as `http://localhost:3000`.
    pub fn new(base_url: impl AsRef<str>) -> Result<Self, ClientError> {
        let mut base_url =
            Url::parse(base_url.as_ref()).map_err(|error| ClientError::InvalidBaseUrl {
                message: error.to_string(),
            })?;
        if !matches!(base_url.scheme(), "http" | "https") {
            return Err(ClientError::InvalidBaseUrl {
                message: "URL scheme must be http or https".to_owned(),
            });
        }
        if base_url.query().is_some() || base_url.fragment().is_some() {
            return Err(ClientError::InvalidBaseUrl {
                message: "base URL cannot contain a query or fragment".to_owned(),
            });
        }

        let path = base_url.path().trim_end_matches('/');
        base_url.set_path(&format!("{path}/"));

        Ok(Self {
            http: reqwest::Client::new(),
            base_url,
        })
    }

    /// Fetch API version lifecycle information.
    pub async fn api_index(&self) -> Result<ApiIndex, ClientError> {
        self.get(self.api_url(""), &()).await
    }

    /// Fetch backend health and dependency status.
    pub async fn health(&self) -> Result<HealthResponse, ClientError> {
        self.get(self.api_url("health"), &()).await
    }

    /// Fetch the legacy root health endpoint.
    pub async fn root_health(&self) -> Result<HealthResponse, ClientError> {
        let url = self
            .base_url
            .join("health")
            .map_err(|error| ClientError::InvalidBaseUrl {
                message: error.to_string(),
            })?;
        self.get(url, &()).await
    }

    /// Fetch a paginated, optionally filtered list of markets.
    pub async fn list_pools(&self, query: &PoolQuery) -> Result<PoolList, ClientError> {
        self.get(self.api_url("pools"), query).await
    }

    /// Fetch a market and its outcome odds.
    pub async fn get_pool(&self, pool_id: i64) -> Result<PoolWithOdds, ClientError> {
        self.get(self.api_url(&format!("pools/{pool_id}")), &()).await
    }

    /// Fetch protocol-wide aggregate statistics.
    pub async fn stats(&self, query: &StatsQuery) -> Result<ProtocolStats, ClientError> {
        self.get(self.api_url("stats"), query).await
    }

    /// Fetch the protocol fee configuration.
    pub async fn fees(&self) -> Result<FeeInfo, ClientError> {
        self.get(self.api_url("fees"), &()).await
    }

    /// Fetch the latest cached asset prices.
    pub async fn prices(&self) -> Result<PricesResponse, ClientError> {
        self.get(self.api_url("prices"), &()).await
    }

    /// Fetch the protocol leaderboard.
    pub async fn leaderboard(
        &self,
        query: &LeaderboardQuery,
    ) -> Result<LeaderboardResponse, ClientError> {
        self.get(self.api_url("leaderboard"), query).await
    }

    /// Fetch one cursor-paginated page of predictions for a market.
    ///
    /// Pass the returned `next_cursor` back as `after` to fetch the next page.
    pub async fn market_predictions_page(
        &self,
        market_id: i64,
        after: Option<&str>,
        limit: Option<u32>,
    ) -> Result<MarketPredictionsPage, ClientError> {
        let mut url = self.api_url(&format!("markets/{market_id}/predictions"));
        url.query_pairs_mut()
            .append_pair("limit", &limit.unwrap_or(20).to_string());
        if let Some(cursor) = after {
            url.query_pairs_mut().append_pair("after", cursor);
        }
        self.get_url(url).await
    }

    /// Fetch every cursor-paginated prediction for a market.
    ///
    /// A repeated cursor is treated as an API pagination error rather than
    /// looping indefinitely.
    pub async fn market_predictions(
        &self,
        market_id: i64,
        page_size: Option<u32>,
    ) -> Result<Vec<MarketPrediction>, ClientError> {
        let mut predictions = Vec::new();
        let mut after: Option<String> = None;
        let mut seen_cursors = HashSet::new();

        loop {
            let page = self
                .market_predictions_page(market_id, after.as_deref(), page_size)
                .await?;
            predictions.extend(page.predictions);
            match page.next_cursor {
                Some(cursor) if seen_cursors.insert(cursor.clone()) => after = Some(cursor),
                Some(cursor) => return Err(ClientError::Pagination { cursor }),
                None => return Ok(predictions),
            }
        }
    }

    /// Fetch a user's paginated prediction history.
    pub async fn user_history(
        &self,
        address: &str,
        query: &PaginationQuery,
    ) -> Result<PredictionHistory, ClientError> {
        self.get_address("users", address, "history", query).await
    }

    /// Fetch a user's predictions enriched with current market state.
    pub async fn user_predictions(
        &self,
        address: &str,
        query: &PaginationQuery,
    ) -> Result<UserPredictions, ClientError> {
        self.get_address("users", address, "predictions", query)
            .await
    }

    /// Fetch a user's profile aggregates.
    pub async fn user_profile(&self, address: &str) -> Result<UserProfile, ClientError> {
        self.get_address("users", address, "profile", &()).await
    }

    /// Fetch a user's saved interest tags.
    pub async fn user_interests(&self, address: &str) -> Result<UserInterests, ClientError> {
        self.get_address("users", address, "interests", &()).await
    }

    /// Fetch referral earnings for an address.
    pub async fn referrals(&self, address: &str) -> Result<ReferralEarnings, ClientError> {
        self.get_address("referrals", address, "", &()).await
    }

    /// Fetch per-pool referral earnings from the user referral endpoint.
    pub async fn user_referrals(&self, address: &str) -> Result<ReferralEarnings, ClientError> {
        self.get_address("users", address, "referrals", &()).await
    }

    /// Fetch a creator's published statistics.
    pub async fn creator_stats(&self, address: &str) -> Result<CreatorStats, ClientError> {
        let mut url = self.api_url("creators/");
        url.path_segments_mut()
            .map_err(|_| ClientError::InvalidBaseUrl {
                message: "base URL cannot be used to construct a path".to_owned(),
            })?
            .pop_if_empty()
            .push(address)
            .push("stats");
        self.get_url(url).await
    }

    /// Fetch tags associated with a market.
    pub async fn pool_tags(&self, pool_id: i64) -> Result<Value, ClientError> {
        self.get(self.api_url(&format!("pools/{pool_id}/tags")), &())
            .await
    }

    /// Fetch the available market tags.
    pub async fn tags(&self) -> Result<Value, ClientError> {
        self.get(self.api_url("tags"), &()).await
    }

    /// Fetch a per-market leaderboard.
    pub async fn pool_leaderboard(
        &self,
        pool_id: i64,
        query: &LeaderboardQuery,
    ) -> Result<Value, ClientError> {
        self.get(
            self.api_url(&format!("pools/{pool_id}/leaderboard")),
            query,
        )
        .await
    }

    /// Fetch referral estimate data returned by the backend.
    pub async fn referral_estimate(&self, address: &str) -> Result<Value, ClientError> {
        self.get_address("referrals", address, "estimate", &()).await
    }

    /// Fetch notifications for an address.
    ///
    /// Notifications use a flexible payload schema in the published API, so
    /// the response data is returned as JSON while preserving the success
    /// envelope handling used by the typed methods.
    pub async fn notifications(
        &self,
        address: &str,
        query: &NotificationQuery,
    ) -> Result<Value, ClientError> {
        self.get_address("notifications", address, "", query).await
    }

    fn api_url(&self, path: &str) -> Url {
        self.base_url
            .join(&format!("{API_PREFIX}{path}"))
            .expect("validated base URL accepts relative API paths")
    }

    async fn get<T, Q>(&self, url: Url, query: &Q) -> Result<T, ClientError>
    where
        T: DeserializeOwned,
        Q: Serialize + ?Sized,
    {
        let response = self.http.get(url).query(query).send().await?;
        self.decode_response(response).await
    }

    async fn get_url<T>(&self, url: Url) -> Result<T, ClientError>
    where
        T: DeserializeOwned,
    {
        let response = self.http.get(url).send().await?;
        self.decode_response(response).await
    }

    async fn get_address<T, Q>(
        &self,
        collection: &str,
        address: &str,
        suffix: &str,
        query: &Q,
    ) -> Result<T, ClientError>
    where
        T: DeserializeOwned,
        Q: Serialize + ?Sized,
    {
        let mut url = self.api_url(&format!("{collection}/"));
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| ClientError::InvalidBaseUrl {
                    message: "base URL cannot be used to construct a path".to_owned(),
                })?;
            segments.pop_if_empty().push(address);
            if !suffix.is_empty() {
                segments.push(suffix);
            }
        }
        self.get(url, query).await
    }

    async fn decode_response<T>(&self, response: reqwest::Response) -> Result<T, ClientError>
    where
        T: DeserializeOwned,
    {
        let status = response.status();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs);
        let body = response.text().await?;

        if !status.is_success() {
            let error = parse_api_error(&body);
            return Err(match status {
                StatusCode::UNAUTHORIZED => ClientError::Unauthorized { error },
                StatusCode::NOT_FOUND => ClientError::NotFound { error },
                StatusCode::TOO_MANY_REQUESTS => {
                    ClientError::RateLimited { error, retry_after }
                }
                status if status.is_server_error() => ClientError::Server { status, error },
                status => ClientError::Api { status, error },
            });
        }

        let value: Value = serde_json::from_str(&body)?;
        if value.get("status").and_then(Value::as_str) == Some("error") {
            let error = parse_api_error(&body);
            return Err(ClientError::Api { status, error });
        }

        let payload = if value.get("status").and_then(Value::as_str) == Some("success") {
            value.get("data").cloned().unwrap_or(value)
        } else {
            value
        };
        Ok(serde_json::from_value(payload)?)
    }
}

fn parse_api_error(body: &str) -> ApiError {
    let value = match serde_json::from_str::<Value>(body) {
        Ok(value) => value,
        Err(_) => {
            return ApiError {
                code: None,
                message: if body.trim().is_empty() {
                    "HTTP request failed".to_owned()
                } else {
                    body.to_owned()
                },
                request_id: None,
                retryable: None,
                details: None,
            }
        }
    };
    let error = value.get("error").unwrap_or(&value);
    ApiError {
        code: error.get("code").and_then(Value::as_str).map(str::to_owned),
        message: error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error.as_str())
            .unwrap_or("HTTP request failed")
            .to_owned(),
        request_id: error
            .get("request_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        retryable: error.get("retryable").and_then(Value::as_bool),
        details: error.get("details").cloned(),
    }
}

/// Errors returned by the client or PrediFi API.
#[derive(Debug, Error)]
pub enum ClientError {
    /// The HTTP request could not be completed.
    #[error("HTTP transport error: {0}")]
    Transport(#[from] reqwest::Error),
    /// A successful response did not match the expected JSON type.
    #[error("invalid API response: {0}")]
    Decode(#[from] serde_json::Error),
    /// The configured URL is invalid or cannot be used by the client.
    #[error("invalid base URL: {message}")]
    InvalidBaseUrl { message: String },
    /// The API rejected the request because it was unauthenticated.
    #[error("unauthorized: {error}")]
    Unauthorized { error: ApiError },
    /// The requested resource does not exist.
    #[error("not found: {error}")]
    NotFound { error: ApiError },
    /// The caller exceeded an API rate limit.
    #[error("rate limited: {error}")]
    RateLimited {
        error: ApiError,
        retry_after: Option<Duration>,
    },
    /// The server returned a 5xx response.
    #[error("server error ({status}): {error}")]
    Server { status: StatusCode, error: ApiError },
    /// The API returned another non-success status.
    #[error("API error ({status}): {error}")]
    Api { status: StatusCode, error: ApiError },
    /// The server returned a cursor already seen in the same pagination run.
    #[error("market predictions pagination repeated cursor {cursor}")]
    Pagination { cursor: String },
}

/// Structured API error details.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ApiError {
    /// Stable machine-readable error code, when provided by the API.
    pub code: Option<String>,
    /// Safe API error message or raw response body for non-JSON errors.
    pub message: String,
    /// Server request identifier, when provided.
    pub request_id: Option<String>,
    /// Whether retrying unchanged may succeed.
    pub retryable: Option<bool>,
    /// Field-level validation details, when returned by the API.
    pub details: Option<Value>,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.code, &self.request_id) {
            (Some(code), Some(request_id)) => {
                write!(formatter, "{code}: {} (request {request_id})", self.message)
            }
            (Some(code), None) => write!(formatter, "{code}: {}", self.message),
            (None, _) => formatter.write_str(&self.message),
        }
    }
}

/// API version lifecycle document.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ApiIndex {
    pub name: String,
    pub version: String,
    pub status: String,
    #[serde(default)]
    pub versions: Vec<ApiVersion>,
    #[serde(default)]
    pub deprecated_routes: Vec<Value>,
    pub deprecation_policy: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ApiVersion {
    pub version: String,
    pub status: String,
    pub deprecated_at: Option<String>,
    pub sunset_at: Option<String>,
    pub successor: Option<String>,
}

/// Backend health and dependency status.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct HealthResponse {
    pub status: String,
    pub version: Option<String>,
    pub service: Option<String>,
    pub dependencies: Option<Value>,
    pub errors: Option<Value>,
}

/// Pool filters and offset pagination.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PoolQuery {
    pub sort_by: Option<String>,
    pub category: Option<String>,
    pub status: Option<String>,
    pub creator: Option<String>,
    pub tags: Option<String>,
    pub tag: Option<String>,
    pub end_time_min: Option<String>,
    pub end_time_max: Option<String>,
    pub from_time: Option<String>,
    pub to_time: Option<String>,
    pub outcome: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// One market summary.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Pool {
    pub pool_id: i64,
    pub name: String,
    pub category: String,
    pub total_stake: i64,
    pub end_time: ApiTimestamp,
    pub created_at: String,
    pub state: String,
    pub creator: String,
    pub token: Option<String>,
    pub result: Option<String>,
    pub tags: Option<Vec<String>>,
}

/// A page of market summaries.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PoolList {
    pub pools: Vec<Pool>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
    pub status: String,
    pub category: Option<String>,
    pub sort_by: String,
}

/// Stake and implied odds for one outcome.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OutcomeOdds {
    pub outcome: i32,
    pub stake: i64,
    pub odds: f64,
}

/// Full market details with outcome odds.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PoolWithOdds {
    #[serde(flatten)]
    pub pool: Pool,
    pub odds: Vec<OutcomeOdds>,
}

/// Optional protocol statistics filters.
#[derive(Debug, Clone, Default, Serialize)]
pub struct StatsQuery {
    pub category: Option<String>,
    pub status: Option<String>,
}

/// Protocol-wide aggregate statistics.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ProtocolStats {
    pub total_value_locked: i64,
    pub total_bets: i64,
    pub total_pools: i64,
}

/// Protocol fee configuration, in basis points.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct FeeInfo {
    pub treasury_fee_bps: u32,
    pub referral_fee_bps: u32,
}

/// Cached prices payload.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PricesResponse {
    pub prices: Value,
    pub timestamp: Option<String>,
}

/// Pagination and ranking options for leaderboards.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LeaderboardQuery {
    pub rank_by: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub period: Option<String>,
}

/// One leaderboard row; fields depend on the selected ranking.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LeaderboardEntry {
    pub user_address: String,
    pub rank: i64,
    pub total_volume: Option<i64>,
    pub prediction_count: Option<i64>,
    pub total_winnings: Option<i64>,
    pub winning_predictions: Option<i64>,
    pub total_predictions: Option<i64>,
    pub win_rate: Option<f64>,
}

/// Leaderboard response.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LeaderboardResponse {
    pub leaderboard: Vec<LeaderboardEntry>,
    pub rank_by: String,
    pub limit: i64,
    pub offset: i64,
    pub period: Option<String>,
}

/// Offset pagination for user history and predictions.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PaginationQuery {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// One historical prediction.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Prediction {
    pub pool_id: i64,
    pub pool_name: String,
    pub pool_result: Option<String>,
    pub outcome: i32,
    pub amount: i64,
    pub created_at: String,
}

/// A user's prediction history page.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PredictionHistory {
    pub address: String,
    pub predictions: Vec<Prediction>,
    pub limit: i64,
    pub offset: i64,
}

/// One prediction enriched with current market state.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct UserPrediction {
    pub prediction_id: i64,
    pub pool_id: i64,
    pub pool_name: String,
    pub pool_category: String,
    pub pool_state: String,
    pub pool_total_stake: i64,
    pub pool_result: Option<String>,
    pub user_outcome: i32,
    pub user_amount: i64,
    pub is_winning_outcome: Option<bool>,
}

/// A user's enhanced prediction page.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct UserPredictions {
    pub address: String,
    pub predictions: Vec<UserPrediction>,
    pub limit: i64,
    pub offset: i64,
    pub total_predictions: i64,
}

/// A prediction in a market's cursor-paginated result.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MarketPrediction {
    pub id: i64,
    pub pool_id: i64,
    pub user_address: String,
    pub outcome: i32,
    pub amount: i64,
    pub created_at: String,
}

/// Cursor-paginated market prediction page.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MarketPredictionsPage {
    pub market_id: i64,
    pub predictions: Vec<MarketPrediction>,
    pub limit: i64,
    pub next_cursor: Option<String>,
    pub total: Option<i64>,
}

/// User aggregate portfolio statistics.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct UserProfile {
    pub total_predictions: i64,
    pub wins: i64,
    pub losses: i64,
    pub pending: i64,
    pub win_rate: f64,
    pub total_staked: i64,
    pub total_earnings: i64,
    pub active_positions: i64,
}

/// A user's saved interest tags.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct UserInterests {
    pub address: String,
    pub interests: Vec<String>,
}

/// Per-pool referral earnings.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ReferralEarning {
    pub pool_id: i64,
    pub pool_name: String,
    pub total_earned: i64,
    pub referral_count: i64,
}

/// Referral earnings summary.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ReferralEarnings {
    pub referrer: String,
    pub total_earned: i64,
    pub pools: Vec<ReferralEarning>,
}

/// Aggregate creator statistics.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CreatorStats {
    pub creator: String,
    pub pools_created: i64,
    pub pools_reward_eligible: i64,
    pub total_volume: i64,
}

/// Notification list filters.
#[derive(Debug, Clone, Default, Serialize)]
pub struct NotificationQuery {
    pub unread_only: Option<bool>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::Query,
        http::StatusCode,
        response::IntoResponse,
        routing::get,
        Json, Router,
    };
    use std::net::SocketAddr;

    async fn prediction_page(
        Query(query): Query<std::collections::HashMap<String, String>>,
    ) -> impl IntoResponse {
        let after = query.get("after").map(String::as_str);
        let (id, next_cursor) = match after {
            None => (1, Some("opaque-cursor".to_owned())),
            Some("opaque-cursor") => (2, None),
            Some(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error":"bad cursor"})),
                )
            }
        };
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "success",
                "data": {
                    "market_id": 8,
                    "predictions": [{
                        "id": id,
                        "pool_id": 8,
                        "user_address": "GUSER",
                        "outcome": 1,
                        "amount": 100,
                        "created_at": "2026-01-01T00:00:00Z"
                    }],
                    "limit": 1,
                    "next_cursor": next_cursor
                }
            })),
        )
    }

    async fn rate_limited() -> impl IntoResponse {
        (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "7")],
            Json(serde_json::json!({
                "status": "error",
                "error": {
                    "version": 1,
                    "code": "RATE_LIMITED",
                    "message": "slow down",
                    "request_id": "request-1",
                    "retryable": true
                }
            })),
        )
    }

    async fn repeated_cursor_page() -> impl IntoResponse {
        Json(serde_json::json!({
            "status": "success",
            "data": {
                "market_id": 9,
                "predictions": [],
                "limit": 1,
                "next_cursor": "repeated-cursor"
            }
        }))
    }

    async fn pool_list() -> impl IntoResponse {
        Json(serde_json::json!({
            "pools": [{
                "pool_id": 1,
                "name": "Market",
                "category": "Crypto",
                "total_stake": 22,
                "end_time": "2026-01-02T03:04:05Z",
                "created_at": "2026-01-01T03:04:05Z",
                "state": "active",
                "creator": "GCREATOR",
                "result": null,
                "tags": ["btc"]
            }],
            "total": 1,
            "limit": 20,
            "offset": 0,
            "status": "active",
            "category": null,
            "sort_by": "new"
        }))
    }

    async fn start_server() -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let app = Router::new()
            .route("/api/v1/markets/8/predictions", get(prediction_page))
            .route(
                "/api/v1/markets/9/predictions",
                get(repeated_cursor_page),
            )
            .route("/api/v1/pools", get(pool_list))
            .route("/api/v1/fees", get(rate_limited));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local test backend");
        let address = listener.local_addr().expect("local address");
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test API");
        });
        (address, task)
    }

    #[tokio::test]
    async fn follows_opaque_market_cursors_against_local_backend() {
        let (address, server) = start_server().await;
        let client = PredifiClient::new(format!("http://{address}")).expect("client");

        let predictions = client
            .market_predictions(8, Some(1))
            .await
            .expect("all market predictions");

        assert_eq!(
            predictions
                .iter()
                .map(|prediction| prediction.id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        server.abort();
    }

    #[tokio::test]
    async fn deserializes_direct_pool_list_from_local_backend() {
        let (address, server) = start_server().await;
        let client = PredifiClient::new(format!("http://{address}")).expect("client");

        let page = client
            .list_pools(&PoolQuery::default())
            .await
            .expect("pool list");

        assert_eq!(page.pools[0].pool_id, 1);
        assert_eq!(
            page.pools[0].end_time,
            ApiTimestamp::Rfc3339("2026-01-02T03:04:05Z".to_owned())
        );
        assert_eq!(page.pools[0].token, None);
        server.abort();
    }

    #[tokio::test]
    async fn reports_rate_limit_error_and_retry_after() {
        let (address, server) = start_server().await;
        let client = PredifiClient::new(format!("http://{address}")).expect("client");

        let result = client.fees().await;

        assert!(matches!(
            result,
            Err(ClientError::RateLimited {
                retry_after: Some(delay),
                ..
            }) if delay == Duration::from_secs(7)
        ));
        server.abort();
    }

    #[tokio::test]
    async fn rejects_repeated_market_cursor() {
        let (address, server) = start_server().await;
        let client = PredifiClient::new(format!("http://{address}")).expect("client");

        let result = client.market_predictions(9, Some(1)).await;

        assert!(matches!(
            result,
            Err(ClientError::Pagination { cursor }) if cursor == "repeated-cursor"
        ));
        server.abort();
    }
}
