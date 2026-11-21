use std::{collections::HashMap, env, fmt, num::ParseIntError};

const DEFAULT_HOST: &str = "0.0.0.0";
const PRODUCTION_ENV_VALUE: &str = "production";
const DEFAULT_PORT: u16 = 3000;
const DEFAULT_DATABASE_URL: &str = "postgres://postgres:postgres@localhost:5432/predifi";
const DEFAULT_DB_MAX_CONNECTIONS: u32 = 10;
const DEFAULT_DB_MIN_CONNECTIONS: u32 = 1;
const DEFAULT_DB_ACQUIRE_TIMEOUT_SECS: u64 = 30;
const DEFAULT_DB_CONNECT_MAX_ATTEMPTS: u32 = 5;
const DEFAULT_DB_CONNECT_BASE_DELAY_MS: u64 = 200;
const DEFAULT_DB_CONNECT_MAX_DELAY_MS: u64 = 5_000;
const DEFAULT_DB_CONNECT_TIMEOUT_SECS: u64 = 10;
const DEFAULT_RPC_HEALTH_TIMEOUT_SECS: u64 = 2;
const DEFAULT_RPC_HEALTH_RETRY_COUNT: u8 = 3;
const DEFAULT_RPC_TIMEOUT_SECS: u64 = 30;
const DEFAULT_SHUTDOWN_TIMEOUT_SECS: u64 = crate::constants::DEFAULT_SHUTDOWN_TIMEOUT_SECS;
const DEFAULT_LOG_LEVEL: &str = "info";
const DEFAULT_STELLAR_RPC_URL: &str = "https://soroban-testnet.stellar.org";
const DEFAULT_TREASURY_FEE_BPS: u32 = 300;
const DEFAULT_REFERRAL_FEE_BPS: u32 = 5000;
const DEFAULT_REDIS_URL: &str = "redis://localhost:6379";
const DEFAULT_REDIS_REQUIRED: bool = false;
/// Default TTL, in seconds, for negative (pool-not-found) entries in the
/// in-memory pool cache. Deliberately lower than the pool cache's positive
/// TTL (10s, see `crate::pool_cache::POOL_CACHE_TTL`) per
/// `PoolCache::with_negative_ttl`'s contract.
const DEFAULT_POOL_NEGATIVE_CACHE_TTL_SECS: u64 = 5;
const DEFAULT_SECRET_KEY: &str = "predifi-dev-secret-do-not-use-in-production-32";
const DEFAULT_APP_ENV: &str = "development";
const DEFAULT_INDEXER_MAX_BATCH_SIZE: usize = crate::constants::DEFAULT_INDEXER_MAX_BATCH_SIZE;

// ── Contract identity pinning (issue #347) ───────────────────────────────────
/// Environment variable holding the path to the pinned deployment manifest.
const MANIFEST_PATH_ENV: &str = "PREDIFI_CONTRACT_MANIFEST_PATH";
/// Environment variable holding the contract ID the backend is configured for.
const CONTRACT_ID_ENV: &str = "PREDIFI_CONTRACT_ID";
/// Environment variable holding the network passphrase the backend expects.
const NETWORK_PASSPHRASE_ENV: &str = "PREDIFI_NETWORK_PASSPHRASE";
/// Environment variable holding the contract interface version the backend
/// was built against.
const CONTRACT_INTERFACE_VERSION_ENV: &str = "PREDIFI_CONTRACT_INTERFACE_VERSION";
/// Interface version this build of the backend supports.
pub const SUPPORTED_CONTRACT_INTERFACE_VERSION: u32 = 1;
/// Default network passphrase used when none is configured (Stellar testnet).
const DEFAULT_NETWORK_PASSPHRASE: &str = "Test SDF Network ; September 2015";
/// Default contract interface version used when none is configured.
const DEFAULT_CONTRACT_INTERFACE_VERSION: u32 = SUPPORTED_CONTRACT_INTERFACE_VERSION;
/// Default path to the pinned deployment manifest.
const DEFAULT_MANIFEST_PATH: &str = "deployments/contract-manifest.json";
/// Maximum age, in seconds, a manifest may have before it is considered stale.
const DEFAULT_MANIFEST_MAX_AGE_SECS: u64 = 86_400;
/// Environment variable overriding the maximum manifest age.
const MANIFEST_MAX_AGE_ENV: &str = "PREDIFI_CONTRACT_MANIFEST_MAX_AGE_SECS";

/// A pinned deployment manifest describing the contract this backend is
/// allowed to talk to.
///
/// The manifest is the source of truth for contract identity: the configured
/// contract ID, network passphrase and interface version must all match it
/// before any mutation is permitted. There is deliberately no fallback to an
/// unverified contract — a missing, stale or mismatched manifest disables
/// mutations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentManifest {
    /// The contract ID (Stellar contract address) pinned by the deployment.
    pub contract_id: String,
    /// The network passphrase the contract is deployed on.
    pub network_passphrase: String,
    /// The contract interface version the manifest was produced for.
    pub interface_version: u32,
    /// Unix timestamp (seconds) at which the manifest was generated.
    pub generated_at: u64,
}

impl DeploymentManifest {
    /// Parse a manifest from its JSON representation.
    ///
    /// The parser is intentionally minimal (no external dependency) and
    /// accepts a flat JSON object with the keys `contract_id`,
    /// `network_passphrase`, `interface_version` and `generated_at`.
    pub fn from_json(raw: &str) -> Result<Self, ManifestError> {
        let contract_id = json_string_field(raw, "contract_id")
            .ok_or_else(|| ManifestError::Malformed(String::from("missing 'contract_id'")))?;
        let network_passphrase = json_string_field(raw, "network_passphrase").ok_or_else(|| {
            ManifestError::Malformed(String::from("missing 'network_passphrase'"))
        })?;
        let interface_version = json_u64_field(raw, "interface_version").ok_or_else(|| {
            ManifestError::Malformed(String::from("missing 'interface_version'"))
        })?;
        let generated_at = json_u64_field(raw, "generated_at")
            .ok_or_else(|| ManifestError::Malformed(String::from("missing 'generated_at'")))?;

        if contract_id.trim().is_empty() {
            return Err(ManifestError::Malformed(String::from(
                "'contract_id' must not be empty",
            )));
        }
        if network_passphrase.trim().is_empty() {
            return Err(ManifestError::Malformed(String::from(
                "'network_passphrase' must not be empty",
            )));
        }
        let interface_version = u32::try_from(interface_version).map_err(|_| {
            ManifestError::Malformed(String::from("'interface_version' out of range"))
        })?;

        Ok(Self {
            contract_id,
            network_passphrase,
            interface_version,
            generated_at,
        })
    }
}

/// Errors produced while loading or parsing a deployment manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// The manifest file could not be read.
    Unavailable(String),
    /// The manifest contents could not be parsed.
    Malformed(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(reason) => write!(f, "manifest unavailable: {reason}"),
            Self::Malformed(reason) => write!(f, "manifest malformed: {reason}"),
        }
    }
}

impl std::error::Error for ManifestError {}

/// Outcome of verifying the configured contract identity against the pinned
/// deployment manifest.
///
/// The server/router uses this to gate mutation routes: only
/// [`ContractIdentityStatus::Verified`] permits mutations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractIdentityStatus {
    /// The configured identity matches the pinned manifest and the manifest is
    /// fresh and supported.
    Verified {
        /// The verified contract ID.
        contract_id: String,
        /// The verified network passphrase.
        network_passphrase: String,
        /// The verified interface version.
        interface_version: u32,
    },
    /// The configured identity does not match the pinned manifest.
    Mismatch {
        /// Human-readable description of the mismatch.
        reason: String,
    },
    /// The manifest could not be loaded.
    Unavailable {
        /// Human-readable description of why the manifest is unavailable.
        reason: String,
    },
    /// The manifest is stale (older than the configured maximum age).
    Stale {
        /// Human-readable description of why the manifest is stale.
        reason: String,
    },
}

impl ContractIdentityStatus {
    /// `true` only when the contract identity has been verified.
    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Verified { .. })
    }

    /// `true` when mutations must be disabled.
    pub fn mutations_disabled(&self) -> bool {
        !self.is_verified()
    }

    /// A stable machine-readable code for diagnostics.
    pub fn diagnostic_code(&self) -> &'static str {
        match self {
            Self::Verified { .. } => "contract_identity_verified",
            Self::Mismatch { .. } => "contract_identity_mismatch",
            Self::Unavailable { .. } => "contract_identity_unavailable",
            Self::Stale { .. } => "contract_identity_stale",
        }
    }

    /// A human-readable diagnostic message.
    pub fn diagnostic_message(&self) -> String {
        match self {
            Self::Verified {
                contract_id,
                network_passphrase,
                interface_version,
            } => format!(
                "contract identity verified: id={contract_id} network={network_passphrase} interface_version={interface_version}"
            ),
            Self::Mismatch { reason } => {
                format!("contract identity mismatch: {reason}; mutations disabled")
            }
            Self::Unavailable { reason } => {
                format!("contract identity unavailable: {reason}; mutations disabled")
            }
            Self::Stale { reason } => {
                format!("contract identity stale: {reason}; mutations disabled")
            }
        }
    }
}

/// The configured contract identity, as read from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractIdentityConfig {
    /// Path to the pinned deployment manifest.
    pub manifest_path: String,
    /// Contract ID the backend is configured to talk to.
    pub contract_id: String,
    /// Network passphrase the backend expects.
    pub network_passphrase: String,
    /// Contract interface version the backend was built against.
    pub interface_version: u32,
    /// Maximum age (seconds) a manifest may have before it is stale.
    pub manifest_max_age_secs: u64,
}

impl ContractIdentityConfig {
    /// Verify this identity against the pinned manifest.
    ///
    /// `now_unix_secs` is the current Unix time in seconds; it is passed in so
    /// callers (and tests) control staleness evaluation.
    pub fn verify(&self, now_unix_secs: u64) -> ContractIdentityStatus {
        let raw = match std::fs::read_to_string(&self.manifest_path) {
            Ok(raw) => raw,
            Err(err) => {
                return ContractIdentityStatus::Unavailable {
                    reason: format!("could not read '{}': {err}", self.manifest_path),
                };
            }
        };

        let manifest = match DeploymentManifest::from_json(&raw) {
            Ok(manifest) => manifest,
            Err(err) => {
                return ContractIdentityStatus::Unavailable {
                    reason: format!("'{}': {err}", self.manifest_path),
                };
            }
        };

        // Staleness is checked before identity so a stale manifest never
        // silently authorises a matching-but-outdated contract.
        if self.manifest_max_age_secs > 0 {
            let age = now_unix_secs.saturating_sub(manifest.generated_at);
            if age > self.manifest_max_age_secs {
                return ContractIdentityStatus::Stale {
                    reason: format!(
                        "manifest generated_at={} is {}s old (max {}s)",
                        manifest.generated_at, age, self.manifest_max_age_secs
                    ),
                };
            }
        }

        if manifest.contract_id != self.contract_id {
            return ContractIdentityStatus::Mismatch {
                reason: format!(
                    "configured contract_id '{}' does not match pinned '{}'",
                    self.contract_id, manifest.contract_id
                ),
            };
        }
        if manifest.network_passphrase != self.network_passphrase {
            return ContractIdentityStatus::Mismatch {
                reason: format!(
                    "configured network_passphrase '{}' does not match pinned '{}'",
                    self.network_passphrase, manifest.network_passphrase
                ),
            };
        }
        if manifest.interface_version != self.interface_version {
            return ContractIdentityStatus::Mismatch {
                reason: format!(
                    "configured interface_version {} does not match pinned {}",
                    self.interface_version, manifest.interface_version
                ),
            };
        }
        if manifest.interface_version != SUPPORTED_CONTRACT_INTERFACE_VERSION {
            return ContractIdentityStatus::Mismatch {
                reason: format!(
                    "pinned interface_version {} is not supported by this build (supports {})",
                    manifest.interface_version, SUPPORTED_CONTRACT_INTERFACE_VERSION
                ),
            };
        }

        ContractIdentityStatus::Verified {
            contract_id: manifest.contract_id,
            network_passphrase: manifest.network_passphrase,
            interface_version: manifest.interface_version,
        }
    }
}

/// Extract a JSON string field from a flat JSON object.
fn json_string_field(raw: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = raw.find(&needle)? + needle.len();
    let rest = raw[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
    None
}

/// Extract a JSON unsigned-integer field from a flat JSON object.
fn json_u64_field(raw: &str, key: &str) -> Option<u64> {
    let needle = format!("\"{key}\"");
    let start = raw.find(&needle)? + needle.len();
    let rest = raw[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok()
}

// ── Database connection budget (issue #353) ──────────────────────────────────
/// Default PostgreSQL server-side connection limit (`max_connections`).
const DEFAULT_DB_SERVER_MAX_CONNECTIONS: u32 = 100;
/// Default number of connections reserved for administration (superuser,
/// migrations, monitoring, `psql` sessions) and left out of the service budget.
const DEFAULT_DB_ADMIN_HEADROOM_CONNECTIONS: u32 = 10;
/// Default maximum number of backend replicas that may run concurrently.
const DEFAULT_DB_MAX_REPLICAS: u32 = 1;
/// Default number of connections each replica's pool may hold. Kept in sync
/// with [`DEFAULT_DB_MAX_CONNECTIONS`] so the budget check is meaningful even
/// when only the replica count is overridden.
const DEFAULT_DB_POOL_SIZE_PER_REPLICA: u32 = DEFAULT_DB_MAX_CONNECTIONS;
/// Default number of additional non-HTTP services (indexer, workers) that each
/// hold their own pool and therefore consume from the same budget.
const DEFAULT_DB_AUXILIARY_SERVICES: u32 = 0;
/// Default number of connections each auxiliary service's pool may hold.
const DEFAULT_DB_AUXILIARY_POOL_SIZE: u32 = DEFAULT_DB_MAX_CONNECTIONS;

// ── Abuse-control defaults (issue #103) ──────────────────────────────────────
/// Default number of market-creation requests a single wallet may make per
/// abuse-control window.
const DEFAULT_ABUSE_QUOTA_MARKET_CREATE_PER_WINDOW: u32 = 5;
/// Default number of expensive wallet actions (e.g. transaction preparation)
/// a single wallet may trigger per abuse-control window.
const DEFAULT_ABUSE_QUOTA_WALLET_ACTION_PER_WINDOW: u32 = 30;
/// Default length of the abuse-control sliding window, in seconds.
const DEFAULT_ABUSE_QUOTA_WINDOW_SECS: u64 = 60;
/// Default network-wide ceiling on market-creation requests per window. This
/// bounds total backend capacity regardless of how many wallets are cycled.
const DEFAULT_ABUSE_QUOTA_NETWORK_MARKET_CREATE_PER_WINDOW: u32 = 500;
/// Default network-wide ceiling on wallet actions per window.
const DEFAULT_ABUSE_QUOTA_NETWORK_WALLET_ACTION_PER_WINDOW: u32 = 3_000;
/// Default minimum validation cost (in abstract units) that must be paid
/// before a request is admitted. Raising this makes account cycling more
/// expensive for automated abusers.
const DEFAULT_ABUSE_MIN_VALIDATION_COST: u32 = 1;
/// Default number of comments a single wallet may post per abuse-control window.
const DEFAULT_ABUSE_QUOTA_COMMENT_PER_WINDOW: u32 = 20;
/// Default network-wide ceiling on comments per abuse-control window.
const DEFAULT_ABUSE_QUOTA_NETWORK_COMMENT_PER_WINDOW: u32 = 500;
/// Default number of reports a single wallet may submit per abuse-control window.
const DEFAULT_ABUSE_QUOTA_REPORT_PER_WINDOW: u32 = 10;
/// Default network-wide ceiling on reports per abuse-control window.
const DEFAULT_ABUSE_QUOTA_NETWORK_REPORT_PER_WINDOW: u32 = 200;

// ── RPC quota / failover defaults (issue #105) ───────────────────────────────
/// Default per-network request budget (requests per window) for the RPC layer.
const DEFAULT_RPC_PER_NETWORK_BUDGET: u32 = 1_000;
/// Default length of the RPC budget window, in seconds.
const DEFAULT_RPC_BUDGET_WINDOW_SECS: u64 = 60;
/// Default maximum number of retries for a single RPC request.
const DEFAULT_RPC_MAX_RETRIES: u8 = 2;
/// Default number of consecutive failures before a provider is marked unhealthy.
const DEFAULT_RPC_PROVIDER_UNHEALTHY_THRESHOLD: u8 = 3;
/// Default number of consecutive successes before an unhealthy provider is
/// considered healthy again.
const DEFAULT_RPC_PROVIDER_HEALTHY_THRESHOLD: u8 = 2;
/// Default policy applied when providers return conflicting results.
const DEFAULT_RPC_DISAGREEMENT_POLICY: &str = "quorum";
/// Default comma-separated priority classes, highest priority first.
const DEFAULT_RPC_PRIORITY_CLASSES: &str = "transaction,indexer,health";
/// Default comma-separated provider fallback order. Empty means "use the
/// configured `PREDIFI_STELLAR_RPC_URL` only".
const DEFAULT_RPC_PROVIDER_FALLBACK_ORDER: &str = "";

// ── Notification provider credentials (issue #355) ───────────────────────────
/// Default notification provider endpoint. Empty means "no provider configured".
const DEFAULT_NOTIFICATION_PROVIDER_URL: &str = "";
/// Whether dual-key (previous + current) notification credentials may be used
/// during a rotation (default `false`). Read from
/// `PREDIFI_NOTIFICATION_DUAL_KEY_ENABLED`.
const DEFAULT_NOTIFICATION_DUAL_KEY_ENABLED: bool = false;

/// Origins allowed by default when `CORS_ALLOWED_ORIGINS` is not set.
pub const DEFAULT_CORS_ORIGINS: &[&str] = &[
    "http://localhost:3000",
    "http://localhost:5173",
    "https://predifi.app",
];

/// Map the configured Stellar RPC endpoint to its token-bound network name.
pub fn stellar_network_for_rpc_url(url: &str) -> &'static str {
    let url = url.to_ascii_lowercase();
    if url.contains("futurenet") {
        "futurenet"
    } else if url.contains("testnet")
        || url.contains("localhost")
        || url.contains("127.0.0.1")
        || url.contains("stellar:8000")
    {
        "testnet"
    } else {
        "public"
    }
}

/// Runtime configuration loaded from environment variables.
///
/// All fields have compiled-in defaults so the server can start without any
/// environment variables set (useful for local development and unit tests).
/// Call [`Config::from_env`] at startup to populate the struct from the
/// process environment, or [`Config::from_map`] in tests to supply values
/// directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// IP address the HTTP server binds to (default `0.0.0.0`).
    pub host: String,
    /// TCP port the HTTP server listens on (default `3000`).
    pub port: u16,
    /// PostgreSQL connection string (default points to a local `predifi` DB).
    pub database_url: String,
    /// Maximum number of connections in the SQLx pool (default `10`).
    pub db_max_connections: u32,
    /// Minimum number of idle connections kept alive in the pool (default `1`).
    pub db_min_connections: u32,
    /// Seconds to wait when acquiring a connection from the pool (default `30`).
    pub db_acquire_timeout_secs: u64,
    /// Max startup attempts when connecting to Postgres (default `5`).
    pub db_connect_max_attempts: u32,
    /// Initial backoff delay in ms between startup connection attempts (default `200`).
    pub db_connect_base_delay_ms: u64,
    /// Maximum backoff delay in ms between startup connection attempts (default `5000`).
    pub db_connect_max_delay_ms: u64,
    /// Per-connection TCP/TLS handshake timeout in seconds for each individual
    /// connection the pool creates (default `10`).  This is distinct from
    /// `db_acquire_timeout_secs`, which governs how long a caller waits for a
    /// slot in an already-open pool; this field controls how long sqlx waits
    /// for the underlying TCP connect + TLS handshake to complete when
    /// establishing a brand-new connection.
    pub db_connect_timeout_secs: u64,
    /// Per-attempt timeout in seconds for the Stellar RPC health check (default `2`).
    pub rpc_health_timeout_secs: u64,
    /// Number of times to retry the Stellar RPC health check before reporting failure (default `3`).
    pub rpc_health_retry_count: u8,
    /// Per-attempt timeout in seconds for general Stellar RPC calls (default `30`).
    pub rpc_timeout_secs: u64,
    /// Maximum number of seconds the HTTP server is allowed to spend
    /// draining in-flight requests after a termination signal is received.
    ///
    /// Once this window closes, any handlers that are still running are
    /// dropped, the database pool is asked to close, and the process exits.
    /// Default [`DEFAULT_SHUTDOWN_TIMEOUT_SECS`] (30 s).
    pub shutdown_timeout_secs: u64,
    /// Tracing log level passed to `RUST_LOG` / `EnvFilter` (default `"info"`).
    pub log_level: String,
    /// Protocol treasury fee in basis points (default `300` = 3 %).
    pub treasury_fee_bps: u32,
    /// Referral share of the protocol fee in basis points (default `5000` = 50 % of the fee).
    pub referral_fee_bps: u32,
    /// Stellar Soroban RPC endpoint URL (default: testnet).
    pub stellar_rpc_url: String,
    /// Optional Sentry DSN for error reporting. `None` disables Sentry.
    pub sentry_dsn: Option<String>,
    /// Redis connection URL (default `redis://localhost:6379`).
    pub redis_url: String,
    /// Whether Redis is required for the service to be considered healthy
    /// (default `false`).
    ///
    /// Read from `PREDIFI_REDIS_REQUIRED` (`"true"`/`"false"`, case-insensitive).
    /// Controls startup behaviour when Redis is unreachable:
    /// - `true`: startup fails fast with a clear error and the process exits.
    /// - `false`: startup logs a single explicit warning and continues in a
    ///   degraded state, which is reflected by the `/ready` endpoint.
    pub redis_required: bool,
    /// TTL in seconds for negative (pool-not-found) entries in the in-memory
    /// pool cache (default `5`). Read from
    /// `PREDIFI_POOL_NEGATIVE_CACHE_TTL_SECS`. Kept separate from — and
    /// intended to be lower than — the pool cache's positive TTL, so a pool
    /// ID that turns out not to exist is reflected as a fast cached 404
    /// without hammering Postgres, while a pool created shortly after a
    /// negative lookup becomes visible again quickly.
    pub pool_negative_cache_ttl_secs: u64,
    /// Validated list of origins permitted by the CORS policy.
    ///
    /// Loaded from the `PREDIFI_CORS_ALLOWED_ORIGINS` environment variable as a
    /// comma-separated list of origins (e.g. `https://app.example.com,https://admin.example.com`).
    /// Each entry must be a valid `http://` or `https://` origin — scheme + host (+ optional port)
    /// with no path, query string, or fragment.  Invalid entries are rejected at startup.
    ///
    /// Falls back to [`DEFAULT_CORS_ORIGINS`] when the variable is absent.
    pub cors_allowed_origins: Vec<String>,
    /// HMAC secret used to verify JWT bearer tokens (default: dev-only placeholder).
    pub secret_key: String,
    /// Signing-key version embedded in issued JWTs. Bump to invalidate all
    /// outstanding tokens after a secret rotation.
    pub jwt_key_version: u64,
    /// Maximum number of Stellar events processed per indexer batch (default `500`).
    pub indexer_max_batch_size: usize,
    /// Deployment environment name (e.g. `"production"`, `"staging"`, `"development"`).
    ///
    /// Read from `PREDIFI_APP_ENV`. When set to `"production"`, additional
    /// hardening checks are applied at startup (e.g. the default dev secret key
    /// is rejected).  Defaults to `"development"` when the variable is absent.
    pub app_env: String,
    /// Validated list of origins permitted for WebSocket connections.
    ///
    /// Loaded from the `PREDIFI_WS_ALLOWED_ORIGINS` environment variable as a
    /// comma-separated list of origins. If empty, all origins are allowed (permissive mode).
    /// In production, this should be configured to restrict WebSocket connections to trusted domains.
    /// Uses the same validation logic as CORS origins.
    pub allowed_ws_origins: Vec<String>,

    // ── Abuse controls (issue #103) ──────────────────────────────────────────
    /// Maximum market-creation requests a single wallet may make per
    /// abuse-control window (default `5`). Read from
    /// `PREDIFI_ABUSE_QUOTA_MARKET_CREATE_PER_WINDOW`.
    pub abuse_quota_market_create_per_window: u32,
    /// Maximum expensive wallet actions a single wallet may trigger per
    /// abuse-control window (default `30`). Read from
    /// `PREDIFI_ABUSE_QUOTA_WALLET_ACTION_PER_WINDOW`.
    pub abuse_quota_wallet_action_per_window: u32,
    /// Length of the abuse-control sliding window in seconds (default `60`).
    /// Read from `PREDIFI_ABUSE_QUOTA_WINDOW_SECS`.
    pub abuse_quota_window_secs: u64,
    /// Network-wide ceiling on market-creation requests per window (default
    /// `500`). Read from `PREDIFI_ABUSE_QUOTA_NETWORK_MARKET_CREATE_PER_WINDOW`.
    pub abuse_quota_network_market_create_per_window: u32,
    /// Network-wide ceiling on wallet actions per window (default `3000`).
    /// Read from `PREDIFI_ABUSE_QUOTA_NETWORK_WALLET_ACTION_PER_WINDOW`.
    pub abuse_quota_network_wallet_action_per_window: u32,
    /// Minimum validation cost that must be paid before a request is admitted
    /// (default `1`). Read from `PREDIFI_ABUSE_MIN_VALIDATION_COST`.
    pub abuse_min_validation_cost: u32,
    /// Maximum comments a single wallet may post per window (default `20`).
    /// Read from `PREDIFI_ABUSE_QUOTA_COMMENT_PER_WINDOW`.
    pub abuse_quota_comment_per_window: u32,
    /// Network-wide ceiling on comments per window (default `500`).
    /// Read from `PREDIFI_ABUSE_QUOTA_NETWORK_COMMENT_PER_WINDOW`.
    pub abuse_quota_network_comment_per_window: u32,
    /// Maximum reports a single wallet may submit per window (default `10`).
    /// Read from `PREDIFI_ABUSE_QUOTA_REPORT_PER_WINDOW`.
    pub abuse_quota_report_per_window: u32,
    /// Network-wide ceiling on reports per window (default `200`).
    /// Read from `PREDIFI_ABUSE_QUOTA_NETWORK_REPORT_PER_WINDOW`.
    pub abuse_quota_network_report_per_window: u32,

    // ── RPC quotas / failover (issue #105) ───────────────────────────────────
    /// Per-network request budget per window (default `1000`). Read from
    /// `PREDIFI_RPC_PER_NETWORK_BUDGET`.
    pub rpc_per_network_budget: u32,
    /// Length of the RPC budget window in seconds (default `60`). Read from
    /// `PREDIFI_RPC_BUDGET_WINDOW_SECS`.
    pub rpc_budget_window_secs: u64,
    /// Maximum retries for a single RPC request (default `2`). Read from
    /// `PREDIFI_RPC_MAX_RETRIES`.
    pub rpc_max_retries: u8,
    /// Priority classes, highest priority first (default
    /// `["transaction", "indexer", "health"]`). Read from
    /// `PREDIFI_RPC_PRIORITY_CLASSES`.
    pub rpc_priority_classes: Vec<String>,
    /// Provider fallback order (default empty → use `stellar_rpc_url` only).
    /// Read from `PREDIFI_RPC_PROVIDER_FALLBACK_ORDER`.
    pub rpc_provider_fallback_order: Vec<String>,
    /// Consecutive failures before a provider is marked unhealthy (default `3`).
    /// Read from `PREDIFI_RPC_PROVIDER_UNHEALTHY_THRESHOLD`.
    pub rpc_provider_unhealthy_threshold: u8,
    /// Consecutive successes before an unhealthy provider is marked healthy
    /// again (default `2`). Read from `PREDIFI_RPC_PROVIDER_HEALTHY_THRESHOLD`.
    pub rpc_provider_healthy_threshold: u8,
    /// Policy applied when providers disagree (default `"quorum"`). One of
    /// `quorum`, `first`, `strict`. Read from `PREDIFI_RPC_DISAGREEMENT_POLICY`.
    pub rpc_disagreement_policy: String,

    // ── Contract identity pinning (issue #347) ───────────────────────────────
    /// Path to the pinned deployment manifest. Read from
    /// `PREDIFI_CONTRACT_MANIFEST_PATH`.
    pub contract_manifest_path: String,
    /// Contract ID the backend is configured to talk to. Read from
    /// `PREDIFI_CONTRACT_ID`.
    pub contract_id: String,
    /// Network passphrase the backend expects. Read from
    /// `PREDIFI_NETWORK_PASSPHRASE`.
    pub network_passphrase: String,
    /// Contract interface version the backend was built against. Read from
    /// `PREDIFI_CONTRACT_INTERFACE_VERSION`.
    pub contract_interface_version: u32,
    /// Maximum age (seconds) a manifest may have before it is considered
    /// stale. Read from `PREDIFI_CONTRACT_MANIFEST_MAX_AGE_SECS`.
    pub contract_manifest_max_age_secs: u64,

    // ── Database connection budget (issue #353) ──────────────────────────────
    /// PostgreSQL server-side connection limit (`max_connections`). Default
    /// `100`. Read from `PREDIFI_DB_SERVER_MAX_CONNECTIONS`.
    pub db_server_max_connections: u32,
    /// Connections reserved for administration and left out of the service
    /// budget (default `10`). Read from `PREDIFI_DB_ADMIN_HEADROOM_CONNECTIONS`.
    pub db_admin_headroom_connections: u32,
    /// Maximum number of backend replicas that may run concurrently (default
    /// `1`). Read from `PREDIFI_DB_MAX_REPLICAS`.
    pub db_max_replicas: u32,
    /// Connections each replica's pool may hold (default `10`). Read from
    /// `PREDIFI_DB_POOL_SIZE_PER_REPLICA`.
    pub db_pool_size_per_replica: u32,
    /// Number of additional non-HTTP services (indexer, workers) that each hold
    /// their own pool (default `0`). Read from `PREDIFI_DB_AUXILIARY_SERVICES`.
    pub db_auxiliary_services: u32,
    /// Connections each auxiliary service's pool may hold (default `10`). Read
    /// from `PREDIFI_DB_AUXILIARY_POOL_SIZE`.
    pub db_auxiliary_pool_size: u32,

    // ── Notification provider credentials (issue #355) ───────────────────────
    /// Base URL of the notification provider (default empty → no provider).
    /// Read from `PREDIFI_NOTIFICATION_PROVIDER_URL`.
    pub notification_provider_url: String,
    /// Current (active) notification provider credential. Read from
    /// `PREDIFI_NOTIFICATION_PROVIDER_KEY`. Must be non-empty whenever a
    /// provider URL is configured; a revoked/empty key is rejected at startup.
    pub notification_provider_key: String,
    /// Optional previous notification provider credential, retained only while
    /// a dual-key rotation is in flight. Read from
    /// `PREDIFI_NOTIFICATION_PROVIDER_PREVIOUS_KEY`. Only honoured when
    /// [`Self::notification_dual_key_enabled`] is `true`.
    pub notification_provider_previous_key: Option<String>,
    /// Whether dual-key transition is permitted (default `false`). Read from
    /// `PREDIFI_NOTIFICATION_DUAL_KEY_ENABLED`. Must be explicitly enabled for
    /// a previous key to be accepted; providers without dual-key semantics
    /// must leave this disabled.
    pub notification_dual_key_enabled: bool,
}

impl Config {
    /// Load all runtime settings from environment variables, with defaults.
    pub fn from_env() -> Result<Self, ConfigError> {
        let vars: HashMap<String, String> = env::vars().collect();
        Self::from_map(&vars)
    }

    pub fn from_map(vars: &HashMap<String, String>) -> Result<Self, ConfigError> {
        let host = get_string(vars, "PREDIFI_APP_HOST", DEFAULT_HOST);
        let port = get_u16(vars, "PREDIFI_APP_PORT", DEFAULT_PORT)?;
        let database_url = get_string(vars, "PREDIFI_DATABASE_URL", DEFAULT_DATABASE_URL);
        let db_max_connections = get_u32(
            vars,
            "PREDIFI_DB_MAX_CONNECTIONS",
            DEFAULT_DB_MAX_CONNECTIONS,
        )?;
        let db_min_connections = get_u32(
            vars,
            "PREDIFI_DB_MIN_CONNECTIONS",
            DEFAULT_DB_MIN_CONNECTIONS,
        )?;
        let db_acquire_timeout_secs = get_u64(
            vars,
            "PREDIFI_DB_ACQUIRE_TIMEOUT_SECS",
            DEFAULT_DB_ACQUIRE_TIMEOUT_SECS,
        )?;
        let db_connect_max_attempts = get_u32(
            vars,
            "PREDIFI_DB_CONNECT_MAX_ATTEMPTS",
            DEFAULT_DB_CONNECT_MAX_ATTEMPTS,
        )?;
        let db_connect_base_delay_ms = get_u64(
            vars,
            "PREDIFI_DB_CONNECT_BASE_DELAY_MS",
            DEFAULT_DB_CONNECT_BASE_DELAY_MS,
        )?;
        let db_connect_max_delay_ms = get_u64(
            vars,
            "PREDIFI_DB_CONNECT_MAX_DELAY_MS",
            DEFAULT_DB_CONNECT_MAX_DELAY_MS,
        )?;
        let db_connect_timeout_secs = get_u64(
            vars,
            "PREDIFI_DB_CONNECT_TIMEOUT_SECS",
            DEFAULT_DB_CONNECT_TIMEOUT_SECS,
        )?;
        let rpc_health_timeout_secs = get_u64(
            vars,
            "PREDIFI_RPC_HEALTH_TIMEOUT_SECS",
            DEFAULT_RPC_HEALTH_TIMEOUT_SECS,
        )?;
        let rpc_health_retry_count = get_u8(
            vars,
            "PREDIFI_RPC_HEALTH_RETRY_COUNT",
            DEFAULT_RPC_HEALTH_RETRY_COUNT,
        )?;
        let rpc_timeout_secs = get_u64(vars, "RPC_TIMEOUT_SECS", DEFAULT_RPC_TIMEOUT_SECS)?;
        let shutdown_timeout_secs = get_u64(
            vars,
            "PREDIFI_SHUTDOWN_TIMEOUT_SECS",
            DEFAULT_SHUTDOWN_TIMEOUT_SECS,
        )?;
        if shutdown_timeout_secs == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_SHUTDOWN_TIMEOUT_SECS",
                reason: String::from("must be greater than zero so in-flight requests can drain"),
            });
        }
        let log_level = get_string(vars, "RUST_LOG", DEFAULT_LOG_LEVEL);
        let treasury_fee_bps = get_u32(vars, "PREDIFI_TREASURY_FEE_BPS", DEFAULT_TREASURY_FEE_BPS)?;
        let referral_fee_bps = get_u32(vars, "PREDIFI_REFERRAL_FEE_BPS", DEFAULT_REFERRAL_FEE_BPS)?;
        let stellar_rpc_url = get_string(vars, "PREDIFI_STELLAR_RPC_URL", DEFAULT_STELLAR_RPC_URL);
        let rpc_network = stellar_network_for_rpc_url(&stellar_rpc_url);
        if let Some(configured_network) = vars.get("PREDIFI_STELLAR_NETWORK") {
            if !matches!(
                configured_network.as_str(),
                "testnet" | "futurenet" | "public"
            ) || configured_network != rpc_network
            {
                return Err(ConfigError::InvalidValue {
                    key: "PREDIFI_STELLAR_NETWORK",
                    reason: format!(
                        "must match the configured Stellar RPC network ({rpc_network})"
                    ),
                });
            }
        }
        let sentry_dsn = vars.get("PREDIFI_SENTRY_DSN").cloned();
        let redis_url = get_string(vars, "PREDIFI_REDIS_URL", DEFAULT_REDIS_URL);
        let redis_required = get_bool(vars, "PREDIFI_REDIS_REQUIRED", DEFAULT_REDIS_REQUIRED)?;
        let pool_negative_cache_ttl_secs = get_u64(
            vars,
            "PREDIFI_POOL_NEGATIVE_CACHE_TTL_SECS",
            DEFAULT_POOL_NEGATIVE_CACHE_TTL_SECS,
        )?;

        // Parse and strictly validate CORS origins.
        let cors_allowed_origins = parse_cors_origins(vars)?;
        let secret_key = get_string(vars, "PREDIFI_SECRET_KEY", DEFAULT_SECRET_KEY);
        let jwt_key_version = get_u64(vars, "PREDIFI_JWT_KEY_VERSION", 0)?;
        let indexer_max_batch_size = get_usize(
            vars,
            "PREDIFI_INDEXER_MAX_BATCH_SIZE",
            DEFAULT_INDEXER_MAX_BATCH_SIZE,
        )?;

        // Parse WebSocket allowed origins (optional, defaults to empty for permissive mode)
        let allowed_ws_origins = parse_ws_origins(vars)?;

        let app_env = get_string(vars, "PREDIFI_APP_ENV", DEFAULT_APP_ENV);

        // ── Abuse controls (issue #103) ──────────────────────────────────────
        let abuse_quota_market_create_per_window = get_u32(
            vars,
            "PREDIFI_ABUSE_QUOTA_MARKET_CREATE_PER_WINDOW",
            DEFAULT_ABUSE_QUOTA_MARKET_CREATE_PER_WINDOW,
        )?;
        let abuse_quota_wallet_action_per_window = get_u32(
            vars,
            "PREDIFI_ABUSE_QUOTA_WALLET_ACTION_PER_WINDOW",
            DEFAULT_ABUSE_QUOTA_WALLET_ACTION_PER_WINDOW,
        )?;
        let abuse_quota_window_secs = get_u64(
            vars,
            "PREDIFI_ABUSE_QUOTA_WINDOW_SECS",
            DEFAULT_ABUSE_QUOTA_WINDOW_SECS,
        )?;
        let abuse_quota_network_market_create_per_window = get_u32(
            vars,
            "PREDIFI_ABUSE_QUOTA_NETWORK_MARKET_CREATE_PER_WINDOW",
            DEFAULT_ABUSE_QUOTA_NETWORK_MARKET_CREATE_PER_WINDOW,
        )?;
        let abuse_quota_network_wallet_action_per_window = get_u32(
            vars,
            "PREDIFI_ABUSE_QUOTA_NETWORK_WALLET_ACTION_PER_WINDOW",
            DEFAULT_ABUSE_QUOTA_NETWORK_WALLET_ACTION_PER_WINDOW,
        )?;
        let abuse_min_validation_cost = get_u32(
            vars,
            "PREDIFI_ABUSE_MIN_VALIDATION_COST",
            DEFAULT_ABUSE_MIN_VALIDATION_COST,
        )?;
        let abuse_quota_comment_per_window = get_u32(
            vars,
            "PREDIFI_ABUSE_QUOTA_COMMENT_PER_WINDOW",
            DEFAULT_ABUSE_QUOTA_COMMENT_PER_WINDOW,
        )?;
        let abuse_quota_network_comment_per_window = get_u32(
            vars,
            "PREDIFI_ABUSE_QUOTA_NETWORK_COMMENT_PER_WINDOW",
            DEFAULT_ABUSE_QUOTA_NETWORK_COMMENT_PER_WINDOW,
        )?;
        let abuse_quota_report_per_window = get_u32(
            vars,
            "PREDIFI_ABUSE_QUOTA_REPORT_PER_WINDOW",
            DEFAULT_ABUSE_QUOTA_REPORT_PER_WINDOW,
        )?;
        let abuse_quota_network_report_per_window = get_u32(
            vars,
            "PREDIFI_ABUSE_QUOTA_NETWORK_REPORT_PER_WINDOW",
            DEFAULT_ABUSE_QUOTA_NETWORK_REPORT_PER_WINDOW,
        )?;

        // ── RPC quotas / failover (issue #105) ───────────────────────────────
        let rpc_per_network_budget = get_u32(
            vars,
            "PREDIFI_RPC_PER_NETWORK_BUDGET",
            DEFAULT_RPC_PER_NETWORK_BUDGET,
        )?;
        let rpc_budget_window_secs = get_u64(
            vars,
            "PREDIFI_RPC_BUDGET_WINDOW_SECS",
            DEFAULT_RPC_BUDGET_WINDOW_SECS,
        )?;
        let rpc_max_retries = get_u8(vars, "PREDIFI_RPC_MAX_RETRIES", DEFAULT_RPC_MAX_RETRIES)?;
        let rpc_priority_classes = parse_csv_list(
            vars,
            "PREDIFI_RPC_PRIORITY_CLASSES",
            DEFAULT_RPC_PRIORITY_CLASSES,
        );
        let rpc_provider_fallback_order = parse_csv_list(
            vars,
            "PREDIFI_RPC_PROVIDER_FALLBACK_ORDER",
            DEFAULT_RPC_PROVIDER_FALLBACK_ORDER,
        );
        let rpc_provider_unhealthy_threshold = get_u8(
            vars,
            "PREDIFI_RPC_PROVIDER_UNHEALTHY_THRESHOLD",
            DEFAULT_RPC_PROVIDER_UNHEALTHY_THRESHOLD,
        )?;
        let rpc_provider_healthy_threshold = get_u8(
            vars,
            "PREDIFI_RPC_PROVIDER_HEALTHY_THRESHOLD",
            DEFAULT_RPC_PROVIDER_HEALTHY_THRESHOLD,
        )?;
        let rpc_disagreement_policy = get_string(
            vars,
            "PREDIFI_RPC_DISAGREEMENT_POLICY",
            DEFAULT_RPC_DISAGREEMENT_POLICY,
        );

        // ── Contract identity pinning (issue #347) ───────────────────────────
        let contract_manifest_path =
            get_string(vars, MANIFEST_PATH_ENV, DEFAULT_MANIFEST_PATH);
        let contract_id = get_string(vars, CONTRACT_ID_ENV, "");
        let network_passphrase = get_string(
            vars,
            NETWORK_PASSPHRASE_ENV,
            DEFAULT_NETWORK_PASSPHRASE,
        );
        let contract_interface_version = get_u32(
            vars,
            CONTRACT_INTERFACE_VERSION_ENV,
            DEFAULT_CONTRACT_INTERFACE_VERSION,
        )?;
        let contract_manifest_max_age_secs = get_u64(
            vars,
            MANIFEST_MAX_AGE_ENV,
            DEFAULT_MANIFEST_MAX_AGE_SECS,
        )?;

        // ── Database connection budget (issue #353) ──────────────────────────
        let db_server_max_connections = get_u32(
            vars,
            "PREDIFI_DB_SERVER_MAX_CONNECTIONS",
            DEFAULT_DB_SERVER_MAX_CONNECTIONS,
        )?;
        let db_admin_headroom_connections = get_u32(
            vars,
            "PREDIFI_DB_ADMIN_HEADROOM_CONNECTIONS",
            DEFAULT_DB_ADMIN_HEADROOM_CONNECTIONS,
        )?;
        let db_max_replicas = get_u32(
            vars,
            "PREDIFI_DB_MAX_REPLICAS",
            DEFAULT_DB_MAX_REPLICAS,
        )?;
        let db_pool_size_per_replica = get_u32(
            vars,
            "PREDIFI_DB_POOL_SIZE_PER_REPLICA",
            DEFAULT_DB_POOL_SIZE_PER_REPLICA,
        )?;
        let db_auxiliary_services = get_u32(
            vars,
            "PREDIFI_DB_AUXILIARY_SERVICES",
            DEFAULT_DB_AUXILIARY_SERVICES,
        )?;
        let db_auxiliary_pool_size = get_u32(
            vars,
            "PREDIFI_DB_AUXILIARY_POOL_SIZE",
            DEFAULT_DB_AUXILIARY_POOL_SIZE,
        )?;

        // ── Notification provider credentials (issue #355) ───────────────────
        let notification_provider_url = get_string(
            vars,
            "PREDIFI_NOTIFICATION_PROVIDER_URL",
            DEFAULT_NOTIFICATION_PROVIDER_URL,
        );
        let notification_provider_key =
            get_string(vars, "PREDIFI_NOTIFICATION_PROVIDER_KEY", "");
        let notification_provider_previous_key = vars
            .get("PREDIFI_NOTIFICATION_PROVIDER_PREVIOUS_KEY")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let notification_dual_key_enabled = get_bool(
            vars,
            "PREDIFI_NOTIFICATION_DUAL_KEY_ENABLED",
            DEFAULT_NOTIFICATION_DUAL_KEY_ENABLED,
        )?;

        if db_min_connections > db_max_connections {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_MIN_CONNECTIONS",
                reason: format!(
                    "must be <= PREDIFI_DB_MAX_CONNECTIONS ({}), got {}",
                    db_max_connections, db_min_connections
                ),
            });
        }

        let config = Self {
            host,
            port,
            database_url,
            db_max_connections,
            db_min_connections,
            db_acquire_timeout_secs,
            db_connect_max_attempts,
            db_connect_base_delay_ms,
            db_connect_max_delay_ms,
            db_connect_timeout_secs,
            rpc_health_timeout_secs,
            rpc_health_retry_count,
            rpc_timeout_secs,
            shutdown_timeout_secs,
            log_level,
            treasury_fee_bps,
            referral_fee_bps,
            stellar_rpc_url,
            sentry_dsn,
            redis_url,
            redis_required,
            pool_negative_cache_ttl_secs,
            cors_allowed_origins,
            secret_key,
            jwt_key_version,
            indexer_max_batch_size,
            app_env,
            allowed_ws_origins,
            abuse_quota_market_create_per_window,
            abuse_quota_wallet_action_per_window,
            abuse_quota_window_secs,
            abuse_quota_network_market_create_per_window,
            abuse_quota_network_wallet_action_per_window,
            abuse_min_validation_cost,
            abuse_quota_comment_per_window,
            abuse_quota_network_comment_per_window,
            abuse_quota_report_per_window,
            abuse_quota_network_report_per_window,
            rpc_per_network_budget,
            rpc_budget_window_secs,
            rpc_max_retries,
            rpc_priority_classes,
            rpc_provider_fallback_order,
            rpc_provider_unhealthy_threshold,
            rpc_provider_healthy_threshold,
            rpc_disagreement_policy,
            contract_manifest_path,
            contract_id,
            network_passphrase,
            contract_interface_version,
            contract_manifest_max_age_secs,
            notification_provider_url,
            notification_provider_key,
            notification_provider_previous_key,
            notification_dual_key_enabled,
            db_server_max_connections,
            db_admin_headroom_connections,
            db_max_replicas,
            db_pool_size_per_replica,
            db_auxiliary_services,
            db_auxiliary_pool_size,
        };

        config.validate()?;
        Ok(config)
    }

    /// Build the contract identity configuration from this config.
    pub fn contract_identity(&self) -> ContractIdentityConfig {
        ContractIdentityConfig {
            manifest_path: self.contract_manifest_path.clone(),
            contract_id: self.contract_id.clone(),
            network_passphrase: self.network_passphrase.clone(),
            interface_version: self.contract_interface_version,
            manifest_max_age_secs: self.contract_manifest_max_age_secs,
        }
    }

    /// Verify the configured contract identity against the pinned manifest.
    ///
    /// Called at startup and on network change. The result gates mutation
    /// routes: anything other than [`ContractIdentityStatus::Verified`]
    /// disables mutations. There is no fallback to an unverified contract.
    pub fn verify_contract_identity(&self, now_unix_secs: u64) -> ContractIdentityStatus {
        self.contract_identity().verify(now_unix_secs)
    }

    /// Validate semantic constraints on a fully-built configuration.
    ///
    /// Called automatically by [`Self::from_map`] and again at application
    /// startup via [`crate::run_server`] so misconfiguration fails fast before
    /// any network listeners or background workers are started.
    pub fn validate(&self) -> Result<(), ConfigError> {
        // ── Host ─────────────────────────────────────────────────────────────
        if self.host.is_empty() {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_APP_HOST",
                reason: String::from("must not be empty"),
            });
        }

        // ── Database URL ─────────────────────────────────────────────────────
        validate_url_scheme(
            &self.database_url,
            "PREDIFI_DATABASE_URL",
            &["postgres://", "postgresql://"],
        )?;
        validate_url_has_host(&self.database_url, "PREDIFI_DATABASE_URL")?;

        // ── Redis URL ────────────────────────────────────────────────────────
        validate_url_scheme(
            &self.redis_url,
            "PREDIFI_REDIS_URL",
            &["redis://", "rediss://"],
        )?;
        validate_url_has_host(&self.redis_url, "PREDIFI_REDIS_URL")?;

        // ── Stellar RPC URL ──────────────────────────────────────────────────
        validate_url_scheme(
            &self.stellar_rpc_url,
            "PREDIFI_STELLAR_RPC_URL",
            &["http://", "https://"],
        )?;
        validate_url_has_host(&self.stellar_rpc_url, "PREDIFI_STELLAR_RPC_URL")?;

        // ── Database pool ────────────────────────────────────────────────────
        if self.db_max_connections == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_MAX_CONNECTIONS",
                reason: String::from("must be greater than zero"),
            });
        }
        if self.db_min_connections == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_MIN_CONNECTIONS",
                reason: String::from("must be greater than zero"),
            });
        }
        if self.db_acquire_timeout_secs == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_ACQUIRE_TIMEOUT_SECS",
                reason: String::from("must be greater than zero"),
            });
        }

        // ── Connection retry settings ────────────────────────────────────────
        if self.db_connect_max_attempts == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_CONNECT_MAX_ATTEMPTS",
                reason: String::from("must be at least one"),
            });
        }
        if self.db_connect_timeout_secs == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_CONNECT_TIMEOUT_SECS",
                reason: String::from("must be greater than zero"),
            });
        }
        // base delay must not exceed max delay (when max_attempts > 1 the
        // first retry fires after base_delay; if base > max the cap would
        // never apply, which is a misconfiguration).
        if self.db_connect_max_attempts > 1
            && self.db_connect_base_delay_ms > self.db_connect_max_delay_ms
        {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_CONNECT_BASE_DELAY_MS",
                reason: format!(
                    "must be <= PREDIFI_DB_CONNECT_MAX_DELAY_MS ({}), got {}",
                    self.db_connect_max_delay_ms, self.db_connect_base_delay_ms
                ),
            });
        }

        // ── RPC timeouts ─────────────────────────────────────────────────────
        if self.rpc_health_timeout_secs == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_RPC_HEALTH_TIMEOUT_SECS",
                reason: String::from("must be greater than zero"),
            });
        }
        if self.rpc_health_retry_count == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_RPC_HEALTH_RETRY_COUNT",
                reason: String::from("must be at least one"),
            });
        }
        if self.rpc_timeout_secs == 0 {
            return Err(ConfigError::InvalidValue {
                key: "RPC_TIMEOUT_SECS",
                reason: String::from("must be greater than zero"),
            });
        }
        // The health-check per-attempt timeout must be within the general RPC
        // budget; otherwise the health check would never complete before the
        // outer timeout fires.
        if self.rpc_health_timeout_secs > self.rpc_timeout_secs {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_RPC_HEALTH_TIMEOUT_SECS",
                reason: format!(
                    "must be <= RPC_TIMEOUT_SECS ({}), got {}",
                    self.rpc_timeout_secs, self.rpc_health_timeout_secs
                ),
            });
        }

        // ── Fee basis points ─────────────────────────────────────────────────
        if self.treasury_fee_bps > 10_000 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_TREASURY_FEE_BPS",
                reason: String::from("must be <= 10000 (100%)"),
            });
        }
        if self.referral_fee_bps > 10_000 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_REFERRAL_FEE_BPS",
                reason: String::from("must be <= 10000 (100%)"),
            });
        }

        // ── CORS ─────────────────────────────────────────────────────────────
        if self.cors_allowed_origins.is_empty() {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                reason: String::from("must contain at least one origin"),
            });
        }

        for origin in &self.cors_allowed_origins {
            validate_cors_origin(origin)?;
        }

        if self.is_production() {
            let default_origins: Vec<String> =
                DEFAULT_CORS_ORIGINS.iter().map(|s| s.to_string()).collect();
            if self.cors_allowed_origins == default_origins {
                return Err(ConfigError::InvalidValue {
                    key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                    reason: String::from(
                        "the default development CORS origins must not be used in production; \
                         set PREDIFI_CORS_ALLOWED_ORIGINS to an explicit list of trusted HTTPS origins",
                    ),
                });
            }

            for origin in &self.cors_allowed_origins {
                if !origin.starts_with("https://") {
                    return Err(ConfigError::InvalidValue {
                        key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                        reason: format!("production CORS origin '{}' must use https://", origin),
                    });
                }
                if origin.contains("localhost") || origin.contains("127.0.0.1") {
                    return Err(ConfigError::InvalidValue {
                        key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                        reason: format!(
                            "production CORS origin '{}' cannot be localhost or loopback",
                            origin
                        ),
                    });
                }
            }
        }

        // ── JWT secret ───────────────────────────────────────────────────────
        if let Err(reason) = crate::jwt::verify_jwt_secret(&self.secret_key) {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_SECRET_KEY",
                reason: reason.to_string(),
            });
        }
        // Reject the compiled-in dev placeholder in production.
        if self.is_production() && self.secret_key == DEFAULT_SECRET_KEY {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_SECRET_KEY",
                reason: String::from(
                    "the default development secret must not be used in production; \
                     set PREDIFI_SECRET_KEY to a strong, randomly generated value",
                ),
            });
        }

        // ── WebSocket origins ────────────────────────────────────────────────
        // Fail closed in production: an empty allow-list would accept any Origin
        // and enable CSRF-style WebSocket hijacking.
        if self.is_production() && self.allowed_ws_origins.is_empty() {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_WS_ALLOWED_ORIGINS",
                reason: String::from(
                    "must contain at least one origin in production to prevent WebSocket hijacking",
                ),
            });
        }

        // ── Indexer ──────────────────────────────────────────────────────────
        if self.indexer_max_batch_size == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_INDEXER_MAX_BATCH_SIZE",
                reason: String::from("must be greater than zero"),
            });
        }

        // ── Abuse controls (issue #103) ──────────────────────────────────────
        if self.abuse_quota_window_secs == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_WINDOW_SECS",
                reason: String::from("must be greater than zero"),
            });
        }
        if self.abuse_quota_market_create_per_window == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_MARKET_CREATE_PER_WINDOW",
                reason: String::from("must be greater than zero"),
            });
        }
        if self.abuse_quota_wallet_action_per_window == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_WALLET_ACTION_PER_WINDOW",
                reason: String::from("must be greater than zero"),
            });
        }
        if self.abuse_quota_network_market_create_per_window == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_NETWORK_MARKET_CREATE_PER_WINDOW",
                reason: String::from("must be greater than zero"),
            });
        }
        if self.abuse_quota_network_wallet_action_per_window == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_NETWORK_WALLET_ACTION_PER_WINDOW",
                reason: String::from("must be greater than zero"),
            });
        }
        // Per-wallet quotas must fit inside the network-wide ceiling, otherwise
        // a single wallet could exhaust the whole network budget.
        if self.abuse_quota_market_create_per_window
            > self.abuse_quota_network_market_create_per_window
        {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_MARKET_CREATE_PER_WINDOW",
                reason: format!(
                    "must be <= PREDIFI_ABUSE_QUOTA_NETWORK_MARKET_CREATE_PER_WINDOW ({}), got {}",
                    self.abuse_quota_network_market_create_per_window,
                    self.abuse_quota_market_create_per_window
                ),
            });
        }
        if self.abuse_quota_wallet_action_per_window
            > self.abuse_quota_network_wallet_action_per_window
        {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_WALLET_ACTION_PER_WINDOW",
                reason: format!(
                    "must be <= PREDIFI_ABUSE_QUOTA_NETWORK_WALLET_ACTION_PER_WINDOW ({}), got {}",
                    self.abuse_quota_network_wallet_action_per_window,
                    self.abuse_quota_wallet_action_per_window
                ),
            });
        }

        // ── Comment and report quotas ─────────────────────────────────────────
        if self.abuse_quota_comment_per_window == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_COMMENT_PER_WINDOW",
                reason: String::from("must be greater than zero"),
            });
        }
        if self.abuse_quota_network_comment_per_window == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_NETWORK_COMMENT_PER_WINDOW",
                reason: String::from("must be greater than zero"),
            });
        }
        if self.abuse_quota_comment_per_window > self.abuse_quota_network_comment_per_window {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_COMMENT_PER_WINDOW",
                reason: format!(
                    "must be <= PREDIFI_ABUSE_QUOTA_NETWORK_COMMENT_PER_WINDOW ({}), got {}",
                    self.abuse_quota_network_comment_per_window,
                    self.abuse_quota_comment_per_window
                ),
            });
        }
        if self.abuse_quota_report_per_window == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_REPORT_PER_WINDOW",
                reason: String::from("must be greater than zero"),
            });
        }
        if self.abuse_quota_network_report_per_window == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_NETWORK_REPORT_PER_WINDOW",
                reason: String::from("must be greater than zero"),
            });
        }
        if self.abuse_quota_report_per_window > self.abuse_quota_network_report_per_window {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_REPORT_PER_WINDOW",
                reason: format!(
                    "must be <= PREDIFI_ABUSE_QUOTA_NETWORK_REPORT_PER_WINDOW ({}), got {}",
                    self.abuse_quota_network_report_per_window,
                    self.abuse_quota_report_per_window
                ),
            });
        }

        // ── RPC quotas / failover (issue #105) ───────────────────────────────
        if self.rpc_per_network_budget == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_RPC_PER_NETWORK_BUDGET",
                reason: String::from("must be greater than zero"),
            });
        }
        if self.rpc_budget_window_secs == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_RPC_BUDGET_WINDOW_SECS",
                reason: String::from("must be greater than zero"),
            });
        }
        if self.rpc_priority_classes.is_empty() {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_RPC_PRIORITY_CLASSES",
                reason: String::from("must contain at least one priority class"),
            });
        }
        for class in &self.rpc_priority_classes {
            if class.is_empty() {
                return Err(ConfigError::InvalidValue {
                    key: "PREDIFI_RPC_PRIORITY_CLASSES",
                    reason: String::from("priority classes must not be empty"),
                });
            }
        }
        if self.rpc_provider_unhealthy_threshold == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_RPC_PROVIDER_UNHEALTHY_THRESHOLD",
                reason: String::from("must be at least one"),
            });
        }
        if self.rpc_provider_healthy_threshold == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_RPC_PROVIDER_HEALTHY_THRESHOLD",
                reason: String::from("must be at least one"),
            });
        }
        if !matches!(
            self.rpc_disagreement_policy.as_str(),
            "quorum" | "first" | "strict"
        ) {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_RPC_DISAGREEMENT_POLICY",
                reason: format!(
                    "must be one of: quorum, first, strict; got '{}'",
                    self.rpc_disagreement_policy
                ),
            });
        }

        // ── Contract identity pinning (issue #347) ───────────────────────────
        if self.contract_manifest_path.trim().is_empty() {
            return Err(ConfigError::InvalidValue {
                key: MANIFEST_PATH_ENV,
                reason: String::from("must not be empty"),
            });
        }
        if self.contract_interface_version != SUPPORTED_CONTRACT_INTERFACE_VERSION {
            return Err(ConfigError::InvalidValue {
                key: CONTRACT_INTERFACE_VERSION_ENV,
                reason: format!(
                    "this build only supports interface version {}; got {}",
                    SUPPORTED_CONTRACT_INTERFACE_VERSION, self.contract_interface_version
                ),
            });
        }
        // In production the contract ID must be pinned explicitly: an empty ID
        // can never match a manifest and would silently disable mutations.
        if self.is_production() && self.contract_id.trim().is_empty() {
            return Err(ConfigError::InvalidValue {
                key: CONTRACT_ID_ENV,
                reason: String::from(
                    "must be set in production so the contract identity can be verified",
                ),
            });
        }
        // ── Database connection budget (issue #353) ──────────────────────────
        if self.db_server_max_connections == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_SERVER_MAX_CONNECTIONS",
                reason: String::from("must be greater than zero"),
            });
        }
        if self.db_max_replicas == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_MAX_REPLICAS",
                reason: String::from("must be at least one"),
            });
        }
        if self.db_pool_size_per_replica == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_POOL_SIZE_PER_REPLICA",
                reason: String::from("must be greater than zero"),
            });
        }
        if self.db_auxiliary_services > 0 && self.db_auxiliary_pool_size == 0 {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_AUXILIARY_POOL_SIZE",
                reason: String::from(
                    "must be greater than zero when PREDIFI_DB_AUXILIARY_SERVICES is set",
                ),
            });
        }
        // The admin headroom must leave at least one connection for the
        // services themselves; otherwise the budget is unusable.
        if self.db_admin_headroom_connections >= self.db_server_max_connections {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_ADMIN_HEADROOM_CONNECTIONS",
                reason: format!(
                    "must be < PREDIFI_DB_SERVER_MAX_CONNECTIONS ({}), got {}",
                    self.db_server_max_connections, self.db_admin_headroom_connections
                ),
            });
        }

        // ── Notification provider credentials (issue #355) ───────────────────
        let provider_url = self.notification_provider_url.trim();
        if !provider_url.is_empty() {
            validate_url_scheme(
                provider_url,
                "PREDIFI_NOTIFICATION_PROVIDER_URL",
                &["http://", "https://"],
            )?;
            validate_url_has_host(provider_url, "PREDIFI_NOTIFICATION_PROVIDER_URL")?;
        }

        // A configured provider must always have a live credential. An empty
        // key means the credential was revoked (or never provisioned) and must
        // fail fast rather than silently dropping notifications.
        if !provider_url.is_empty() && self.notification_provider_key.trim().is_empty() {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_NOTIFICATION_PROVIDER_KEY",
                reason: String::from(
                    "must be set to a non-empty credential when \
                     PREDIFI_NOTIFICATION_PROVIDER_URL is configured; \
                     a revoked or empty key is rejected",
                ),
            });
        }

        // Dual-key transition is opt-in only. A previous key without explicit
        // opt-in is a misconfiguration (the provider may not support it).
        if self.notification_provider_previous_key.is_some() && !self.notification_dual_key_enabled {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_NOTIFICATION_PROVIDER_PREVIOUS_KEY",
                reason: String::from(
                    "a previous credential requires PREDIFI_NOTIFICATION_DUAL_KEY_ENABLED=true; \
                     dual-key transition must be explicitly allowed",
                ),
            });
        }

        // The previous key must differ from the current key — otherwise the
        // rotation has not actually happened.
        if let Some(previous) = &self.notification_provider_previous_key {
            if previous.trim() == self.notification_provider_key.trim() {
                return Err(ConfigError::InvalidValue {
                    key: "PREDIFI_NOTIFICATION_PROVIDER_PREVIOUS_KEY",
                    reason: String::from(
                        "must differ from PREDIFI_NOTIFICATION_PROVIDER_KEY during a rotation",
                    ),
                });
            }
        }

        let budget = self.db_connection_budget();
        if budget.required_connections > budget.available_connections {
            return Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_MAX_REPLICAS",
                reason: format!(
                    "unsafe database connection budget: {} replicas x {} pool + {} auxiliary x {} \
                     = {} connections required, but only {} available after reserving {} of {} \
                     for administration; reduce PREDIFI_DB_POOL_SIZE_PER_REPLICA, \
                     PREDIFI_DB_MAX_REPLICAS or PREDIFI_DB_AUXILIARY_SERVICES, or raise \
                     PREDIFI_DB_SERVER_MAX_CONNECTIONS",
                    self.db_max_replicas,
                    self.db_pool_size_per_replica,
                    self.db_auxiliary_services,
                    self.db_auxiliary_pool_size,
                    budget.required_connections,
                    budget.available_connections,
                    self.db_admin_headroom_connections,
                    self.db_server_max_connections
                ),
            });
        }

        Ok(())
    }

    /// Compute the global database connection budget (issue #353).
    ///
    /// The budget accounts for every pool that may connect to PostgreSQL at
    /// peak replica count: the HTTP replicas plus any auxiliary services
    /// (indexer, workers) that hold their own pool. Administration headroom is
    /// subtracted from the server limit before comparing.
    pub fn db_connection_budget(&self) -> DbConnectionBudget {
        let replica_connections = self
            .db_max_replicas
            .saturating_mul(self.db_pool_size_per_replica);
        let auxiliary_connections = self
            .db_auxiliary_services
            .saturating_mul(self.db_auxiliary_pool_size);
        let required_connections = replica_connections.saturating_add(auxiliary_connections);
        let available_connections = self
            .db_server_max_connections
            .saturating_sub(self.db_admin_headroom_connections);

        DbConnectionBudget {
            server_max_connections: self.db_server_max_connections,
            admin_headroom_connections: self.db_admin_headroom_connections,
            available_connections,
            replica_connections,
            auxiliary_connections,
            required_connections,
        }
    }

    /// Return the `host:port` string used to bind the TCP listener.
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// `true` when `PREDIFI_APP_ENV` is `production` (case-insensitive).
    pub fn is_production(&self) -> bool {
        self.app_env.eq_ignore_ascii_case(PRODUCTION_ENV_VALUE)
    }

    /// Build a minimal [`Config`] suitable for unit and integration tests.
    ///
    /// Uses `127.0.0.1:0` (OS-assigned port), an in-memory-style Postgres URL,
    /// and the compiled-in fee defaults so tests do not depend on environment
    /// variables.
    pub fn default_for_test() -> Self {
        Self {
            host: String::from("127.0.0.1"),
            port: 0,
            database_url: String::from("postgres://localhost/test"),
            db_max_connections: 1,
            db_min_connections: 1,
            db_acquire_timeout_secs: 1,
            db_connect_max_attempts: 1,
            db_connect_base_delay_ms: 0,
            db_connect_max_delay_ms: 0,
            db_connect_timeout_secs: 5,
            rpc_health_timeout_secs: 2,
            rpc_health_retry_count: 3,
            rpc_timeout_secs: 30,
            shutdown_timeout_secs: 5,
            log_level: String::from("debug"),
            treasury_fee_bps: DEFAULT_TREASURY_FEE_BPS,
            referral_fee_bps: DEFAULT_REFERRAL_FEE_BPS,
            stellar_rpc_url: String::from(DEFAULT_STELLAR_RPC_URL),
            sentry_dsn: None,
            redis_url: String::from(DEFAULT_REDIS_URL),
            redis_required: DEFAULT_REDIS_REQUIRED,
            pool_negative_cache_ttl_secs: DEFAULT_POOL_NEGATIVE_CACHE_TTL_SECS,
            cors_allowed_origins: DEFAULT_CORS_ORIGINS.iter().map(|s| s.to_string()).collect(),
            secret_key: String::from(DEFAULT_SECRET_KEY),
            jwt_key_version: 0,
            indexer_max_batch_size: DEFAULT_INDEXER_MAX_BATCH_SIZE,
            app_env: String::from(DEFAULT_APP_ENV),
            allowed_ws_origins: Vec::new(), // Empty for permissive mode in tests
            abuse_quota_market_create_per_window: DEFAULT_ABUSE_QUOTA_MARKET_CREATE_PER_WINDOW,
            abuse_quota_wallet_action_per_window: DEFAULT_ABUSE_QUOTA_WALLET_ACTION_PER_WINDOW,
            abuse_quota_window_secs: DEFAULT_ABUSE_QUOTA_WINDOW_SECS,
            abuse_quota_network_market_create_per_window:
                DEFAULT_ABUSE_QUOTA_NETWORK_MARKET_CREATE_PER_WINDOW,
            abuse_quota_network_wallet_action_per_window:
                DEFAULT_ABUSE_QUOTA_NETWORK_WALLET_ACTION_PER_WINDOW,
            abuse_min_validation_cost: DEFAULT_ABUSE_MIN_VALIDATION_COST,
            abuse_quota_comment_per_window: DEFAULT_ABUSE_QUOTA_COMMENT_PER_WINDOW,
            abuse_quota_network_comment_per_window: DEFAULT_ABUSE_QUOTA_NETWORK_COMMENT_PER_WINDOW,
            abuse_quota_report_per_window: DEFAULT_ABUSE_QUOTA_REPORT_PER_WINDOW,
            abuse_quota_network_report_per_window: DEFAULT_ABUSE_QUOTA_NETWORK_REPORT_PER_WINDOW,
            rpc_per_network_budget: DEFAULT_RPC_PER_NETWORK_BUDGET,
            rpc_budget_window_secs: DEFAULT_RPC_BUDGET_WINDOW_SECS,
            rpc_max_retries: DEFAULT_RPC_MAX_RETRIES,
            rpc_priority_classes: DEFAULT_RPC_PRIORITY_CLASSES
                .split(',')
                .map(|s| s.to_string())
                .collect(),
            rpc_provider_fallback_order: Vec::new(),
            rpc_provider_unhealthy_threshold: DEFAULT_RPC_PROVIDER_UNHEALTHY_THRESHOLD,
            rpc_provider_healthy_threshold: DEFAULT_RPC_PROVIDER_HEALTHY_THRESHOLD,
            rpc_disagreement_policy: String::from(DEFAULT_RPC_DISAGREEMENT_POLICY),
            contract_manifest_path: String::from(DEFAULT_MANIFEST_PATH),
            contract_id: String::new(),
            network_passphrase: String::from(DEFAULT_NETWORK_PASSPHRASE),
            contract_interface_version: DEFAULT_CONTRACT_INTERFACE_VERSION,
            contract_manifest_max_age_secs: DEFAULT_MANIFEST_MAX_AGE_SECS,
            notification_provider_url: String::from(DEFAULT_NOTIFICATION_PROVIDER_URL),
            notification_provider_key: String::new(),
            notification_provider_previous_key: None,
            notification_dual_key_enabled: DEFAULT_NOTIFICATION_DUAL_KEY_ENABLED,
            db_server_max_connections: DEFAULT_DB_SERVER_MAX_CONNECTIONS,
            db_admin_headroom_connections: DEFAULT_DB_ADMIN_HEADROOM_CONNECTIONS,
            db_max_replicas: DEFAULT_DB_MAX_REPLICAS,
            db_pool_size_per_replica: DEFAULT_DB_POOL_SIZE_PER_REPLICA,
            db_auxiliary_services: DEFAULT_DB_AUXILIARY_SERVICES,
            db_auxiliary_pool_size: DEFAULT_DB_AUXILIARY_POOL_SIZE,
        }
    }
}

/// Snapshot of the global database connection budget (issue #353).
///
/// Produced by [`Config::db_connection_budget`] and used both by startup
/// validation and by deployment checks to verify that the calculated maximum
/// connection count stays within the PostgreSQL server limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DbConnectionBudget {
    /// PostgreSQL server-side connection limit (`max_connections`).
    pub server_max_connections: u32,
    /// Connections reserved for administration and excluded from the budget.
    pub admin_headroom_connections: u32,
    /// Connections available to services after subtracting admin headroom.
    pub available_connections: u32,
    /// Connections consumed by all HTTP replicas at peak replica count.
    pub replica_connections: u32,
    /// Connections consumed by auxiliary services at peak replica count.
    pub auxiliary_connections: u32,
    /// Total connections required at peak replica count.
    pub required_connections: u32,
}

impl DbConnectionBudget {
    /// `true` when the calculated maximum fits within the available budget.
    pub fn is_within_limit(&self) -> bool {
        self.required_connections <= self.available_connections
    }
}

/// Error returned when a configuration value cannot be parsed or is logically invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// An environment variable was set but its value could not be parsed as the
    /// expected numeric type.
    InvalidNumber {
        /// Name of the environment variable.
        key: &'static str,
        /// The raw string value that failed to parse.
        value: String,
        /// Human-readable parse error from the standard library.
        reason: String,
    },
    /// An environment variable was set to a syntactically valid string but the
    /// value violates a semantic constraint (e.g. min > max, invalid URL).
    InvalidValue {
        /// Name of the environment variable.
        key: &'static str,
        /// Human-readable description of the constraint that was violated.
        reason: String,
    },
    /// A required environment variable is absent and has no compiled-in default.
    MissingRequired {
        /// Name of the missing environment variable.
        key: &'static str,
        /// Human-readable description of what the variable is used for.
        description: &'static str,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNumber { key, value, reason } => {
                write!(f, "invalid value for {}='{}': {}", key, value, reason)
            }
            Self::InvalidValue { key, reason } => {
                write!(f, "invalid value for {}: {}", key, reason)
            }
            Self::MissingRequired { key, description } => {
                write!(
                    f,
                    "required environment variable '{}' is not set ({})",
                    key, description
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// Parse and strictly validate the `PREDIFI_CORS_ALLOWED_ORIGINS` environment variable.
///
/// The value must be a comma-separated list of origins.  Each origin must:
/// - Use the `http` or `https` scheme.
/// - Contain a non-empty host.
/// - Have **no** path component other than an optional trailing slash on the root.
/// - Have **no** query string or fragment.
///
/// Returns the default origins when the variable is absent.
fn parse_cors_origins(vars: &HashMap<String, String>) -> Result<Vec<String>, ConfigError> {
    let raw = match vars.get("PREDIFI_CORS_ALLOWED_ORIGINS") {
        Some(v) => v.clone(),
        None => {
            return Ok(DEFAULT_CORS_ORIGINS.iter().map(|s| s.to_string()).collect());
        }
    };

    let origins: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if origins.is_empty() {
        return Err(ConfigError::InvalidValue {
            key: "PREDIFI_CORS_ALLOWED_ORIGINS",
            reason: String::from("must contain at least one origin"),
        });
    }

    for origin in &origins {
        validate_cors_origin(origin)?;
    }

    Ok(origins)
}

/// Parse and validate the `PREDIFI_WS_ALLOWED_ORIGINS` environment variable.
///
/// The value must be a comma-separated list of origins using the same validation
/// rules as CORS origins. If the variable is absent, returns an empty vector
/// (permissive mode - allows any origin). This is intentional for WebSocket
/// origin validation to be optional in development while enforceable in production.
fn parse_ws_origins(vars: &HashMap<String, String>) -> Result<Vec<String>, ConfigError> {
    let raw = match vars.get("PREDIFI_WS_ALLOWED_ORIGINS") {
        Some(v) => v.clone(),
        None => {
            // Permissive mode when not configured
            return Ok(Vec::new());
        }
    };

    let origins: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Empty string means explicitly disable restrictions (same as absent)
    if origins.is_empty() {
        return Ok(Vec::new());
    }

    for origin in &origins {
        validate_cors_origin(origin)?;
    }

    Ok(origins)
}

/// Validate a single CORS origin string.
///
/// A valid origin is `scheme://host` or `scheme://host:port` where:
/// - `scheme` is `http` or `https`
/// - `host` is non-empty
/// - There is no path (other than an empty path or bare `/`), no query, and no fragment.
pub fn validate_cors_origin(origin: &str) -> Result<(), ConfigError> {
    // Reject wildcards and the special `null` origin outright.
    if origin == "*" || origin.contains('*') || origin.eq_ignore_ascii_case("null") {
        return Err(ConfigError::InvalidValue {
            key: "PREDIFI_CORS_ALLOWED_ORIGINS",
            reason: format!(
                "'{}' is not a valid origin — wildcards and 'null' are not permitted",
                origin
            ),
        });
    }

    // Split off the scheme.
    let rest = if let Some(r) = origin.strip_prefix("https://") {
        r
    } else if let Some(r) = origin.strip_prefix("http://") {
        r
    } else {
        return Err(ConfigError::InvalidValue {
            key: "PREDIFI_CORS_ALLOWED_ORIGINS",
            reason: format!(
                "'{}' is not a valid origin — must start with 'http://' or 'https://'",
                origin
            ),
        });
    };

    // Reject fragments and query strings.
    if rest.contains('#') || rest.contains('?') {
        return Err(ConfigError::InvalidValue {
            key: "PREDIFI_CORS_ALLOWED_ORIGINS",
            reason: format!(
                "'{}' is not a valid origin — must not contain a query string or fragment",
                origin
            ),
        });
    }

    // Split host[:port] from any path component.
    let (authority, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    };

    // A path other than "" or "/" is not allowed in an origin.
    if !path.is_empty() && path != "/" {
        return Err(ConfigError::InvalidValue {
            key: "PREDIFI_CORS_ALLOWED_ORIGINS",
            reason: format!(
                "'{}' is not a valid origin — must not contain a path component",
                origin
            ),
        });
    }

    // The host part must be non-empty.
    let host = match authority.rfind(':') {
        // Possible port — strip it and validate.
        Some(colon_idx) => {
            let port_str = &authority[colon_idx + 1..];
            // Only treat it as a port if it's all digits; otherwise it's part of an IPv6 address.
            if port_str.chars().all(|c| c.is_ascii_digit()) {
                let port: u32 = port_str.parse().unwrap_or(u32::MAX);
                if port > 65535 {
                    return Err(ConfigError::InvalidValue {
                        key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                        reason: format!(
                            "'{}' is not a valid origin — port {} is out of range (0-65535)",
                            origin, port_str
                        ),
                    });
                }
                &authority[..colon_idx]
            } else {
                authority
            }
        }
        None => authority,
    };

    if host.is_empty() {
        return Err(ConfigError::InvalidValue {
            key: "PREDIFI_CORS_ALLOWED_ORIGINS",
            reason: format!(
                "'{}' is not a valid origin — host must not be empty",
                origin
            ),
        });
    }

    Ok(())
}

fn get_string(vars: &HashMap<String, String>, key: &'static str, default: &str) -> String {
    vars.get(key)
        .map_or_else(|| default.to_string(), |value| value.clone())
}

/// Parse a comma-separated list from `key`, trimming whitespace and dropping
/// empty entries. Falls back to splitting `default` when the variable is
/// absent. An explicitly empty value yields an empty vector.
fn parse_csv_list(
    vars: &HashMap<String, String>,
    key: &'static str,
    default: &str,
) -> Vec<String> {
    let raw = vars.get(key).map_or(default, String::as_str);
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn get_u16(
    vars: &HashMap<String, String>,
    key: &'static str,
    default: u16,
) -> Result<u16, ConfigError> {
    match vars.get(key) {
        Some(value) => value
            .parse::<u16>()
            .map_err(|err| to_number_error(key, value, err)),
        None => Ok(default),
    }
}

fn get_u32(
    vars: &HashMap<String, String>,
    key: &'static str,
    default: u32,
) -> Result<u32, ConfigError> {
    match vars.get(key) {
        Some(value) => value
            .parse::<u32>()
            .map_err(|err| to_number_error(key, value, err)),
        None => Ok(default),
    }
}

fn get_u8(
    vars: &HashMap<String, String>,
    key: &'static str,
    default: u8,
) -> Result<u8, ConfigError> {
    match vars.get(key) {
        Some(value) => value
            .parse::<u8>()
            .map_err(|err| to_number_error(key, value, err)),
        None => Ok(default),
    }
}

fn get_u64(
    vars: &HashMap<String, String>,
    key: &'static str,
    default: u64,
) -> Result<u64, ConfigError> {
    match vars.get(key) {
        Some(value) => value
            .parse::<u64>()
            .map_err(|err| to_number_error(key, value, err)),
        None => Ok(default),
    }
}

fn get_bool(
    vars: &HashMap<String, String>,
    key: &'static str,
    default: bool,
) -> Result<bool, ConfigError> {
    match vars.get(key) {
        Some(value) => match value.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            _ => Err(ConfigError::InvalidValue {
                key,
                reason: format!("must be a boolean (true/false), got '{value}'"),
            }),
        },
        None => Ok(default),
    }
}

fn get_usize(
    vars: &HashMap<String, String>,
    key: &'static str,
    default: usize,
) -> Result<usize, ConfigError> {
    match vars.get(key) {
        Some(value) => value
            .parse::<usize>()
            .map_err(|err| to_number_error(key, value, err)),
        None => Ok(default),
    }
}

fn to_number_error(key: &'static str, value: &str, err: ParseIntError) -> ConfigError {
    ConfigError::InvalidNumber {
        key,
        value: value.to_string(),
        reason: err.to_string(),
    }
}

/// Ensure `url` begins with one of the allowed schemes.
fn validate_url_scheme(url: &str, key: &'static str, schemes: &[&str]) -> Result<(), ConfigError> {
    if schemes.iter().any(|scheme| url.starts_with(scheme)) {
        Ok(())
    } else {
        Err(ConfigError::InvalidValue {
            key,
            reason: format!("must start with one of: {}", schemes.join(", ")),
        })
    }
}

/// Ensure `url` has a non-empty host after the scheme.
///
/// This catches malformed URLs like `postgres:///dbname` or `redis:///` that
/// pass the scheme check but have no host, which would cause a confusing
/// runtime error at connection time.
fn validate_url_has_host(url: &str, key: &'static str) -> Result<(), ConfigError> {
    // Strip the scheme (e.g. "postgres://", "redis://", "https://").
    let after_scheme = match url.find("://") {
        Some(idx) => &url[idx + 3..],
        None => {
            // validate_url_scheme already caught this, but be defensive.
            return Err(ConfigError::InvalidValue {
                key,
                reason: String::from("missing '://' separator"),
            });
        }
    };

    // The authority ends at the first '/', '?', or '#'.
    let authority = match after_scheme.find(['/', '?', '#']) {
        Some(idx) => &after_scheme[..idx],
        None => after_scheme,
    };

    // Strip optional userinfo (user:pass@).
    let host_and_port = match authority.rfind('@') {
        Some(at) => &authority[at + 1..],
        None => authority,
    };

    // Strip optional port.
    let host = match host_and_port.rfind(':') {
        Some(colon) => {
            let port_str = &host_and_port[colon + 1..];
            if port_str.chars().all(|c| c.is_ascii_digit()) {
                &host_and_port[..colon]
            } else {
                host_and_port
            }
        }
        None => host_and_port,
    };

    if host.is_empty() {
        return Err(ConfigError::InvalidValue {
            key,
            reason: format!("URL '{}' has no host — check for a missing hostname", url),
        });
    }

    Ok(())
}

/// Return the value of `key` from `vars`, or `Err(ConfigError::MissingRequired)` if absent.
///
/// Use this for environment variables that **must** be explicitly set in
/// production deployments (i.e. variables with no safe compiled-in default).
#[allow(dead_code)]
fn get_required_string(
    vars: &HashMap<String, String>,
    key: &'static str,
    description: &'static str,
) -> Result<String, ConfigError> {
    vars.get(key)
        .cloned()
        .ok_or(ConfigError::MissingRequired { key, description })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_uses_defaults_when_env_is_missing() {
        let vars = HashMap::new();
        let config = Config::from_map(&vars).expect("defaults should build a valid config");

        assert_eq!(config.host, DEFAULT_HOST);
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.database_url, DEFAULT_DATABASE_URL);
        assert_eq!(config.db_max_connections, DEFAULT_DB_MAX_CONNECTIONS);
        assert_eq!(config.db_min_connections, DEFAULT_DB_MIN_CONNECTIONS);
        assert_eq!(
            config.db_acquire_timeout_secs,
            DEFAULT_DB_ACQUIRE_TIMEOUT_SECS
        );
        assert_eq!(config.log_level, DEFAULT_LOG_LEVEL);
        assert_eq!(config.treasury_fee_bps, DEFAULT_TREASURY_FEE_BPS);
        assert_eq!(config.referral_fee_bps, DEFAULT_REFERRAL_FEE_BPS);
        assert_eq!(config.redis_url, DEFAULT_REDIS_URL);
        assert_eq!(config.rpc_timeout_secs, DEFAULT_RPC_TIMEOUT_SECS);
    }

    #[test]
    fn config_rejects_non_numeric_port() {
        let vars = HashMap::from([(
            String::from("PREDIFI_APP_PORT"),
            String::from("not-a-number"),
        )]);
        let error = Config::from_map(&vars).expect_err("port must be numeric");

        assert!(
            matches!(
                error,
                ConfigError::InvalidNumber {
                    key: "PREDIFI_APP_PORT",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn config_rejects_min_connections_larger_than_max() {
        let vars = HashMap::from([
            (
                String::from("PREDIFI_DB_MIN_CONNECTIONS"),
                String::from("20"),
            ),
            (
                String::from("PREDIFI_DB_MAX_CONNECTIONS"),
                String::from("10"),
            ),
        ]);
        let error = Config::from_map(&vars).expect_err("min > max must be rejected");

        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "PREDIFI_DB_MIN_CONNECTIONS",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    // ── CORS origin validation ────────────────────────────────────────────────

    #[test]
    fn cors_defaults_when_env_absent() {
        let vars = HashMap::new();
        let config = Config::from_map(&vars).expect("defaults should be valid");
        let expected: Vec<String> = DEFAULT_CORS_ORIGINS.iter().map(|s| s.to_string()).collect();
        assert_eq!(config.cors_allowed_origins, expected);
    }

    #[test]
    fn cors_accepts_valid_https_origin() {
        let vars = HashMap::from([(
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("https://example.com"),
        )]);
        let config = Config::from_map(&vars).expect("valid https origin should be accepted");
        assert_eq!(config.cors_allowed_origins, vec!["https://example.com"]);
    }

    #[test]
    fn cors_accepts_valid_http_origin_with_port() {
        let vars = HashMap::from([(
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("http://localhost:5173"),
        )]);
        let config = Config::from_map(&vars).expect("http origin with port should be accepted");
        assert_eq!(config.cors_allowed_origins, vec!["http://localhost:5173"]);
    }

    #[test]
    fn cors_accepts_multiple_valid_origins() {
        let vars = HashMap::from([(
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("https://app.example.com,https://admin.example.com"),
        )]);
        let config = Config::from_map(&vars).expect("multiple valid origins should be accepted");
        assert_eq!(
            config.cors_allowed_origins,
            vec!["https://app.example.com", "https://admin.example.com"]
        );
    }

    #[test]
    fn cors_trims_whitespace_around_origins() {
        let vars = HashMap::from([(
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("  https://app.example.com , https://admin.example.com  "),
        )]);
        let config = Config::from_map(&vars).expect("whitespace should be trimmed");
        assert_eq!(
            config.cors_allowed_origins,
            vec!["https://app.example.com", "https://admin.example.com"]
        );
    }

    #[test]
    fn cors_rejects_wildcard() {
        let vars = HashMap::from([(
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("*"),
        )]);
        let error = Config::from_map(&vars).expect_err("wildcard must be rejected");
        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn cors_rejects_null_origin() {
        let vars = HashMap::from([(
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("null"),
        )]);
        let error = Config::from_map(&vars).expect_err("null origin must be rejected");
        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn cors_rejects_missing_scheme() {
        let vars = HashMap::from([(
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("example.com"),
        )]);
        let error = Config::from_map(&vars).expect_err("origin without scheme must be rejected");
        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn cors_rejects_ftp_scheme() {
        let vars = HashMap::from([(
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("ftp://example.com"),
        )]);
        let error = Config::from_map(&vars).expect_err("ftp scheme must be rejected");
        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn cors_rejects_origin_with_path() {
        let vars = HashMap::from([(
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("https://example.com/api"),
        )]);
        let error = Config::from_map(&vars).expect_err("origin with path must be rejected");
        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn cors_rejects_origin_with_query_string() {
        let vars = HashMap::from([(
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("https://example.com?foo=bar"),
        )]);
        let error = Config::from_map(&vars).expect_err("origin with query string must be rejected");
        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn cors_rejects_origin_with_fragment() {
        let vars = HashMap::from([(
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("https://example.com#section"),
        )]);
        let error = Config::from_map(&vars).expect_err("origin with fragment must be rejected");
        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn cors_rejects_port_out_of_range() {
        let vars = HashMap::from([(
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("https://example.com:99999"),
        )]);
        let error = Config::from_map(&vars).expect_err("port > 65535 must be rejected");
        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn cors_rejects_empty_list() {
        let vars = HashMap::from([(
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("   "),
        )]);
        let error = Config::from_map(&vars).expect_err("empty origin list must be rejected");
        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn cors_production_rejects_default_dev_origins() {
        let vars = HashMap::from([
            (String::from("PREDIFI_APP_ENV"), String::from("production")),
            (
                String::from("PREDIFI_SECRET_KEY"),
                String::from("custom-prod-secret-key-at-least-32-chars-long"),
            ),
            (
                String::from("PREDIFI_WS_ALLOWED_ORIGINS"),
                String::from("https://app.predifi.com"),
            ),
        ]);
        let error =
            Config::from_map(&vars).expect_err("production must reject default dev CORS origins");
        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
        let error_msg = error.to_string();
        assert!(
            error_msg.contains("default development CORS origins must not be used in production"),
            "expected actionable error message, got: {error_msg}"
        );
    }

    #[test]
    fn cors_production_rejects_localhost_origin() {
        let vars = HashMap::from([
            (String::from("PREDIFI_APP_ENV"), String::from("production")),
            (
                String::from("PREDIFI_SECRET_KEY"),
                String::from("custom-prod-secret-key-at-least-32-chars-long"),
            ),
            (
                String::from("PREDIFI_WS_ALLOWED_ORIGINS"),
                String::from("https://app.predifi.com"),
            ),
            (
                String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
                String::from("https://localhost:3000"),
            ),
        ]);
        let error = Config::from_map(&vars).expect_err("production must reject localhost origins");
        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
        let error_msg = error.to_string();
        assert!(
            error_msg.contains("cannot be localhost or loopback"),
            "expected localhost rejection, got: {error_msg}"
        );
    }

    #[test]
    fn cors_production_rejects_http_origin() {
        let vars = HashMap::from([
            (String::from("PREDIFI_APP_ENV"), String::from("production")),
            (
                String::from("PREDIFI_SECRET_KEY"),
                String::from("custom-prod-secret-key-at-least-32-chars-long"),
            ),
            (
                String::from("PREDIFI_WS_ALLOWED_ORIGINS"),
                String::from("https://app.predifi.com"),
            ),
            (
                String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
                String::from("http://app.predifi.com"),
            ),
        ]);
        let error = Config::from_map(&vars).expect_err("production must reject http origins");
        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "PREDIFI_CORS_ALLOWED_ORIGINS",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
        let error_msg = error.to_string();
        assert!(
            error_msg.contains("must use https://"),
            "expected https requirement, got: {error_msg}"
        );
    }

    #[test]
    fn cors_production_accepts_valid_https_origins() {
        let vars = HashMap::from([
            (String::from("PREDIFI_APP_ENV"), String::from("production")),
            (
                String::from("PREDIFI_SECRET_KEY"),
                String::from("custom-prod-secret-key-at-least-32-chars-long"),
            ),
            (
                String::from("PREDIFI_WS_ALLOWED_ORIGINS"),
                String::from("https://app.predifi.com"),
            ),
            (
                String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
                String::from("https://app.predifi.com,https://predifi.com"),
            ),
            (
                String::from("PREDIFI_CONTRACT_ID"),
                String::from("CTESTCONTRACTID00000000000000000000000000000000000000"),
            ),
        ]);
        let config = Config::from_map(&vars).expect("valid production origins must be accepted");
        assert_eq!(
            config.cors_allowed_origins,
            vec!["https://app.predifi.com", "https://predifi.com"]
        );
    }

    #[test]
    fn cors_development_allows_explicit_localhost_origin() {
        let vars = HashMap::from([
            (String::from("PREDIFI_APP_ENV"), String::from("development")),
            (
                String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
                String::from("http://localhost:3000,http://localhost:5173"),
            ),
        ]);
        let config = Config::from_map(&vars).expect("development explicitly allows localhost");
        assert_eq!(
            config.cors_allowed_origins,
            vec!["http://localhost:3000", "http://localhost:5173"]
        );
    }

    // ── Specific error codes for config (issue #976) ──────────────────────────

    /// `get_required_string` returns `MissingRequired` when the variable is absent.
    #[test]
    fn missing_required_error_when_variable_absent() {
        let vars = HashMap::new();
        let error = get_required_string(&vars, "PREDIFI_SECRET_KEY", "JWT signing secret")
            .expect_err("absent required variable must produce an error");

        assert!(
            matches!(
                error,
                ConfigError::MissingRequired {
                    key: "PREDIFI_SECRET_KEY",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    /// `get_required_string` succeeds when the variable is present.
    #[test]
    fn required_string_succeeds_when_variable_present() {
        let vars = HashMap::from([(
            String::from("PREDIFI_SECRET_KEY"),
            String::from("supersecret"),
        )]);
        let value = get_required_string(&vars, "PREDIFI_SECRET_KEY", "JWT signing secret")
            .expect("present required variable must succeed");
        assert_eq!(value, "supersecret");
    }

    /// `MissingRequired` display message includes the variable name and description.
    #[test]
    fn missing_required_display_includes_key_and_description() {
        let error = ConfigError::MissingRequired {
            key: "PREDIFI_SECRET_KEY",
            description: "JWT signing secret",
        };
        let msg = error.to_string();
        assert!(
            msg.contains("PREDIFI_SECRET_KEY"),
            "error message must contain the variable name, got: {msg}"
        );
        assert!(
            msg.contains("JWT signing secret"),
            "error message must contain the description, got: {msg}"
        );
    }

    /// `InvalidNumber` display message includes the variable name and bad value.
    #[test]
    fn invalid_number_display_includes_key_and_value() {
        let vars = HashMap::from([(String::from("PREDIFI_APP_PORT"), String::from("not-a-port"))]);
        let error = Config::from_map(&vars).expect_err("non-numeric port must fail");
        let msg = error.to_string();
        assert!(
            msg.contains("PREDIFI_APP_PORT"),
            "error message must contain the variable name, got: {msg}"
        );
        assert!(
            msg.contains("not-a-port"),
            "error message must contain the bad value, got: {msg}"
        );
    }

    /// `InvalidValue` display message includes the variable name and reason.
    #[test]
    fn invalid_value_display_includes_key_and_reason() {
        let vars = HashMap::from([
            (
                String::from("PREDIFI_DB_MIN_CONNECTIONS"),
                String::from("50"),
            ),
            (
                String::from("PREDIFI_DB_MAX_CONNECTIONS"),
                String::from("10"),
            ),
        ]);
        let error = Config::from_map(&vars).expect_err("min > max must fail");
        let msg = error.to_string();
        assert!(
            msg.contains("PREDIFI_DB_MIN_CONNECTIONS"),
            "error message must contain the variable name, got: {msg}"
        );
    }

    // ── String field overrides ────────────────────────────────────────────────

    #[test]
    fn config_reads_host_from_env() {
        let vars = HashMap::from([(String::from("PREDIFI_APP_HOST"), String::from("127.0.0.1"))]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.host, "127.0.0.1");
    }

    #[test]
    fn config_reads_database_url_from_env() {
        let vars = HashMap::from([(
            String::from("PREDIFI_DATABASE_URL"),
            String::from("postgres://user:pass@db:5432/mydb"),
        )]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.database_url, "postgres://user:pass@db:5432/mydb");
    }

    #[test]
    fn config_reads_stellar_rpc_url_from_env() {
        let vars = HashMap::from([(
            String::from("PREDIFI_STELLAR_RPC_URL"),
            String::from("https://my-rpc.example.com"),
        )]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.stellar_rpc_url, "https://my-rpc.example.com");
    }

    #[test]
    fn config_reads_redis_url_from_env() {
        let vars = HashMap::from([(
            String::from("PREDIFI_REDIS_URL"),
            String::from("redis://redis-host:6380"),
        )]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.redis_url, "redis://redis-host:6380");
    }

    #[test]
    fn config_reads_log_level_from_env() {
        let vars = HashMap::from([(String::from("RUST_LOG"), String::from("warn"))]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.log_level, "warn");
    }

    // ── Numeric field overrides and invalid-input rejection ───────────────────

    #[test]
    fn config_reads_port_from_env() {
        let vars = HashMap::from([(String::from("PREDIFI_APP_PORT"), String::from("8080"))]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.port, 8080);
    }

    #[test]
    fn config_reads_db_max_connections_from_env() {
        let vars = HashMap::from([(
            String::from("PREDIFI_DB_MAX_CONNECTIONS"),
            String::from("25"),
        )]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.db_max_connections, 25);
    }

    #[test]
    fn config_rejects_non_numeric_db_max_connections() {
        let vars = HashMap::from([(
            String::from("PREDIFI_DB_MAX_CONNECTIONS"),
            String::from("x"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidNumber {
                key: "PREDIFI_DB_MAX_CONNECTIONS",
                ..
            })
        ));
    }

    #[test]
    fn config_reads_db_acquire_timeout_from_env() {
        let vars = HashMap::from([(
            String::from("PREDIFI_DB_ACQUIRE_TIMEOUT_SECS"),
            String::from("60"),
        )]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.db_acquire_timeout_secs, 60);
    }

    #[test]
    fn config_rejects_non_numeric_db_acquire_timeout() {
        let vars = HashMap::from([(
            String::from("PREDIFI_DB_ACQUIRE_TIMEOUT_SECS"),
            String::from("never"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidNumber {
                key: "PREDIFI_DB_ACQUIRE_TIMEOUT_SECS",
                ..
            })
        ));
    }

    #[test]
    fn config_reads_rpc_health_timeout_from_env() {
        let vars = HashMap::from([(
            String::from("PREDIFI_RPC_HEALTH_TIMEOUT_SECS"),
            String::from("5"),
        )]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.rpc_health_timeout_secs, 5);
    }

    #[test]
    fn config_rejects_non_numeric_rpc_health_timeout() {
        let vars = HashMap::from([(
            String::from("PREDIFI_RPC_HEALTH_TIMEOUT_SECS"),
            String::from("fast"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidNumber {
                key: "PREDIFI_RPC_HEALTH_TIMEOUT_SECS",
                ..
            })
        ));
    }

    #[test]
    fn config_reads_rpc_health_retry_count_from_env() {
        let vars = HashMap::from([(
            String::from("PREDIFI_RPC_HEALTH_RETRY_COUNT"),
            String::from("5"),
        )]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.rpc_health_retry_count, 5);
    }

    #[test]
    fn config_rejects_rpc_health_retry_count_overflow() {
        // u8 max is 255; 300 overflows
        let vars = HashMap::from([(
            String::from("PREDIFI_RPC_HEALTH_RETRY_COUNT"),
            String::from("300"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidNumber {
                key: "PREDIFI_RPC_HEALTH_RETRY_COUNT",
                ..
            })
        ));
    }

    #[test]
    fn config_reads_rpc_timeout_secs_from_env() {
        let vars = HashMap::from([(String::from("RPC_TIMEOUT_SECS"), String::from("30"))]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.rpc_timeout_secs, 30);
    }

    #[test]
    fn config_rejects_non_numeric_rpc_timeout() {
        let vars = HashMap::from([(String::from("RPC_TIMEOUT_SECS"), String::from("inf"))]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidNumber {
                key: "RPC_TIMEOUT_SECS",
                ..
            })
        ));
    }

    // ── Graceful shutdown timeout ─────────────────────────────────────────────

    /// When `PREDIFI_SHUTDOWN_TIMEOUT_SECS` is absent we fall back to the
    /// compiled-in default.
    #[test]
    fn shutdown_timeout_defaults_when_env_absent() {
        let vars = HashMap::new();
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.shutdown_timeout_secs, DEFAULT_SHUTDOWN_TIMEOUT_SECS);
    }

    /// Operators can override the shutdown grace period.
    #[test]
    fn shutdown_timeout_reads_from_env() {
        let vars = HashMap::from([(
            String::from("PREDIFI_SHUTDOWN_TIMEOUT_SECS"),
            String::from("60"),
        )]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.shutdown_timeout_secs, 60);
    }

    /// Reject a zero shutdown timeout — the server must allow at least one
    /// tick for in-flight requests to begin draining.
    #[test]
    fn shutdown_timeout_rejects_zero() {
        let vars = HashMap::from([(
            String::from("PREDIFI_SHUTDOWN_TIMEOUT_SECS"),
            String::from("0"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_SHUTDOWN_TIMEOUT_SECS",
                ..
            })
        ));
    }

    /// Reject non-numeric shutdown timeout values.
    #[test]
    fn shutdown_timeout_rejects_non_numeric() {
        let vars = HashMap::from([(
            String::from("PREDIFI_SHUTDOWN_TIMEOUT_SECS"),
            String::from("fast"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidNumber {
                key: "PREDIFI_SHUTDOWN_TIMEOUT_SECS",
                ..
            })
        ));
    }

    #[test]
    fn config_reads_treasury_fee_bps_from_env() {
        let vars = HashMap::from([(
            String::from("PREDIFI_TREASURY_FEE_BPS"),
            String::from("500"),
        )]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.treasury_fee_bps, 500);
    }

    #[test]
    fn config_rejects_non_numeric_treasury_fee_bps() {
        let vars = HashMap::from([(
            String::from("PREDIFI_TREASURY_FEE_BPS"),
            String::from("high"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidNumber {
                key: "PREDIFI_TREASURY_FEE_BPS",
                ..
            })
        ));
    }

    #[test]
    fn config_reads_referral_fee_bps_from_env() {
        let vars = HashMap::from([(
            String::from("PREDIFI_REFERRAL_FEE_BPS"),
            String::from("3000"),
        )]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.referral_fee_bps, 3000);
    }

    #[test]
    fn config_rejects_non_numeric_referral_fee_bps() {
        let vars = HashMap::from([(
            String::from("PREDIFI_REFERRAL_FEE_BPS"),
            String::from("lots"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidNumber {
                key: "PREDIFI_REFERRAL_FEE_BPS",
                ..
            })
        ));
    }

    // ── Optional fields ───────────────────────────────────────────────────────

    #[test]
    fn sentry_dsn_is_none_when_absent() {
        let vars = HashMap::new();
        let config = Config::from_map(&vars).unwrap();
        assert!(config.sentry_dsn.is_none());
    }

    #[test]
    fn sentry_dsn_is_some_when_set() {
        let vars = HashMap::from([(
            String::from("PREDIFI_SENTRY_DSN"),
            String::from("https://abc@sentry.io/123"),
        )]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(
            config.sentry_dsn.as_deref(),
            Some("https://abc@sentry.io/123")
        );
    }

    // ── bind_address ──────────────────────────────────────────────────────────

    #[test]
    fn bind_address_combines_host_and_port() {
        let vars = HashMap::from([
            (String::from("PREDIFI_APP_HOST"), String::from("0.0.0.0")),
            (String::from("PREDIFI_APP_PORT"), String::from("9000")),
        ]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.bind_address(), "0.0.0.0:9000");
    }

    // ── CORS: additional edge cases ───────────────────────────────────────────

    #[test]
    fn cors_accepts_trailing_slash_on_root() {
        // A bare trailing slash (scheme://host/) is permitted — it is equivalent
        // to scheme://host and browsers may normalise to this form.
        let vars = HashMap::from([(
            String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
            String::from("https://example.com/"),
        )]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.cors_allowed_origins, vec!["https://example.com/"]);
    }

    #[test]
    fn validate_rejects_invalid_database_url_scheme() {
        let config = Config {
            database_url: String::from("mysql://localhost/db"),
            ..Config::default_for_test()
        };
        let error = config.validate().expect_err("mysql URL must be rejected");
        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "PREDIFI_DATABASE_URL",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validate_rejects_treasury_fee_above_100_percent() {
        let config = Config {
            treasury_fee_bps: 10_001,
            ..Config::default_for_test()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_TREASURY_FEE_BPS",
                ..
            })
        ));
    }

    #[test]
    fn validate_accepts_default_for_test_config() {
        Config::default_for_test()
            .validate()
            .expect("default test config must be valid");
    }

    // ── New validations ───────────────────────────────────────────────────────

    // -- host --

    #[test]
    fn validate_rejects_empty_host() {
        let config = Config {
            host: String::new(),
            ..Config::default_for_test()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_APP_HOST",
                ..
            })
        ));
    }

    #[test]
    fn config_rejects_empty_host_via_env() {
        let vars = HashMap::from([(String::from("PREDIFI_APP_HOST"), String::new())]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_APP_HOST",
                ..
            })
        ));
    }

    // -- URL host presence --

    #[test]
    fn validate_rejects_database_url_without_host() {
        let config = Config {
            database_url: String::from("postgres:///dbname"),
            ..Config::default_for_test()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_DATABASE_URL",
                ..
            })
        ));
    }

    #[test]
    fn validate_rejects_redis_url_without_host() {
        let config = Config {
            redis_url: String::from("redis:///"),
            ..Config::default_for_test()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_REDIS_URL",
                ..
            })
        ));
    }

    #[test]
    fn validate_rejects_stellar_rpc_url_without_host() {
        let config = Config {
            stellar_rpc_url: String::from("https:///rpc"),
            ..Config::default_for_test()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_STELLAR_RPC_URL",
                ..
            })
        ));
    }

    #[test]
    fn validate_accepts_urls_with_userinfo() {
        // postgres://user:pass@host/db is a legitimate URL with userinfo — host must not be
        // mistakenly parsed as empty.
        let config = Config {
            database_url: String::from("postgres://user:pass@db-host:5432/predifi"),
            ..Config::default_for_test()
        };
        config
            .validate()
            .expect("URL with userinfo should be valid");
    }

    // -- db_connect_timeout_secs --

    #[test]
    fn validate_rejects_zero_db_connect_timeout_secs() {
        let config = Config {
            db_connect_timeout_secs: 0,
            ..Config::default_for_test()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_CONNECT_TIMEOUT_SECS",
                ..
            })
        ));
    }

    #[test]
    fn config_rejects_zero_db_connect_timeout_via_env() {
        let vars = HashMap::from([(
            String::from("PREDIFI_DB_CONNECT_TIMEOUT_SECS"),
            String::from("0"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_CONNECT_TIMEOUT_SECS",
                ..
            })
        ));
    }

    // -- db_connect_base_delay_ms > db_connect_max_delay_ms --

    #[test]
    fn validate_rejects_base_delay_larger_than_max_delay_when_retries_enabled() {
        let config = Config {
            db_connect_max_attempts: 3,
            db_connect_base_delay_ms: 10_000,
            db_connect_max_delay_ms: 1_000,
            ..Config::default_for_test()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_CONNECT_BASE_DELAY_MS",
                ..
            })
        ));
    }

    #[test]
    fn validate_accepts_base_delay_larger_than_max_delay_when_only_one_attempt() {
        // With a single attempt there are no retries so the delay values are
        // never used — the constraint does not apply.
        let config = Config {
            db_connect_max_attempts: 1,
            db_connect_base_delay_ms: 10_000,
            db_connect_max_delay_ms: 0,
            ..Config::default_for_test()
        };
        config
            .validate()
            .expect("base > max delay is harmless when max_attempts == 1");
    }

    #[test]
    fn config_rejects_base_delay_larger_than_max_delay_via_env() {
        let vars = HashMap::from([
            (
                String::from("PREDIFI_DB_CONNECT_MAX_ATTEMPTS"),
                String::from("3"),
            ),
            (
                String::from("PREDIFI_DB_CONNECT_BASE_DELAY_MS"),
                String::from("9000"),
            ),
            (
                String::from("PREDIFI_DB_CONNECT_MAX_DELAY_MS"),
                String::from("1000"),
            ),
        ]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_CONNECT_BASE_DELAY_MS",
                ..
            })
        ));
    }

    // -- rpc_health_timeout_secs > rpc_timeout_secs --

    #[test]
    fn validate_rejects_rpc_health_timeout_exceeding_rpc_timeout() {
        let config = Config {
            rpc_health_timeout_secs: 60,
            rpc_timeout_secs: 30,
            ..Config::default_for_test()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_RPC_HEALTH_TIMEOUT_SECS",
                ..
            })
        ));
    }

    #[test]
    fn validate_accepts_rpc_health_timeout_equal_to_rpc_timeout() {
        let config = Config {
            rpc_health_timeout_secs: 30,
            rpc_timeout_secs: 30,
            ..Config::default_for_test()
        };
        config
            .validate()
            .expect("health timeout equal to rpc timeout should be valid");
    }

    #[test]
    fn config_rejects_rpc_health_timeout_exceeding_rpc_timeout_via_env() {
        let vars = HashMap::from([
            (
                String::from("PREDIFI_RPC_HEALTH_TIMEOUT_SECS"),
                String::from("100"),
            ),
            (String::from("RPC_TIMEOUT_SECS"), String::from("30")),
        ]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_RPC_HEALTH_TIMEOUT_SECS",
                ..
            })
        ));
    }

    // -- production dev secret rejection --

    #[test]
    fn validate_rejects_default_dev_secret_in_production() {
        let config = Config {
            app_env: String::from("production"),
            secret_key: String::from(DEFAULT_SECRET_KEY),
            // Isolate the secret-key check: use a non-default CORS origin so
            // the production CORS-default check doesn't fire first.
            cors_allowed_origins: vec!["https://app.predifi.com".to_string()],
            ..Config::default_for_test()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_SECRET_KEY",
                ..
            })
        ));
    }

    #[test]
    fn validate_accepts_default_dev_secret_outside_production() {
        // The dev secret is only banned in production; development/staging are fine.
        let config = Config {
            app_env: String::from("staging"),
            secret_key: String::from(DEFAULT_SECRET_KEY),
            ..Config::default_for_test()
        };
        config
            .validate()
            .expect("dev secret is allowed outside production");
    }

    #[test]
    fn validate_accepts_strong_secret_in_production() {
        let config = Config {
            app_env: String::from("production"),
            secret_key: String::from("a-very-strong-production-secret-key-at-least-32-bytes"),
            allowed_ws_origins: vec!["https://app.predifi.com".to_string()],
            cors_allowed_origins: vec!["https://app.predifi.com".to_string()],
            contract_id: String::from("CTESTCONTRACTID00000000000000000000000000000000000000"),
            ..Config::default_for_test()
        };
        config
            .validate()
            .expect("strong secret should be valid in production");
    }

    #[test]
    fn validate_rejects_empty_ws_origins_in_production() {
        let config = Config {
            app_env: String::from("production"),
            secret_key: String::from("a-very-strong-production-secret-key-at-least-32-bytes"),
            allowed_ws_origins: Vec::new(),
            // Isolate the WS-origins check: use a non-default CORS origin so
            // the production CORS-default check doesn't fire first.
            cors_allowed_origins: vec!["https://app.predifi.com".to_string()],
            ..Config::default_for_test()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_WS_ALLOWED_ORIGINS",
                ..
            })
        ));
    }

    #[test]
    fn config_rejects_default_secret_in_production_via_env() {
        let vars = HashMap::from([
            (String::from("PREDIFI_APP_ENV"), String::from("production")),
            // Default secret key is not set, so from_map uses DEFAULT_SECRET_KEY.
            // Isolate that check: use a non-default CORS origin so the
            // production CORS-default check doesn't fire first.
            (
                String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
                String::from("https://app.predifi.com"),
            ),
        ]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_SECRET_KEY",
                ..
            })
        ));
    }

    // -- app_env field --

    #[test]
    fn config_reads_app_env_from_env() {
        let vars = HashMap::from([(String::from("PREDIFI_APP_ENV"), String::from("staging"))]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.app_env, "staging");
    }

    #[test]
    fn config_app_env_defaults_to_development() {
        let vars = HashMap::new();
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.app_env, DEFAULT_APP_ENV);
    }

    #[test]
    fn configured_stellar_network_must_match_rpc_endpoint() {
        let vars = HashMap::from([
            (
                String::from("PREDIFI_STELLAR_RPC_URL"),
                String::from("https://horizon-testnet.stellar.org"),
            ),
            (
                String::from("PREDIFI_STELLAR_NETWORK"),
                String::from("public"),
            ),
        ]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_STELLAR_NETWORK",
                ..
            })
        ));
    }

    // ── Abuse controls (issue #103) ──────────────────────────────────────────

    #[test]
    fn abuse_controls_use_defaults_when_env_absent() {
        let vars = HashMap::new();
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(
            config.abuse_quota_market_create_per_window,
            DEFAULT_ABUSE_QUOTA_MARKET_CREATE_PER_WINDOW
        );
        assert_eq!(
            config.abuse_quota_wallet_action_per_window,
            DEFAULT_ABUSE_QUOTA_WALLET_ACTION_PER_WINDOW
        );
        assert_eq!(
            config.abuse_quota_window_secs,
            DEFAULT_ABUSE_QUOTA_WINDOW_SECS
        );
        assert_eq!(
            config.abuse_quota_network_market_create_per_window,
            DEFAULT_ABUSE_QUOTA_NETWORK_MARKET_CREATE_PER_WINDOW
        );
        assert_eq!(
            config.abuse_quota_network_wallet_action_per_window,
            DEFAULT_ABUSE_QUOTA_NETWORK_WALLET_ACTION_PER_WINDOW
        );
        assert_eq!(
            config.abuse_min_validation_cost,
            DEFAULT_ABUSE_MIN_VALIDATION_COST
        );
    }

    #[test]
    fn abuse_controls_read_from_env() {
        let vars = HashMap::from([
            (
                String::from("PREDIFI_ABUSE_QUOTA_MARKET_CREATE_PER_WINDOW"),
                String::from("2"),
            ),
            (
                String::from("PREDIFI_ABUSE_QUOTA_WALLET_ACTION_PER_WINDOW"),
                String::from("10"),
            ),
            (
                String::from("PREDIFI_ABUSE_QUOTA_WINDOW_SECS"),
                String::from("120"),
            ),
            (
                String::from("PREDIFI_ABUSE_QUOTA_NETWORK_MARKET_CREATE_PER_WINDOW"),
                String::from("100"),
            ),
            (
                String::from("PREDIFI_ABUSE_QUOTA_NETWORK_WALLET_ACTION_PER_WINDOW"),
                String::from("1000"),
            ),
            (
                String::from("PREDIFI_ABUSE_MIN_VALIDATION_COST"),
                String::from("5"),
            ),
        ]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.abuse_quota_market_create_per_window, 2);
        assert_eq!(config.abuse_quota_wallet_action_per_window, 10);
        assert_eq!(config.abuse_quota_window_secs, 120);
        assert_eq!(config.abuse_quota_network_market_create_per_window, 100);
        assert_eq!(config.abuse_quota_network_wallet_action_per_window, 1000);
        assert_eq!(config.abuse_min_validation_cost, 5);
    }

    #[test]
    fn abuse_controls_reject_zero_window() {
        let vars = HashMap::from([(
            String::from("PREDIFI_ABUSE_QUOTA_WINDOW_SECS"),
            String::from("0"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_WINDOW_SECS",
                ..
            })
        ));
    }

    #[test]
    fn abuse_controls_reject_zero_per_wallet_quota() {
        let vars = HashMap::from([(
            String::from("PREDIFI_ABUSE_QUOTA_MARKET_CREATE_PER_WINDOW"),
            String::from("0"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_MARKET_CREATE_PER_WINDOW",
                ..
            })
        ));
    }

    #[test]
    fn abuse_controls_reject_per_wallet_quota_above_network_quota() {
        let vars = HashMap::from([
            (
                String::from("PREDIFI_ABUSE_QUOTA_MARKET_CREATE_PER_WINDOW"),
                String::from("1000"),
            ),
            (
                String::from("PREDIFI_ABUSE_QUOTA_NETWORK_MARKET_CREATE_PER_WINDOW"),
                String::from("100"),
            ),
        ]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_ABUSE_QUOTA_MARKET_CREATE_PER_WINDOW",
                ..
            })
        ));
    }

    #[test]
    fn abuse_controls_reject_non_numeric_quota() {
        let vars = HashMap::from([(
            String::from("PREDIFI_ABUSE_QUOTA_WALLET_ACTION_PER_WINDOW"),
            String::from("many"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidNumber {
                key: "PREDIFI_ABUSE_QUOTA_WALLET_ACTION_PER_WINDOW",
                ..
            })
        ));
    }

    // ── RPC quotas / failover (issue #105) ───────────────────────────────────

    #[test]
    fn rpc_quota_settings_use_defaults_when_env_absent() {
        let vars = HashMap::new();
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.rpc_per_network_budget, DEFAULT_RPC_PER_NETWORK_BUDGET);
        assert_eq!(config.rpc_budget_window_secs, DEFAULT_RPC_BUDGET_WINDOW_SECS);
        assert_eq!(config.rpc_max_retries, DEFAULT_RPC_MAX_RETRIES);
        assert_eq!(
            config.rpc_priority_classes,
            vec!["transaction", "indexer", "health"]
        );
        assert!(config.rpc_provider_fallback_order.is_empty());
        assert_eq!(
            config.rpc_provider_unhealthy_threshold,
            DEFAULT_RPC_PROVIDER_UNHEALTHY_THRESHOLD
        );
        assert_eq!(
            config.rpc_provider_healthy_threshold,
            DEFAULT_RPC_PROVIDER_HEALTHY_THRESHOLD
        );
        assert_eq!(config.rpc_disagreement_policy, DEFAULT_RPC_DISAGREEMENT_POLICY);
    }

    #[test]
    fn rpc_quota_settings_read_from_env() {
        let vars = HashMap::from([
            (
                String::from("PREDIFI_RPC_PER_NETWORK_BUDGET"),
                String::from("250"),
            ),
            (
                String::from("PREDIFI_RPC_BUDGET_WINDOW_SECS"),
                String::from("30"),
            ),
            (String::from("PREDIFI_RPC_MAX_RETRIES"), String::from("4")),
            (
                String::from("PREDIFI_RPC_PRIORITY_CLASSES"),
                String::from("transaction, indexer"),
            ),
            (
                String::from("PREDIFI_RPC_PROVIDER_FALLBACK_ORDER"),
                String::from("https://a.example.com, https://b.example.com"),
            ),
            (
                String::from("PREDIFI_RPC_PROVIDER_UNHEALTHY_THRESHOLD"),
                String::from("5"),
            ),
            (
                String::from("PREDIFI_RPC_PROVIDER_HEALTHY_THRESHOLD"),
                String::from("3"),
            ),
            (
                String::from("PREDIFI_RPC_DISAGREEMENT_POLICY"),
                String::from("strict"),
            ),
        ]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.rpc_per_network_budget, 250);
        assert_eq!(config.rpc_budget_window_secs, 30);
        assert_eq!(config.rpc_max_retries, 4);
        assert_eq!(config.rpc_priority_classes, vec!["transaction", "indexer"]);
        assert_eq!(
            config.rpc_provider_fallback_order,
            vec!["https://a.example.com", "https://b.example.com"]
        );
        assert_eq!(config.rpc_provider_unhealthy_threshold, 5);
        assert_eq!(config.rpc_provider_healthy_threshold, 3);
        assert_eq!(config.rpc_disagreement_policy, "strict");
    }

    #[test]
    fn rpc_quota_settings_reject_zero_budget() {
        let vars = HashMap::from([(
            String::from("PREDIFI_RPC_PER_NETWORK_BUDGET"),
            String::from("0"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_RPC_PER_NETWORK_BUDGET",
                ..
            })
        ));
    }

    #[test]
    fn rpc_quota_settings_reject_empty_priority_classes() {
        let vars = HashMap::from([(
            String::from("PREDIFI_RPC_PRIORITY_CLASSES"),
            String::from("  "),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_RPC_PRIORITY_CLASSES",
                ..
            })
        ));
    }

    #[test]
    fn rpc_quota_settings_reject_zero_health_thresholds() {
        let vars = HashMap::from([(
            String::from("PREDIFI_RPC_PROVIDER_UNHEALTHY_THRESHOLD"),
            String::from("0"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_RPC_PROVIDER_UNHEALTHY_THRESHOLD",
                ..
            })
        ));
    }

    #[test]
    fn rpc_quota_settings_reject_unknown_disagreement_policy() {
        let vars = HashMap::from([(
            String::from("PREDIFI_RPC_DISAGREEMENT_POLICY"),
            String::from("majority-rules"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_RPC_DISAGREEMENT_POLICY",
                ..
            })
        ));
    }

    #[test]
    fn rpc_quota_settings_reject_non_numeric_retries() {
        let vars = HashMap::from([(
            String::from("PREDIFI_RPC_MAX_RETRIES"),
            String::from("lots"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidNumber {
                key: "PREDIFI_RPC_MAX_RETRIES",
                ..
            })
        ));
    }

    // ── Contract identity pinning (issue #347) ───────────────────────────────

    fn write_manifest(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).expect("write manifest");
        path
    }

    fn manifest_json(
        contract_id: &str,
        network_passphrase: &str,
        interface_version: u32,
        generated_at: u64,
    ) -> String {
        format!(
            "{{\n  \"contract_id\": \"{contract_id}\",\n  \"network_passphrase\": \"{network_passphrase}\",\n  \"interface_version\": {interface_version},\n  \"generated_at\": {generated_at}\n}}\n"
        )
    }

    fn identity_config(path: &std::path::Path, contract_id: &str) -> ContractIdentityConfig {
        ContractIdentityConfig {
            manifest_path: path.to_string_lossy().into_owned(),
            contract_id: String::from(contract_id),
            network_passphrase: String::from(DEFAULT_NETWORK_PASSPHRASE),
            interface_version: SUPPORTED_CONTRACT_INTERFACE_VERSION,
            manifest_max_age_secs: DEFAULT_MANIFEST_MAX_AGE_SECS,
        }
    }

    #[test]
    fn contract_identity_verified_when_manifest_matches() {
        let dir = std::env::temp_dir().join(format!("predifi-manifest-ok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_manifest(
            &dir,
            "ok.json",
            &manifest_json("CABC", DEFAULT_NETWORK_PASSPHRASE, 1, 1_000),
        );
        let status = identity_config(&path, "CABC").verify(1_100);
        assert!(status.is_verified(), "expected verified, got {status:?}");
        assert!(!status.mutations_disabled());
        assert_eq!(status.diagnostic_code(), "contract_identity_verified");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn contract_identity_mismatch_disables_mutations() {
        let dir =
            std::env::temp_dir().join(format!("predifi-manifest-mm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_manifest(
            &dir,
            "mm.json",
            &manifest_json("COTHER", DEFAULT_NETWORK_PASSPHRASE, 1, 1_000),
        );
        let status = identity_config(&path, "CABC").verify(1_100);
        assert!(matches!(status, ContractIdentityStatus::Mismatch { .. }));
        assert!(status.mutations_disabled());
        assert_eq!(status.diagnostic_code(), "contract_identity_mismatch");
        assert!(status.diagnostic_message().contains("mutations disabled"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn contract_identity_mismatch_on_network_passphrase() {
        let dir =
            std::env::temp_dir().join(format!("predifi-manifest-net-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_manifest(
            &dir,
            "net.json",
            &manifest_json("CABC", "Public Global Stellar Network ; September 2015", 1, 1_000),
        );
        let status = identity_config(&path, "CABC").verify(1_100);
        assert!(matches!(status, ContractIdentityStatus::Mismatch { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn contract_identity_mismatch_on_unsupported_interface() {
        let dir =
            std::env::temp_dir().join(format!("predifi-manifest-if-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_manifest(
            &dir,
            "if.json",
            &manifest_json("CABC", DEFAULT_NETWORK_PASSPHRASE, 99, 1_000),
        );
        let status = identity_config(&path, "CABC").verify(1_100);
        assert!(matches!(status, ContractIdentityStatus::Mismatch { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn contract_identity_unavailable_when_manifest_missing() {
        let path = std::env::temp_dir().join(format!(
            "predifi-manifest-missing-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let status = identity_config(&path, "CABC").verify(1_100);
        assert!(matches!(status, ContractIdentityStatus::Unavailable { .. }));
        assert!(status.mutations_disabled());
        assert_eq!(status.diagnostic_code(), "contract_identity_unavailable");
    }

    #[test]
    fn contract_identity_unavailable_when_manifest_malformed() {
        let dir =
            std::env::temp_dir().join(format!("predifi-manifest-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_manifest(&dir, "bad.json", "{ not json");
        let status = identity_config(&path, "CABC").verify(1_100);
        assert!(matches!(status, ContractIdentityStatus::Unavailable { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn contract_identity_stale_manifest_disables_mutations() {
        let dir =
            std::env::temp_dir().join(format!("predifi-manifest-st-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_manifest(
            &dir,
            "st.json",
            &manifest_json("CABC", DEFAULT_NETWORK_PASSPHRASE, 1, 1_000),
        );
        let mut cfg = identity_config(&path, "CABC");
        cfg.manifest_max_age_secs = 60;
        let status = cfg.verify(10_000);
        assert!(matches!(status, ContractIdentityStatus::Stale { .. }));
        assert!(status.mutations_disabled());
        assert_eq!(status.diagnostic_code(), "contract_identity_stale");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn contract_identity_never_falls_back_to_unverified_contract() {
        // A missing manifest must never produce a Verified status even when the
        // configured contract ID is non-empty.
        let path = std::env::temp_dir().join(format!(
            "predifi-manifest-nofallback-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let status = identity_config(&path, "CABC").verify(0);
        assert!(!status.is_verified());
        assert!(status.mutations_disabled());
    }

    #[test]
    fn config_reads_contract_identity_from_env() {
        let vars = HashMap::from([
            (
                String::from(MANIFEST_PATH_ENV),
                String::from("/etc/predifi/manifest.json"),
            ),
            (String::from(CONTRACT_ID_ENV), String::from("CABC")),
            (
                String::from(NETWORK_PASSPHRASE_ENV),
                String::from("Test SDF Network ; September 2015"),
            ),
            (
                String::from(CONTRACT_INTERFACE_VERSION_ENV),
                String::from("1"),
            ),
            (
                String::from(MANIFEST_MAX_AGE_ENV),
                String::from("3600"),
            ),
        ]);
        let config = Config::from_map(&vars).unwrap();
        assert_eq!(config.contract_manifest_path, "/etc/predifi/manifest.json");
        assert_eq!(config.contract_id, "CABC");
        assert_eq!(config.contract_interface_version, 1);
        assert_eq!(config.contract_manifest_max_age_secs, 3600);
    }

    #[test]
    fn config_rejects_unsupported_contract_interface_version() {
        let vars = HashMap::from([(
            String::from(CONTRACT_INTERFACE_VERSION_ENV),
            String::from("42"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: CONTRACT_INTERFACE_VERSION_ENV,
                ..
            })
        ));
    }

    #[test]
    fn db_budget_accepts_exact_limit() {
        // 9 replicas x 10 = 90 == 100 - 10 available.
        let config = Config {
            db_server_max_connections: 100,
            db_admin_headroom_connections: 10,
            db_max_replicas: 9,
            db_pool_size_per_replica: 10,
            db_auxiliary_services: 0,
            ..Config::default_for_test()
        };
        config
            .validate()
            .expect("configuration exactly at the limit must be accepted");
    }

    #[test]
    fn db_budget_rejects_headroom_covering_whole_limit() {
        let config = Config {
            db_server_max_connections: 100,
            db_admin_headroom_connections: 100,
            ..Config::default_for_test()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_ADMIN_HEADROOM_CONNECTIONS",
                ..
            })
        ));
    }

    #[test]
    fn db_budget_rejects_zero_replicas() {
        let config = Config {
            db_max_replicas: 0,
            ..Config::default_for_test()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_MAX_REPLICAS",
                ..
            })
        ));
    }

    #[test]
    fn db_budget_rejects_zero_auxiliary_pool_when_services_configured() {
        let config = Config {
            db_auxiliary_services: 2,
            db_auxiliary_pool_size: 0,
            ..Config::default_for_test()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidValue {
                key: "PREDIFI_DB_AUXILIARY_POOL_SIZE",
                ..
            })
        ));
    }

    #[test]
    fn db_budget_rejects_non_numeric_replicas() {
        let vars = HashMap::from([(
            String::from("PREDIFI_DB_MAX_REPLICAS"),
            String::from("many"),
        )]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidNumber {
                key: "PREDIFI_DB_MAX_REPLICAS",
                ..
            })
        ));
    }

    #[test]
    fn config_rejects_empty_manifest_path() {
        let vars = HashMap::from([(String::from(MANIFEST_PATH_ENV), String::new())]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: MANIFEST_PATH_ENV,
                ..
            })
        ));
    }

    #[test]
    fn config_requires_contract_id_in_production() {
        let vars = HashMap::from([
            (String::from("PREDIFI_APP_ENV"), String::from("production")),
            (
                String::from("PREDIFI_SECRET_KEY"),
                String::from("a-very-strong-production-secret-key-at-least-32-bytes"),
            ),
            (
                String::from("PREDIFI_WS_ALLOWED_ORIGINS"),
                String::from("https://app.predifi.com"),
            ),
            (
                String::from("PREDIFI_CORS_ALLOWED_ORIGINS"),
                String::from("https://app.predifi.com"),
            ),
        ]);
        assert!(matches!(
            Config::from_map(&vars),
            Err(ConfigError::InvalidValue {
                key: CONTRACT_ID_ENV,
                ..
            })
        ));
    }

    #[test]
    fn notification_provider_ignores_blank_previous_key() {
        let vars = HashMap::from([
            (
                String::from("PREDIFI_NOTIFICATION_PROVIDER_URL"),
                String::from("https://notify.example.com"),
            ),
            (
                String::from("PREDIFI_NOTIFICATION_PROVIDER_KEY"),
                String::from("new-key"),
            ),
            (
                String::from("PREDIFI_NOTIFICATION_PROVIDER_PREVIOUS_KEY"),
                String::from("   "),
            ),
        ]);
        let config = Config::from_map(&vars).expect("blank previous key should be ignored");
        assert_eq!(config.notification_provider_previous_key, None);
    }
}
