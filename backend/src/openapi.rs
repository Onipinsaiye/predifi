//! OpenAPI documentation.
//!
//! The spec is served at `/api-docs/openapi.json` and committed as
//! `openapi.json` for frontend type generation. CI keeps it honest (#52):
//!
//! * `predifi-openapi --check-contract` fails when the committed artifact
//!   differs from a fresh, canonical regeneration or an operation lacks
//!   responses;
//! * `tests/openapi_drift.rs` fails when a route in `src/routes` is neither
//!   documented nor listed in [`UNDOCUMENTED_ROUTES`], when a documented
//!   operation is missing its error cases, or when a live response does not
//!   match the documented status code and schema.

use axum::{routing::get, Json, Router};
use utoipa::{OpenApi, ToSchema};

// ── Documented schemas ────────────────────────────────────────────────────────

/// OpenAPI schema for a prediction market pool.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct PoolDoc {
    pub pool_id: i64,
    pub name: String,
    pub category: String,
    pub total_stake: i64,
    pub end_time: i64,
    pub created_at: String,
    pub state: String,
    pub creator: String,
    pub token: String,
    pub result: Option<String>,
}

/// OpenAPI schema for outcome odds within a pool.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct OutcomeOddsDoc {
    pub outcome: i32,
    pub stake: i64,
    pub odds: f64,
}

/// OpenAPI schema for a pool with real-time odds attached.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct PoolWithOddsDoc {
    #[serde(flatten)]
    pub pool: PoolDoc,
    pub odds: Vec<OutcomeOddsDoc>,
}

/// OpenAPI schema for the paginated pool list response.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct PoolListResponse {
    pub pools: Vec<PoolDoc>,
    /// Number of pools matching the filters (across all pages).
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
    pub status: String,
    pub category: Option<String>,
    pub sort_by: String,
}

/// OpenAPI schema for a single prediction in the market predictions list.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct MarketPredictionDoc {
    /// Stable row ID; also used as the pagination cursor value.
    pub id: i64,
    pub pool_id: i64,
    pub user_address: String,
    pub outcome: i32,
    pub amount: i64,
    pub created_at: String,
}

/// OpenAPI schema for the cursor-paginated market predictions response.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct MarketPredictionsResponse {
    pub market_id: i64,
    pub predictions: Vec<MarketPredictionDoc>,
    pub limit: i64,
    /// Opaque cursor: pass as `?after=<value>` to retrieve the next page.
    /// `null` when the current page is the last.
    pub next_cursor: Option<String>,
}

/// OpenAPI schema for a single prediction record.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct PredictionDoc {
    pub pool_id: i64,
    pub pool_name: String,
    pub pool_result: Option<String>,
    pub outcome: i32,
    pub amount: i64,
    pub created_at: String,
}

/// OpenAPI schema for the paginated prediction history response.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct PredictionHistoryResponse {
    pub address: String,
    pub predictions: Vec<PredictionDoc>,
    pub limit: i64,
    pub offset: i64,
}

/// OpenAPI schema for an enhanced user prediction with current pool status.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct UserPredictionDoc {
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

/// OpenAPI schema for the enhanced user predictions response.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct UserPredictionsResponse {
    pub address: String,
    pub predictions: Vec<UserPredictionDoc>,
    pub limit: i64,
    pub offset: i64,
    pub total_predictions: usize,
}

/// OpenAPI schema for protocol-wide aggregate statistics.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct ProtocolStatsDoc {
    /// Total value locked across all pools (stroops).
    pub total_value_locked: i64,
    pub total_bets: i64,
    pub total_pools: i64,
}

/// OpenAPI schema for the fee configuration response.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct FeeInfoDoc {
    /// Protocol (treasury) fee in basis points.
    pub treasury_fee_bps: u32,
    /// Referral share of the protocol fee in basis points.
    pub referral_fee_bps: u32,
}

/// OpenAPI schema for a leaderboard entry ranked by betting volume.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct LeaderboardEntryVolume {
    pub user_address: String,
    pub total_volume: i64,
    pub prediction_count: i64,
    pub rank: i64,
}

/// OpenAPI schema for a leaderboard entry ranked by winnings.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct LeaderboardEntryWinnings {
    pub user_address: String,
    pub total_winnings: i64,
    pub winning_predictions: i64,
    pub total_predictions: i64,
    pub win_rate: f64,
    pub rank: i64,
}

/// OpenAPI schema for the leaderboard response.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct LeaderboardResponse {
    pub leaderboard: Vec<LeaderboardEntryVolume>,
    pub rank_by: String,
    pub limit: i64,
    pub offset: i64,
}

/// OpenAPI schema for a single referral earning row (per pool).
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct ReferralEarningDoc {
    pub pool_id: i64,
    pub pool_name: String,
    pub total_earned: i64,
    pub referral_count: i64,
}

/// OpenAPI schema for the referral earnings response.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct ReferralEarningsResponse {
    pub referrer: String,
    pub total_earned: i64,
    pub pools: Vec<ReferralEarningDoc>,
}

/// OpenAPI schema for the dependency status block inside a health response.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct DependencyStatus {
    pub db: String,
    pub rpc: String,
    pub redis: String,
    pub price_cache: String,
}

/// OpenAPI schema for the per-dependency error details block inside a health response.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct HealthErrors {
    pub db: Option<String>,
    pub rpc: Option<String>,
    pub redis: Option<String>,
    pub price_cache: Option<String>,
}

/// OpenAPI schema for the full health check response body.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
    pub version: String,
    pub dependencies: DependencyStatus,
    pub errors: HealthErrors,
}

// ── Profile (src/profile.rs) ───────────────────────────────────────────────────

/// OpenAPI schema for aggregate prediction statistics on a user's profile.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct ProfileStatsDoc {
    pub total_predictions: i64,
    pub wins: i64,
    pub losses: i64,
    pub pending: i64,
    pub win_rate: f64,
    pub total_staked: i64,
    pub total_earnings: i64,
    pub active_positions: i64,
}

/// OpenAPI schema for one prediction's claim status on a user's profile.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct ClaimStatusDoc {
    pub prediction_id: i64,
    pub pool_id: i64,
    pub pool_name: String,
    pub outcome: i32,
    pub amount: i64,
    pub pool_state: String,
    pub pool_result: Option<String>,
    pub is_winner: Option<bool>,
    pub claimed: bool,
    pub claimed_amount: i64,
    pub claim_window_expires_at: Option<String>,
    pub claim_expired: bool,
}

/// OpenAPI schema for one day of staking/earnings activity.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct PerformancePointDoc {
    /// Start of the UTC day this point represents.
    pub day: String,
    pub staked: i64,
    pub earnings: i64,
    pub predictions: i64,
}

/// OpenAPI schema for the full user profile payload.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct UserProfileDoc {
    pub address: String,
    pub stats: ProfileStatsDoc,
    pub claims: Vec<ClaimStatusDoc>,
    pub performance: Vec<PerformancePointDoc>,
}

// ── Portfolio: wallet activity ledger (#309) ────────────────────────────────────

/// OpenAPI schema for one typed wallet-activity event. Forward-compatible:
/// an `event_type` this build doesn't recognize decodes as `unknown` rather
/// than being dropped or erroring.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActivityEventDoc {
    PoolCreated {
        pool_id: i64,
        creator: String,
    },
    PredictionPlaced {
        pool_id: i64,
        outcome: i32,
        amount: i64,
    },
    PoolResolved {
        pool_id: i64,
        winning_outcome: i32,
    },
    Claim {
        pool_id: i64,
        amount_paid: i64,
    },
    Unknown {
        raw_type: String,
    },
}

/// OpenAPI schema for one wallet-activity item.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct WalletActivityItemDoc {
    pub network: String,
    pub contract_id: String,
    pub ledger: i64,
    pub tx_hash: String,
    pub event_index: i32,
    pub created_at: String,
    pub event: ActivityEventDoc,
    /// Opaque cursor for this item; pass as `?after=` to fetch the next page.
    pub cursor: String,
}

/// OpenAPI schema for the wallet-activity page's indexing-status block.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct IndexingStatusDoc {
    /// `true` only when the listener checkpoint is a valid, known ledger.
    pub complete: bool,
    pub checkpoint_ledger: Option<i64>,
    pub reason: Option<String>,
}

/// OpenAPI schema for a cursor-paginated page of wallet activity.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct WalletActivityPageDoc {
    pub items: Vec<WalletActivityItemDoc>,
    pub next_cursor: Option<String>,
    pub indexing_status: IndexingStatusDoc,
}

// ── Portfolio: P&L (#310) ───────────────────────────────────────────────────────

/// OpenAPI schema for why a position's P&L could not be computed.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MissingInputReasonDoc {
    PoolUnresolved,
    ZeroWinningStake,
    AmountOverflow,
    NegativeInput,
}

/// OpenAPI schema for one position's computed (or not-yet-computable) P&L.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct PositionPnlDoc {
    pub pnl_model_version: u32,
    pub stake: i64,
    /// `true` when `net_return` is a realized on-chain claim rather than an
    /// estimate derived from current pool state.
    pub realized: bool,
    pub gross_payout_before_fee: Option<i64>,
    pub fees: Option<i64>,
    pub net_return: Option<i64>,
    pub net_pnl: Option<i64>,
    pub missing_reason: Option<MissingInputReasonDoc>,
}

/// OpenAPI schema for one `(pool_id, pnl)` entry in a wallet's P&L response.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct WalletPositionDoc {
    pub pool_id: i64,
    pub pnl: PositionPnlDoc,
}

/// OpenAPI schema for the aggregate P&L across a wallet's whole portfolio.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct PortfolioPnlDoc {
    pub pnl_model_version: u32,
    /// Exact integer, serialized as a number (sums of `i64` stakes fit
    /// comfortably in this range in practice).
    #[schema(value_type = i64)]
    pub total_stake: i128,
    #[schema(value_type = i64)]
    pub total_net_return: i128,
    #[schema(value_type = i64)]
    pub total_net_pnl: i128,
    pub positions_computed: i64,
    pub positions_missing: i64,
    pub missing_reasons: Vec<MissingInputReasonDoc>,
}

/// OpenAPI schema for `GET /api/v1/wallets/{address}/pnl`.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct WalletPnlDoc {
    pub address: String,
    pub positions: Vec<WalletPositionDoc>,
    pub portfolio: PortfolioPnlDoc,
}

// ── Portfolio: snapshots (#311) ─────────────────────────────────────────────────

/// OpenAPI schema for a checkpoint-gated portfolio snapshot.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct PortfolioSnapshotDoc {
    pub id: i64,
    pub user_address: String,
    /// Ledger the snapshot was computed against — its source cursor.
    pub checkpoint_ledger: i64,
    pub pnl_model_version: i32,
    pub total_staked: i64,
    pub total_realized_return: i64,
    pub net_realized_pnl: i64,
    pub open_positions: i64,
    pub computed_at: String,
}

// ── Portfolio: exports (#312) ───────────────────────────────────────────────────

/// OpenAPI schema for `POST /api/v1/wallets/{address}/exports`.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct CreateExportRequestDoc {
    /// Max rows to export (default 10000, max 50000).
    pub row_limit: Option<i64>,
    /// Max wall-clock seconds the export worker may spend (default 60, max 300).
    pub time_limit_secs: Option<i64>,
}

/// Lifecycle status of a portfolio export.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExportStatusKindDoc {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    /// Derived, not stored: `succeeded` plus `expires_at` in the past.
    Expired,
}

/// OpenAPI schema for an export's status/progress.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct ExportStatusViewDoc {
    pub id: i64,
    pub status: ExportStatusKindDoc,
    pub rows_written: i64,
    pub row_limit: i64,
    pub time_limit_secs: i64,
    pub checkpoint_ledger: Option<i64>,
    pub error: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
    /// `true` only when the artifact exists and has not expired.
    pub download_available: bool,
}

// ── Market history (#313, #314) ─────────────────────────────────────────────────

/// OpenAPI schema for one bucketed historical market-value sample.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct MarketValueSampleDoc {
    /// Start of this bucket's interval, UTC, RFC 3339.
    pub bucket_start: String,
    /// Cumulative value as of the end of this interval, as an exact decimal
    /// string (never a float).
    pub value: String,
    /// Committed listener checkpoint this sample was derived from.
    pub checkpoint_ledger: i64,
}

/// OpenAPI schema for `GET /api/v1/markets/{pool_id}/history`.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct MarketHistoryPageDoc {
    pub pool_id: i64,
    /// `1m` | `5m` | `1h` | `1d`.
    pub bucket_width: String,
    pub samples: Vec<MarketValueSampleDoc>,
}

// ── Envelopes ─────────────────────────────────────────────────────────────────

/// One rejected field inside a validation error.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct FieldViolationDoc {
    /// Field name, query parameter or JSON pointer.
    pub field: String,
    /// Why the value was rejected.
    pub reason: String,
}

/// Error detail inside the standard error envelope (version 1).
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct ErrorDetailDoc {
    /// Envelope version; currently `1`.
    pub version: u32,
    /// Stable machine-readable error code, e.g. `DATABASE_UNAVAILABLE`.
    pub code: String,
    /// Safe human-readable message. Never contains internal error details.
    pub message: String,
    /// Request id; also returned in the `x-request-id` response header.
    pub request_id: String,
    /// Whether the same request may succeed if retried later.
    pub retryable: bool,
    /// Field-level validation failures; present only for validation errors.
    #[serde(default)]
    pub details: Vec<FieldViolationDoc>,
}

/// Standard error envelope returned by every failing request.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct ErrorResponse {
    /// Always `"error"`.
    pub status: String,
    pub error: ErrorDetailDoc,
}

/// Body of `429` responses (`RATE_LIMIT_EXCEEDED`), and of `503` responses
/// when the distributed rate limiter itself is unavailable
/// (`RATE_LIMITER_UNAVAILABLE`). Same shape as [`ErrorResponse`].
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct RateLimitErrorResponse {
    /// Always `"error"`.
    pub status: String,
    pub error: ErrorDetailDoc,
}

/// `503` body: a handler error envelope, or the rate limiter's own body when
/// Redis is unavailable for a fail-closed tier.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
#[serde(untagged)]
pub enum ServiceUnavailableResponse {
    Envelope(ErrorResponse),
    RateLimiter(RateLimitErrorResponse),
}

/// Success envelope with an endpoint-specific `data` object.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct DataEnvelope {
    /// Always `"success"`.
    pub status: String,
    #[schema(value_type = Object)]
    pub data: serde_json::Value,
}

macro_rules! success_envelope {
    ($(#[$doc:meta])* $name:ident, $data:ty) => {
        $(#[$doc])*
        #[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
        pub struct $name {
            /// Always `"success"`.
            pub status: String,
            pub data: $data,
        }
    };
}

success_envelope!(
    /// Success envelope for `GET /api/v1/pools/{pool_id}`.
    PoolWithOddsEnvelope,
    PoolWithOddsDoc
);
success_envelope!(
    /// Success envelope for `GET /api/v1/stats`.
    ProtocolStatsEnvelope,
    ProtocolStatsDoc
);
success_envelope!(
    /// Success envelope for `GET /api/v1/leaderboard`.
    LeaderboardEnvelope,
    LeaderboardResponse
);
success_envelope!(
    /// Success envelope for `GET /api/v1/users/{address}/history`.
    PredictionHistoryEnvelope,
    PredictionHistoryResponse
);
success_envelope!(
    /// Success envelope for `GET /api/v1/users/{address}/predictions`.
    UserPredictionsEnvelope,
    UserPredictionsResponse
);
success_envelope!(
    /// Success envelope for `GET /api/v1/markets/{market_id}/predictions`.
    MarketPredictionsEnvelope,
    MarketPredictionsResponse
);
success_envelope!(
    /// Success envelope for indexer ingestion endpoints.
    IndexerOkEnvelope,
    IndexerOkResponse
);
success_envelope!(
    /// Success envelope for `GET /api/v1/prices`.
    PricesEnvelope,
    Vec<AssetPriceDoc>
);
success_envelope!(
    /// Success envelope for `GET /api/v1/users/{address}/profile`.
    UserProfileEnvelope,
    UserProfileDoc
);
success_envelope!(
    /// Success envelope for `GET /api/v1/wallets/{address}/activity`.
    WalletActivityEnvelope,
    WalletActivityPageDoc
);
success_envelope!(
    /// Success envelope for `GET /api/v1/wallets/{address}/pnl`.
    WalletPnlEnvelope,
    WalletPnlDoc
);
success_envelope!(
    /// Success envelope for `GET /api/v1/wallets/{address}/snapshot`.
    PortfolioSnapshotEnvelope,
    PortfolioSnapshotDoc
);
success_envelope!(
    /// Success envelope for the portfolio export endpoints.
    ExportStatusEnvelope,
    ExportStatusViewDoc
);
success_envelope!(
    /// Success envelope for `GET /api/v1/markets/{pool_id}/history`.
    MarketHistoryEnvelope,
    MarketHistoryPageDoc
);

// ── Request / response bodies ─────────────────────────────────────────────────

/// OpenAPI schema for the `pool_created` event ingestion request body.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct PoolCreatedPayloadDoc {
    pub pool_id: u64,
    pub creator: String,
    pub end_time: u64,
    pub token: String,
    pub category: String,
    pub description: String,
}

/// OpenAPI schema for the `prediction_placed` event ingestion request body.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct PredictionPlacedPayloadDoc {
    pub pool_id: u64,
    pub user_address: String,
    pub outcome: i32,
    pub amount: i64,
}

/// OpenAPI schema for the claim event ingestion request body.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct ClaimPayloadDoc {
    pub pool_id: u64,
    pub user_address: String,
    /// Amount paid out by `claim_winnings` / `claim_refund` (0 for losers).
    pub amount_paid: i64,
}

/// OpenAPI schema for a successful indexer ingestion response.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct IndexerOkResponse {
    pub status: String,
    pub pool_id: u64,
}

/// OpenAPI schema for one cached asset price.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct AssetPriceDoc {
    pub symbol: String,
    pub price_usd: f64,
}

/// Lifecycle status of one API version.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct ApiVersionDoc {
    pub version: String,
    /// `active`, `deprecated` or `sunset`.
    pub status: String,
    /// RFC 3339 time deprecation was announced.
    pub deprecated_at: Option<String>,
    /// RFC 3339 time after which the version answers `410 Gone`.
    pub sunset_at: Option<String>,
    /// Version clients should migrate to.
    pub successor: Option<String>,
}

/// A deprecated operation inside a supported API version.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct DeprecatedRouteDoc {
    pub method: String,
    pub path: String,
    /// `deprecated` or `sunset`.
    pub status: String,
    pub deprecated_at: Option<String>,
    pub sunset_at: Option<String>,
    /// Path of the replacement operation.
    pub replacement: Option<String>,
}

/// OpenAPI schema for the v1 API index document.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct ApiIndexDoc {
    pub name: String,
    pub version: String,
    /// Lifecycle status of this version: `active`, `deprecated` or `sunset`.
    pub status: String,
    /// Every public API version and its lifecycle dates.
    pub versions: Vec<ApiVersionDoc>,
    /// Deprecated operations and their sunset dates.
    pub deprecated_routes: Vec<DeprecatedRouteDoc>,
    /// URL of the API versioning and deprecation policy.
    pub deprecation_policy: String,
}

/// OpenAPI schema for `POST /api/v1/auth/logout`.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct LogoutRequestDoc {
    /// Refresh token of the same session to revoke as well.
    pub refresh_token: Option<String>,
    /// Revoke every token issued to the user so far.
    #[serde(default)]
    pub all_sessions: bool,
}

/// OpenAPI schema for `POST /api/v1/auth/refresh`.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct RefreshTokenRequestDoc {
    pub refresh_token: String,
}

/// OpenAPI schema for `PATCH /api/v1/pools/{pool_id}/tags`.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct UpdatePoolTagsRequestDoc {
    /// Must match the pool's on-chain creator.
    pub creator: String,
    pub tags: Vec<String>,
}

/// OpenAPI schema for `POST /api/v1/notifications/{address}/read`.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct MarkNotificationsReadRequestDoc {
    /// Notification ids to mark read; omit to mark all as read.
    #[serde(default)]
    pub ids: Vec<i64>,
}

/// OpenAPI schema for `PUT /api/v1/users/{address}/interests`.
#[derive(Debug, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct SetInterestsRequestDoc {
    pub interests: Vec<String>,
}

// ── OpenAPI spec ──────────────────────────────────────────────────────────────

#[derive(OpenApi)]
#[openapi(
    info(
        title = "PrediFi API",
        version = "1.0.0",
        description = "REST API for the PrediFi prediction-market platform built on Stellar/Soroban.\n\nVersioning: `/api/v1` is active. Every `/api/<version>` response carries an `API-Version` header; deprecated versions and operations add `Deprecation`, `Sunset` and `Link` headers and are listed by `GET /api/v1`. See docs/API_VERSIONING_POLICY.md.\n\nErrors: every failure uses the versioned error envelope (`ErrorResponse`) with a stable `code`, a safe `message`, `request_id` (also in the `x-request-id` header) and `retryable`.",
        contact(name = "PrediFi Team", url = "https://github.com/Web3Novalabs/predifi"),
        license(name = "MIT")
    ),
    paths(
        api_health,
        api_v1_index,
        api_v1_health,
        api_get_pools,
        api_get_pool_by_id,
        api_get_pool_leaderboard,
        api_update_pool_tags,
        api_get_stats,
        api_get_fees,
        api_get_prices,
        api_get_leaderboard,
        api_list_tags,
        api_get_user_history,
        api_get_user_predictions,
        api_get_user_profile,
        api_get_user_interests,
        api_set_user_interests,
        api_list_notifications,
        api_mark_notifications_read,
        api_get_market_predictions,
        api_get_referrals,
        api_get_referral_estimate,
        api_get_user_referral_earnings,
        api_get_creator_stats,
        api_refresh_tokens,
        api_logout,
        api_ingest_pool_created,
        api_ingest_prediction_placed,
        api_ingest_claim,
        api_get_wallet_activity,
        api_get_wallet_pnl,
        api_get_wallet_snapshot,
        api_create_wallet_export,
        api_get_export_status,
        api_download_export,
        api_cancel_export,
        api_get_market_history,
    ),
    components(schemas(
        PoolDoc,
        OutcomeOddsDoc,
        PoolWithOddsDoc,
        PoolListResponse,
        MarketPredictionDoc,
        MarketPredictionsResponse,
        PredictionDoc,
        PredictionHistoryResponse,
        UserPredictionDoc,
        UserPredictionsResponse,
        ProtocolStatsDoc,
        FeeInfoDoc,
        LeaderboardEntryVolume,
        LeaderboardEntryWinnings,
        LeaderboardResponse,
        ReferralEarningDoc,
        ReferralEarningsResponse,
        DependencyStatus,
        HealthErrors,
        HealthResponse,
        FieldViolationDoc,
        ErrorDetailDoc,
        ErrorResponse,
        RateLimitErrorResponse,
        ServiceUnavailableResponse,
        DataEnvelope,
        PoolWithOddsEnvelope,
        ProtocolStatsEnvelope,
        LeaderboardEnvelope,
        PredictionHistoryEnvelope,
        UserPredictionsEnvelope,
        MarketPredictionsEnvelope,
        IndexerOkEnvelope,
        PricesEnvelope,
        AssetPriceDoc,
        ApiVersionDoc,
        DeprecatedRouteDoc,
        ApiIndexDoc,
        PoolCreatedPayloadDoc,
        PredictionPlacedPayloadDoc,
        ClaimPayloadDoc,
        IndexerOkResponse,
        RefreshTokenRequestDoc,
        LogoutRequestDoc,
        UpdatePoolTagsRequestDoc,
        MarkNotificationsReadRequestDoc,
        SetInterestsRequestDoc,
        UserProfileDoc,
        ProfileStatsDoc,
        ClaimStatusDoc,
        PerformancePointDoc,
        UserProfileEnvelope,
        ActivityEventDoc,
        WalletActivityItemDoc,
        IndexingStatusDoc,
        WalletActivityPageDoc,
        WalletActivityEnvelope,
        MissingInputReasonDoc,
        PositionPnlDoc,
        WalletPositionDoc,
        PortfolioPnlDoc,
        WalletPnlDoc,
        WalletPnlEnvelope,
        PortfolioSnapshotDoc,
        PortfolioSnapshotEnvelope,
        CreateExportRequestDoc,
        ExportStatusKindDoc,
        ExportStatusViewDoc,
        ExportStatusEnvelope,
        MarketValueSampleDoc,
        MarketHistoryPageDoc,
        MarketHistoryEnvelope,
    )),
    tags(
        (name = "health", description = "Service liveness and dependency checks"),
        (name = "pools", description = "Prediction market pools"),
        (name = "markets", description = "Historical market-value time series"),
        (name = "stats", description = "Protocol-wide statistics"),
        (name = "leaderboard", description = "User rankings"),
        (name = "predictions", description = "User prediction history"),
        (name = "users", description = "User profiles, interests and notifications"),
        (name = "portfolio", description = "Wallet activity ledger, P&L, snapshots and exports"),
        (name = "referrals", description = "Referral earnings"),
        (name = "auth", description = "Token rotation"),
        (name = "indexer", description = "Internal event ingestion endpoints"),
    )
)]
pub struct ApiDoc;

/// Routes under `/api` that are deliberately absent from the spec, with the
/// reason. The drift test fails for any other undocumented route.
///
/// Entries are `(METHOD, path)` with Axum-style `:param` segments.
pub const UNDOCUMENTED_ROUTES: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/api/healthz",
        "orchestrator liveness probe, not a client API",
    ),
    (
        "GET",
        "/api/readyz",
        "orchestrator readiness probe, not a client API",
    ),
    (
        "GET",
        "/api/v1/ws",
        "WebSocket upgrade; not describable in OpenAPI 3.1",
    ),
    (
        "PATCH",
        "/api/v1/admin/log-level",
        "operator endpoint guarded by an admin key; not for generated clients",
    ),
    (
        "POST",
        "/api/v1/pools/:id/pay-creator-incentive",
        "legacy un-enveloped responses (200 with {error}); document after migrating to ApiResponse",
    ),
    (
        "GET",
        "/api/v1/pool-templates",
        "legacy un-enveloped responses (200 with {error}); document after migrating to ApiResponse",
    ),
    (
        "POST",
        "/api/v1/pool-templates",
        "legacy un-enveloped responses (200 with {error}); document after migrating to ApiResponse",
    ),
];

// ── Path stubs ────────────────────────────────────────────────────────────────
//
// Response conventions (enforced by tests/openapi_drift.rs):
// * every failure, including path/query/JSON extraction failures, unknown
//   routes and body limits, uses the `ErrorResponse` envelope (#51);
// * rate-limited routes may answer `429`, and `503` from either a handler or
//   the rate limiter (`ServiceUnavailableResponse`).

#[allow(dead_code)]
#[utoipa::path(get, path = "/health", tag = "health",
    responses(
        (status = 200, description = "All dependencies healthy", body = HealthResponse),
        (status = 503, description = "One or more dependencies unreachable", body = HealthResponse),
    )
)]
async fn api_health() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1", tag = "health",
    responses(
        (status = 200, description = "API name, version and version lifecycle status", body = ApiIndexDoc),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
    )
)]
async fn api_v1_index() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/health", tag = "health",
    responses(
        (status = 200, description = "All dependencies healthy", body = HealthResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 503, description = "One or more dependencies unreachable", body = HealthResponse),
    )
)]
async fn api_v1_health() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/pools", tag = "pools",
    params(
        ("sort_by" = Option<String>, Query, description = "popular | ending_soon | new (default)"),
        ("category" = Option<String>, Query, description = "Category filter e.g. Sports, Crypto"),
        ("status" = Option<String>, Query, description = "active | closed | settled (default: active)"),
        ("creator" = Option<String>, Query, description = "Creator Stellar address filter (G...)"),
        ("tags" = Option<String>, Query, description = "Comma-separated tag filter (e.g. 'crypto,btc')"),
        ("end_time_min" = Option<String>, Query, description = "Filter pools ending at or after timestamp (RFC3339 or Unix seconds)"),
        ("end_time_max" = Option<String>, Query, description = "Filter pools ending at or before timestamp (RFC3339 or Unix seconds)"),
        ("outcome" = Option<String>, Query, description = "Settled outcome filter (only valid when status is settled)"),
        ("limit" = Option<i64>, Query, description = "Max results (default 20, max 100)"),
        ("offset" = Option<i64>, Query, description = "Pagination offset (default 0, max 10000)"),
    ),
    responses(
        (status = 200, description = "Paginated pool list", body = PoolListResponse),
        (status = 400, description = "Invalid query parameter", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_pools() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/pools/{pool_id}", tag = "pools",
    params(("pool_id" = i64, Path, description = "On-chain pool identifier")),
    responses(
        (status = 200, description = "Pool details with live odds", body = PoolWithOddsEnvelope),
        (status = 400, description = "Malformed pool id", body = ErrorResponse),
        (status = 404, description = "Pool not found", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available, or the coalesced lookup timed out (retry after `Retry-After`)", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_pool_by_id() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/pools/{pool_id}/leaderboard", tag = "leaderboard",
    params(
        ("pool_id" = i64, Path, description = "On-chain pool identifier"),
        ("rank_by" = Option<String>, Query, description = "volume (default) | win_rate | streak"),
        ("limit" = Option<i64>, Query, description = "Max results (default 20, max 100)"),
        ("offset" = Option<i64>, Query, description = "Pagination offset (default 0)"),
    ),
    responses(
        (status = 200, description = "Per-pool leaderboard", body = DataEnvelope),
        (status = 400, description = "Malformed path or query parameter", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_pool_leaderboard() {}

#[allow(dead_code)]
#[utoipa::path(patch, path = "/api/v1/pools/{pool_id}/tags", tag = "pools",
    params(("pool_id" = i64, Path, description = "On-chain pool identifier")),
    request_body = UpdatePoolTagsRequestDoc,
    responses(
        (status = 200, description = "Tags replaced", body = DataEnvelope),
        (status = 400, description = "Malformed path or body", body = ErrorResponse),
        (status = 403, description = "Caller is not the pool creator", body = ErrorResponse),
        (status = 404, description = "Pool not found", body = ErrorResponse),
        (status = 413, description = "Request body too large", body = ErrorResponse),
        (status = 415, description = "Missing JSON content type", body = ErrorResponse),
        (status = 422, description = "Body does not match the schema", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database or rate limiter not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_update_pool_tags() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/stats", tag = "stats",
    params(
        ("category" = Option<String>, Query, description = "Category filter"),
        ("status" = Option<String>, Query, description = "active | closed | settled"),
    ),
    responses(
        (status = 200, description = "Protocol-wide aggregate statistics", body = ProtocolStatsEnvelope),
        (status = 400, description = "Invalid query parameter", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_stats() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/fees", tag = "stats",
    responses(
        (status = 200, description = "Current protocol fee configuration", body = FeeInfoDoc),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
    )
)]
async fn api_get_fees() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/prices", tag = "stats",
    responses(
        (status = 200, description = "Latest cached asset prices", body = PricesEnvelope),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 503, description = "Price cache not ready", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_prices() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/leaderboard", tag = "leaderboard",
    params(
        ("rank_by" = Option<String>, Query, description = "volume (default) | winnings"),
        ("limit" = Option<i64>, Query, description = "Max results (default 20, max 100)"),
        ("offset" = Option<i64>, Query, description = "Pagination offset (default 0)"),
    ),
    responses(
        (status = 200, description = "User leaderboard", body = LeaderboardEnvelope),
        (status = 400, description = "Invalid query parameter", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_leaderboard() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/tags", tag = "pools",
    responses(
        (status = 200, description = "All tags in use", body = DataEnvelope),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_list_tags() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/users/{address}/history", tag = "predictions",
    params(
        ("address" = String, Path, description = "Stellar account address (G...)"),
        ("limit" = Option<i64>, Query, description = "Max results (default 20, max 100)"),
        ("offset" = Option<i64>, Query, description = "Pagination offset (default 0)"),
    ),
    responses(
        (status = 200, description = "Paginated prediction history", body = PredictionHistoryEnvelope),
        (status = 400, description = "Invalid address or query parameter", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_user_history() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/users/{address}/predictions", tag = "predictions",
    params(
        ("address" = String, Path, description = "Stellar account address (G...)"),
        ("limit" = Option<i64>, Query, description = "Max results (default 20, max 100)"),
        ("offset" = Option<i64>, Query, description = "Pagination offset (default 0)"),
    ),
    responses(
        (status = 200, description = "Enhanced predictions with pool status", body = UserPredictionsEnvelope),
        (status = 400, description = "Invalid address or query parameter", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_user_predictions() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/users/{address}/profile", tag = "users",
    params(("address" = String, Path, description = "Stellar account address (G...)")),
    responses(
        (status = 200, description = "User profile and statistics", body = UserProfileEnvelope),
        (status = 400, description = "Invalid address", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_user_profile() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/users/{address}/interests", tag = "users",
    params(("address" = String, Path, description = "Stellar account address (G...)")),
    responses(
        (status = 200, description = "Notification interests", body = DataEnvelope),
        (status = 400, description = "Invalid address", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_user_interests() {}

#[allow(dead_code)]
#[utoipa::path(put, path = "/api/v1/users/{address}/interests", tag = "users",
    params(("address" = String, Path, description = "Stellar account address (G...)")),
    request_body = SetInterestsRequestDoc,
    responses(
        (status = 200, description = "Interests replaced", body = DataEnvelope),
        (status = 400, description = "Invalid address or body", body = ErrorResponse),
        (status = 413, description = "Request body too large", body = ErrorResponse),
        (status = 415, description = "Missing JSON content type", body = ErrorResponse),
        (status = 422, description = "Body does not match the schema", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_set_user_interests() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/notifications/{address}", tag = "users",
    params(
        ("address" = String, Path, description = "Stellar account address (G...)"),
        ("unread_only" = Option<bool>, Query, description = "Only unread notifications"),
        ("limit" = Option<i64>, Query, description = "Max results (1-100)"),
        ("offset" = Option<i64>, Query, description = "Pagination offset"),
    ),
    responses(
        (status = 200, description = "Notifications, newest first", body = DataEnvelope),
        (status = 400, description = "Invalid address or query parameter", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_list_notifications() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/api/v1/notifications/{address}/read", tag = "users",
    params(("address" = String, Path, description = "Stellar account address (G...)")),
    request_body = MarkNotificationsReadRequestDoc,
    responses(
        (status = 200, description = "Number of notifications marked read", body = DataEnvelope),
        (status = 400, description = "Invalid address or body", body = ErrorResponse),
        (status = 413, description = "Request body too large", body = ErrorResponse),
        (status = 415, description = "Missing JSON content type", body = ErrorResponse),
        (status = 422, description = "Body does not match the schema", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_mark_notifications_read() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/markets/{market_id}/predictions", tag = "predictions",
    params(
        ("market_id" = i64, Path, description = "On-chain pool (market) identifier"),
        ("after" = Option<String>, Query, description = "Opaque cursor from previous page's `next_cursor` field"),
        ("limit" = Option<i64>, Query, description = "Page size (1–100, default 20)"),
    ),
    responses(
        (status = 200, description = "Cursor-paginated predictions for the market", body = MarketPredictionsEnvelope),
        (status = 400, description = "Malformed market id or query parameter", body = ErrorResponse),
        (status = 404, description = "Market not found", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_market_predictions() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/referrals/{address}", tag = "referrals",
    params(("address" = String, Path, description = "Stellar referrer address (G...)")),
    responses(
        (status = 200, description = "Referral summary", body = ReferralEarningsResponse),
        (status = 400, description = "Invalid address", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not configured", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_referrals() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/referrals/{address}/estimate", tag = "referrals",
    params(("address" = String, Path, description = "Stellar referrer address (G...)")),
    responses(
        (status = 200, description = "Estimated referral reward", body = DataEnvelope),
        (status = 400, description = "Invalid address", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not configured", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_referral_estimate() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/users/{address}/referrals", tag = "referrals",
    params(("address" = String, Path, description = "Stellar referrer address (G...)")),
    responses(
        (status = 200, description = "Per-pool referral earnings", body = ReferralEarningsResponse),
        (status = 400, description = "Invalid address", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not configured", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_user_referral_earnings() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/creators/{address}/stats", tag = "users",
    params(("address" = String, Path, description = "Creator Stellar address (G...)")),
    responses(
        (status = 200, description = "Creator statistics", body = DataEnvelope),
        (status = 400, description = "Invalid address", body = ErrorResponse),
        (status = 404, description = "Creator has no pools", body = ErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ErrorResponse),
    )
)]
async fn api_get_creator_stats() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/api/v1/auth/refresh", tag = "auth",
    request_body = RefreshTokenRequestDoc,
    responses(
        (status = 200, description = "New access/refresh token pair", body = DataEnvelope),
        (status = 400, description = "Malformed body", body = ErrorResponse),
        (status = 401, description = "Invalid, expired, or already-rotated refresh token", body = ErrorResponse),
        (status = 413, description = "Request body too large", body = ErrorResponse),
        (status = 415, description = "Missing JSON content type", body = ErrorResponse),
        (status = 422, description = "Body does not match the schema", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 503, description = "Revocation store or rate limiter unavailable (fails closed)", body = ServiceUnavailableResponse),
    )
)]
async fn api_refresh_tokens() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/api/v1/auth/logout", tag = "auth",
    request_body = LogoutRequestDoc,
    responses(
        (status = 200, description = "Session (or all sessions) revoked", body = DataEnvelope),
        (status = 400, description = "Malformed JSON body", body = ErrorResponse),
        (status = 401, description = "Missing, invalid, expired or revoked access token", body = ErrorResponse),
        (status = 403, description = "Refresh token belongs to a different user", body = ErrorResponse),
        (status = 413, description = "Request body too large", body = ErrorResponse),
        (status = 415, description = "Body present with a non-JSON content type", body = ErrorResponse),
        (status = 422, description = "Body does not match the schema", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 503, description = "Revocation store or rate limiter unavailable (fails closed)", body = ServiceUnavailableResponse),
    )
)]
async fn api_logout() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/api/v1/indexer/pool-created", tag = "indexer",
    params(
        ("Idempotency-Key" = Option<String>, Header, description = "Client-chosen key (1-255 visible ASCII chars) making retries safe; requires a bearer access token. Replays carry `Idempotent-Replayed: true`."),
    ),
    request_body = PoolCreatedPayloadDoc,
    responses(
        (status = 200, description = "Pool indexed successfully", body = IndexerOkEnvelope),
        (status = 400, description = "Malformed body or Idempotency-Key", body = ErrorResponse),
        (status = 401, description = "Idempotency-Key sent without a valid bearer access token", body = ErrorResponse),
        (status = 409, description = "A request with this Idempotency-Key is still in progress (retry after `Retry-After`)", body = ErrorResponse),
        (status = 413, description = "Request body too large", body = ErrorResponse),
        (status = 415, description = "Missing JSON content type", body = ErrorResponse),
        (status = 422, description = "Body does not match the schema, or Idempotency-Key reused with a different request", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database, idempotency store, or rate limiter unavailable", body = ServiceUnavailableResponse),
    )
)]
async fn api_ingest_pool_created() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/api/v1/indexer/prediction-placed", tag = "indexer",
    params(
        ("Idempotency-Key" = Option<String>, Header, description = "Client-chosen key (1-255 visible ASCII chars) making retries safe; requires a bearer access token. Replays carry `Idempotent-Replayed: true`."),
    ),
    request_body = PredictionPlacedPayloadDoc,
    responses(
        (status = 200, description = "Prediction indexed successfully", body = IndexerOkEnvelope),
        (status = 400, description = "Malformed body or Idempotency-Key", body = ErrorResponse),
        (status = 401, description = "Idempotency-Key sent without a valid bearer access token", body = ErrorResponse),
        (status = 409, description = "A request with this Idempotency-Key is still in progress (retry after `Retry-After`)", body = ErrorResponse),
        (status = 413, description = "Request body too large", body = ErrorResponse),
        (status = 415, description = "Missing JSON content type", body = ErrorResponse),
        (status = 422, description = "Body does not match the schema, or Idempotency-Key reused with a different request", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database, idempotency store, or rate limiter unavailable", body = ServiceUnavailableResponse),
    )
)]
async fn api_ingest_prediction_placed() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/api/v1/indexer/claim", tag = "indexer",
    params(
        ("Idempotency-Key" = Option<String>, Header, description = "Client-chosen key (1-255 visible ASCII chars) making retries safe; requires a bearer access token. Replays carry `Idempotent-Replayed: true`."),
    ),
    request_body = ClaimPayloadDoc,
    responses(
        (status = 200, description = "Claim recorded; `data.rows_updated` predictions marked claimed", body = DataEnvelope),
        (status = 400, description = "Malformed body or Idempotency-Key", body = ErrorResponse),
        (status = 401, description = "Idempotency-Key sent without a valid bearer access token", body = ErrorResponse),
        (status = 409, description = "A request with this Idempotency-Key is still in progress (retry after `Retry-After`)", body = ErrorResponse),
        (status = 413, description = "Request body too large", body = ErrorResponse),
        (status = 415, description = "Missing JSON content type", body = ErrorResponse),
        (status = 422, description = "Body does not match the schema, or Idempotency-Key reused with a different request", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database, idempotency store, or rate limiter unavailable", body = ServiceUnavailableResponse),
    )
)]
async fn api_ingest_claim() {}

// ── Portfolio & market history (#309-#314) ──────────────────────────────────────

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/wallets/{address}/activity", tag = "portfolio",
    params(
        ("address" = String, Path, description = "Stellar wallet address (G...)"),
        ("after" = Option<String>, Query, description = "Opaque cursor from previous page's `next_cursor` field"),
        ("limit" = Option<i64>, Query, description = "Page size (1-100, default 20)"),
    ),
    responses(
        (status = 200, description = "Cursor-paginated, typed wallet activity feed", body = WalletActivityEnvelope),
        (status = 400, description = "Invalid address or cursor", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_wallet_activity() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/wallets/{address}/pnl", tag = "portfolio",
    params(("address" = String, Path, description = "Stellar wallet address (G...)")),
    responses(
        (status = 200, description = "Centralized, typed portfolio P&L across every position", body = WalletPnlEnvelope),
        (status = 400, description = "Invalid address", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_wallet_pnl() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/wallets/{address}/snapshot", tag = "portfolio",
    params(("address" = String, Path, description = "Stellar wallet address (G...)")),
    responses(
        (status = 200, description = "Portfolio snapshot generated against the latest committed checkpoint", body = PortfolioSnapshotEnvelope),
        (status = 400, description = "Invalid address", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Indexer checkpoint not ready, or database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_wallet_snapshot() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/api/v1/wallets/{address}/exports", tag = "portfolio",
    params(("address" = String, Path, description = "Stellar wallet address (G...)")),
    request_body = CreateExportRequestDoc,
    responses(
        (status = 202, description = "Export job accepted; poll its status endpoint for progress", body = ExportStatusEnvelope),
        (status = 400, description = "Invalid address or malformed body", body = ErrorResponse),
        (status = 401, description = "Missing or invalid bearer access token", body = ErrorResponse),
        (status = 403, description = "Token does not authorize this wallet", body = ErrorResponse),
        (status = 413, description = "Request body too large", body = ErrorResponse),
        (status = 415, description = "Missing JSON content type", body = ErrorResponse),
        (status = 422, description = "Body does not match the schema", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_create_wallet_export() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/portfolio/exports/{id}", tag = "portfolio",
    params(("id" = i64, Path, description = "Export job identifier")),
    responses(
        (status = 200, description = "Export status and progress", body = ExportStatusEnvelope),
        (status = 400, description = "Malformed export id", body = ErrorResponse),
        (status = 401, description = "Missing or invalid bearer access token", body = ErrorResponse),
        (status = 403, description = "Token does not authorize the export's owning wallet", body = ErrorResponse),
        (status = 404, description = "Export not found", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_export_status() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/portfolio/exports/{id}/download", tag = "portfolio",
    params(("id" = i64, Path, description = "Export job identifier")),
    responses(
        (status = 200, description = "Finished CSV artifact", body = String, content_type = "text/csv"),
        (status = 400, description = "Malformed export id", body = ErrorResponse),
        (status = 401, description = "Missing or invalid bearer access token", body = ErrorResponse),
        (status = 403, description = "Token does not authorize the export's owning wallet", body = ErrorResponse),
        (status = 404, description = "Export not found, or not ready yet", body = ErrorResponse),
        (status = 410, description = "Export artifact has expired", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_download_export() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/api/v1/portfolio/exports/{id}/cancel", tag = "portfolio",
    params(("id" = i64, Path, description = "Export job identifier")),
    responses(
        (status = 200, description = "Export cancelled", body = DataEnvelope),
        (status = 400, description = "Malformed export id", body = ErrorResponse),
        (status = 401, description = "Missing or invalid bearer access token", body = ErrorResponse),
        (status = 403, description = "Token does not authorize the export's owning wallet", body = ErrorResponse),
        (status = 404, description = "Export not found", body = ErrorResponse),
        (status = 409, description = "Export already reached a terminal state", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_cancel_export() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/api/v1/markets/{pool_id}/history", tag = "markets",
    params(
        ("pool_id" = i64, Path, description = "On-chain pool (market) identifier"),
        ("bucket" = String, Query, description = "Bucket width: 1m | 5m | 1h | 1d"),
        ("from" = String, Query, description = "Inclusive start of the range, RFC3339 UTC"),
        ("to" = String, Query, description = "Exclusive end of the range, RFC3339 UTC"),
    ),
    responses(
        (status = 200, description = "Ordered, bucketed historical market-value samples", body = MarketHistoryEnvelope),
        (status = 400, description = "Invalid pool id, bucket width, timestamp, range order, or a range exceeding the maximum bucket count", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = RateLimitErrorResponse),
        (status = 500, description = "Database error", body = ErrorResponse),
        (status = 503, description = "Indexer checkpoint not ready, or database not available", body = ServiceUnavailableResponse),
    )
)]
async fn api_get_market_history() {}

// ── Router ────────────────────────────────────────────────────────────────────

/// Serve the OpenAPI spec as JSON at `/api-docs/openapi.json`.
///
/// The full Swagger UI requires `utoipa-swagger-ui` which is not available in
/// this build environment.  The raw JSON spec is still useful for tooling
/// (Insomnia, Postman, code generators, etc.).
pub fn swagger_router() -> Router {
    Router::new().route(
        "/api-docs/openapi.json",
        get(|| async { Json(openapi_json()) }),
    )
}

// ── Contract validation ───────────────────────────────────────────────────────

/// The OpenAPI document as JSON with object keys sorted at every level.
///
/// Sorting makes the output independent of map iteration order and of
/// whether `serde_json`'s `preserve_order` feature is enabled anywhere in
/// the dependency graph, so regeneration is byte-for-byte reproducible.
pub fn openapi_json() -> serde_json::Value {
    let value =
        serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI document must serialize to JSON");
    sort_keys(value)
}

/// The canonical artifact text: pretty-printed [`openapi_json`] plus a
/// trailing newline. This is exactly what `openapi.json` must contain.
pub fn openapi_artifact() -> String {
    let mut text =
        serde_json::to_string_pretty(&openapi_json()).expect("OpenAPI JSON must serialize");
    text.push('\n');
    text
}

fn sort_keys(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(k, v)| (k, sort_keys(v)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_keys).collect()),
        other => other,
    }
}

/// Canonical path of the committed OpenAPI artifact used by the clean-diff check.
pub const OPENAPI_ARTIFACT_PATH: &str = "openapi.json";
