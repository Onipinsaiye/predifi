//! JWT format validation and secret-backed token verification.
//!
//! [`validate_jwt_format`] performs a lightweight structural check before any
//! cryptographic work. [`verify_jwt_secret`] validates that the configured
//! signing secret meets minimum security requirements. [`verify_jwt_token`]
//! combines both checks and verifies the token signature and claims.
//!
//! Algorithm pinning: verification always uses HMAC-SHA256. The token `alg`
//! header is never trusted, which blocks `alg:none` and RS256/HS256 confusion.

use crate::constants::{
    JWT_ACCESS_TOKEN_EXPIRY_SECS, JWT_MIN_LENGTH, JWT_PARTS_COUNT, JWT_REFRESH_TOKEN_EXPIRY_SECS,
    JWT_SECRET_MIN_LENGTH,
};
use jsonwebtoken::{decode, decode_header, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

/// Clock-skew leeway applied to `exp` (seconds).
pub const JWT_EXP_LEEWAY_SECS: u64 = 30;
pub const JWT_ISSUER: &str = "predifi-api";
pub const JWT_AUDIENCE: &str = "predifi-client";
pub(crate) const TEST_WALLET: &str = "GAAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQDZ7H";

/// Maximum age, in seconds, of a step-up wallet proof accepted for a
/// privileged moderation action. Proofs older than this must be re-obtained.
pub const JWT_STEP_UP_MAX_AGE_SECS: u64 = 120;

/// Errors returned when a token string fails the structural JWT check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JwtFormatError {
    /// The token is too short to be a plausible JWT.
    TooShort,
    /// The token does not have exactly three dot-separated parts.
    WrongPartCount {
        /// How many parts were actually found.
        found: usize,
    },
    /// One of the three parts is empty.
    EmptyPart {
        /// Zero-based index of the empty part (0 = header, 1 = payload, 2 = signature).
        index: usize,
    },
    /// A part contains characters that are not valid base64url.
    InvalidBase64Url {
        /// Zero-based index of the offending part.
        index: usize,
    },
}

impl std::fmt::Display for JwtFormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "token is too short to be a valid JWT"),
            Self::WrongPartCount { found } => write!(
                f,
                "JWT must have exactly {JWT_PARTS_COUNT} parts separated by '.', found {found}"
            ),
            Self::EmptyPart { index } => {
                write!(f, "JWT part {index} (0-indexed) must not be empty")
            }
            Self::InvalidBase64Url { index } => {
                write!(f, "JWT part {index} contains invalid base64url characters")
            }
        }
    }
}

impl std::error::Error for JwtFormatError {}

/// Errors returned when the configured JWT signing secret is unusable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JwtSecretError {
    /// The secret is empty.
    Empty,
    /// The secret is shorter than [`JWT_SECRET_MIN_LENGTH`].
    TooShort {
        /// Minimum required length in bytes.
        min_length: usize,
    },
}

impl std::fmt::Display for JwtSecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "JWT signing secret must not be empty"),
            Self::TooShort { min_length } => {
                write!(f, "JWT signing secret must be at least {min_length} bytes")
            }
        }
    }
}

impl std::error::Error for JwtSecretError {}

/// Errors returned when JWT verification fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JwtVerifyError {
    /// The configured signing secret is invalid.
    InvalidSecret(JwtSecretError),
    /// The token failed the structural format check.
    InvalidFormat(JwtFormatError),
    /// The token signature does not match the configured secret.
    InvalidSignature,
    /// The token has expired.
    Expired,
    /// The `token_type` claim does not match the expected type.
    WrongTokenType,
    /// The token was issued under a different signing-key version.
    KeyVersionMismatch,
    /// The token has been revoked (logout, rotation, or user-wide revoke).
    Revoked,
    /// Refresh-token rotation requires a reachable token store (Redis).
    TokenStoreUnavailable,
    /// The token could not be decoded.
    Decode(String),
}

impl std::fmt::Display for JwtVerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSecret(error) => write!(f, "invalid JWT secret: {error}"),
            Self::InvalidFormat(error) => write!(f, "invalid JWT format: {error}"),
            Self::InvalidSignature => write!(f, "JWT signature verification failed"),
            Self::Expired => write!(f, "JWT has expired"),
            Self::WrongTokenType => write!(f, "JWT token type is not valid for this endpoint"),
            Self::KeyVersionMismatch => write!(f, "JWT was issued under a different key version"),
            Self::Revoked => write!(f, "JWT has been revoked"),
            Self::TokenStoreUnavailable => {
                write!(f, "token store unavailable; cannot rotate refresh tokens")
            }
            Self::Decode(message) => write!(f, "failed to decode JWT: {message}"),
        }
    }
}

impl std::error::Error for JwtVerifyError {}

/// Claims extracted from a verified PrediFi JWT.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PredifiClaims {
    /// Required token issuer.
    pub iss: String,
    /// Required token audience.
    pub aud: String,
    /// Required chain identifier.
    pub chain: String,
    /// Required Stellar network (`testnet`, `futurenet`, or `public`).
    pub network: String,
    /// Subject — the authenticated user's wallet address.
    pub sub: String,
    /// Expiration time as a Unix timestamp (seconds since epoch).
    /// Required for all issued tokens; validated on every call to
    /// `verify_jwt_token`.
    pub exp: u64,
    /// Issued-at time as a Unix timestamp. Used for user-wide revocation.
    #[serde(default)]
    pub iat: u64,
    /// Unique token identifier used for revocation and refresh rotation.
    #[serde(default)]
    pub jti: String,
    /// Token type: "access" or "refresh".
    /// Prevents accidental use of a refresh token in place of an access token.
    #[serde(default = "default_token_type")]
    pub token_type: String,
    /// Key version at token issuance. Bumping key_version on the server
    /// invalidates all tokens issued under previous versions (rotation support).
    #[serde(default = "default_key_version")]
    pub key_version: u64,
    /// Per-user token version at issuance. Bumping the user's version in the
    /// revocation store (logout-all, compromise) revokes every token carrying
    /// a lower value. Tokens issued before this claim existed decode as `0`.
    #[serde(default)]
    pub tv: u64,
    /// Unix timestamp of the most recent fresh wallet re-authentication
    /// (step-up). `0` when the token carries no step-up proof.
    #[serde(default)]
    pub step_up_at: u64,
    /// Privileged action the step-up proof is bound to, for example
    /// `moderation:hide_content`. Empty when no step-up proof is present.
    #[serde(default)]
    pub step_up_action: String,
}

fn default_token_type() -> String {
    "access".to_string()
}

fn default_key_version() -> u64 {
    0
}

/// Validate that the configured JWT signing secret meets minimum requirements.
pub fn verify_jwt_secret(secret: &str) -> Result<(), JwtSecretError> {
    if secret.is_empty() {
        return Err(JwtSecretError::Empty);
    }
    if secret.len() < JWT_SECRET_MIN_LENGTH {
        return Err(JwtSecretError::TooShort {
            min_length: JWT_SECRET_MIN_LENGTH,
        });
    }
    Ok(())
}

/// Validate that `token` matches the structural JWT format.
pub fn validate_jwt_format(token: &str) -> Result<(), JwtFormatError> {
    if token.len() < JWT_MIN_LENGTH {
        return Err(JwtFormatError::TooShort);
    }

    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != JWT_PARTS_COUNT {
        return Err(JwtFormatError::WrongPartCount { found: parts.len() });
    }

    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            return Err(JwtFormatError::EmptyPart { index });
        }

        if !part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '=')
        {
            return Err(JwtFormatError::InvalidBase64Url { index });
        }
    }

    Ok(())
}

/// Returns true when `claims` carries a step-up proof bound to `action` whose
/// timestamp is no later than `now` and no older than
/// [`JWT_STEP_UP_MAX_AGE_SECS`]. Stale, future-dated, unbound, or absent
/// proofs are rejected.
pub fn has_fresh_step_up(claims: &PredifiClaims, action: &str, now: u64) -> bool {
    !action.is_empty()
        && claims.step_up_action == action
        && claims.step_up_at != 0
        && claims.step_up_at <= now
        && now - claims.step_up_at <= JWT_STEP_UP_MAX_AGE_SECS
}

/// Build a validation config that pins HS256 and always enforces `exp`.
fn hs256_validation() -> Validation {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    // `iss`/`aud`/`chain`/`network` are checked explicitly against our own
    // expected values just below in `verify_jwt_token`. jsonwebtoken's
    // built-in `aud` check is disabled here because, with no expected `aud`
    // configured, it rejects *any* token that carries an `aud` claim at
    // all (`InvalidAudience`) rather than leaving it unchecked.
    validation.validate_aud = false;
    validation.leeway = JWT_EXP_LEEWAY_SECS;
    validation.required_spec_claims.insert("exp".to_string());
    validation
}

/// Verify a JWT against the configured HMAC secret and return decoded claims.
///
/// This function:
/// - Validates the JWT secret
/// - Checks format (3 dot-separated base64url parts)
/// - Rejects any `alg` other than HS256 (`alg:none`, RS256 confusion, etc.)
/// - Verifies the HMAC-SHA256 signature
/// - Checks expiration (`exp` is required)
/// - Validates non-empty subject claim
///
/// **Algorithm Confusion Prevention:**
/// The `Algorithm::HS256` is hard-coded and NEVER negotiated based on the token's
/// `alg` header. The header is inspected first and any mismatch is rejected
/// before signature verification.
pub fn verify_jwt_token(token: &str, secret: &str) -> Result<PredifiClaims, JwtVerifyError> {
    verify_jwt_secret(secret).map_err(JwtVerifyError::InvalidSecret)?;
    validate_jwt_format(token).map_err(JwtVerifyError::InvalidFormat)?;

    let header = decode_header(token).map_err(|_| JwtVerifyError::InvalidSignature)?;
    if header.alg != Algorithm::HS256 {
        return Err(JwtVerifyError::InvalidSignature);
    }

    let token_data = decode::<PredifiClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &hs256_validation(),
    )
    .map_err(map_decode_error)?;

    let mut claims = token_data.claims;
    if claims.iss != JWT_ISSUER
        || claims.aud != JWT_AUDIENCE
        || claims.chain != "stellar"
        || claims.network != configured_stellar_network()
        || !matches!(claims.network.as_str(), "testnet" | "futurenet" | "public")
    {
        return Err(JwtVerifyError::Decode(
            "JWT issuer, audience, or chain context is invalid".to_string(),
        ));
    }

    let wallet = crate::validated_types::normalize_wallet_address(&claims.sub).map_err(|_| {
        JwtVerifyError::Decode("JWT subject must be a valid Stellar account address".to_string())
    })?;
    if !wallet.starts_with('G') {
        return Err(JwtVerifyError::Decode(
            "JWT subject must be a valid Stellar account address".to_string(),
        ));
    }
    claims.sub = wallet;

    Ok(claims)
}

/// Resolve the configured network name from explicit configuration or the
/// Stellar RPC URL. Unknown RPC hosts are treated as public network endpoints.
fn configured_stellar_network() -> String {
    if let Ok(network) = std::env::var("PREDIFI_STELLAR_NETWORK") {
        return network;
    }
    let rpc = std::env::var("PREDIFI_STELLAR_RPC_URL")
        .unwrap_or_else(|_| "https://soroban-testnet.stellar.org".to_string());
    crate::config::stellar_network_for_rpc_url(&rpc).to_string()
}

/// Verify a JWT and validate it matches the expected token type and key version.
///
/// This is the stricter validation function for production use.
/// It ensures:
/// - Token is valid (signature, expiration, format)
/// - Token type matches expected (prevents refresh token misuse as access token)
/// - Key version matches current server version (allows key rotation)
pub fn verify_jwt_token_strict(
    token: &str,
    secret: &str,
    expected_type: &str,
    current_key_version: u64,
) -> Result<PredifiClaims, JwtVerifyError> {
    let claims = verify_jwt_token(token, secret)?;

    if claims.token_type != expected_type {
        return Err(JwtVerifyError::WrongTokenType);
    }

    if claims.key_version != current_key_version {
        return Err(JwtVerifyError::KeyVersionMismatch);
    }

    Ok(claims)
}

/// Extract the bearer token from an `Authorization` header value.
///
/// Accepts the standard `Bearer <token>` form (case-insensitive scheme),
/// trims surrounding whitespace, and returns `None` for anything else
/// (missing scheme, empty token, wrong scheme).
pub fn extract_bearer_token(value: &str) -> Option<&str> {
    let value = value.trim();
    let rest = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .or_else(|| value.strip_prefix("BEARER "))?;
    let rest = rest.trim();
    if rest.is_empty() {
        None
    } else {
        Some(rest)
    }
}

/// Sign a new PrediFi JWT carrying an explicit per-user token version (`tv`).
///
/// `token_type` is either `"access"` or `"refresh"`, which selects the
/// expiry window ([`JWT_ACCESS_TOKEN_EXPIRY_SECS`] /
/// [`JWT_REFRESH_TOKEN_EXPIRY_SECS`]). Always signs with HS256.
pub fn sign_jwt_with_token_version(
    sub: &str,
    secret: &str,
    now_unix: u64,
    token_type: &str,
    key_version: u64,
    token_version: u64,
) -> Result<String, String> {
    verify_jwt_secret(secret).map_err(|e| e.to_string())?;

    let expiry_secs = if token_type == "refresh" {
        JWT_REFRESH_TOKEN_EXPIRY_SECS
    } else {
        JWT_ACCESS_TOKEN_EXPIRY_SECS
    };

    let claims = PredifiClaims {
        iss: JWT_ISSUER.to_string(),
        aud: JWT_AUDIENCE.to_string(),
        chain: "stellar".to_string(),
        network: configured_stellar_network(),
        sub: sub.to_string(),
        exp: now_unix.saturating_add(expiry_secs),
        iat: now_unix,
        jti: uuid::Uuid::new_v4().to_string(),
        token_type: token_type.to_string(),
        key_version,
        tv: token_version,
        step_up_at: 0,
        step_up_action: String::new(),
    };

    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| e.to_string())
}

/// Sign a token for `sub` with an explicit type but no per-user token
/// version (`tv` defaults to `0`). Thin convenience wrapper over
/// [`sign_jwt_with_token_version`] used by other modules' tests.
pub fn sign_jwt_with_type(
    sub: &str,
    secret: &str,
    now_unix: u64,
    token_type: &str,
    key_version: u64,
) -> Result<String, String> {
    sign_jwt_with_token_version(sub, secret, now_unix, token_type, key_version, 0)
}

/// Test-only convenience: sign an access token for `wallet` with sensible
/// defaults (key version 0, a fixed far-future-relative timestamp so tokens
/// are valid for the lifetime of a test run).
pub fn sign_jwt_for_test(wallet: &str, secret: &str) -> String {
    sign_jwt_with_type(wallet, secret, 1_800_000_000, "access", 0)
        .expect("sign_jwt_for_test: signing must not fail for a valid test secret")
}

fn map_decode_error(error: jsonwebtoken::errors::Error) -> JwtVerifyError {
    match error.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => JwtVerifyError::Expired,
        _ => JwtVerifyError::InvalidSignature,
    }
}
