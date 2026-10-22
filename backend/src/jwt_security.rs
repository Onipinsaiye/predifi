//! JWT security hardening: token revocation, refresh token rotation, and key versioning.
//!
//! This module provides:
//! - Token revocation lists (blacklist) backed by Redis
//! - Refresh token lifecycle management with automatic rotation
//! - Key version tracking to support secret rotation

use crate::constants::JWT_ACCESS_TOKEN_EXPIRY_SECS;
use crate::jwt::{sign_jwt_with_token_version, verify_jwt_token_strict, JwtVerifyError};
use crate::redis_cache::RedisCache;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Redis key prefix for revoked JWT IDs. Kept inside the cache namespace.
const REVOKED_JTI_PREFIX: &str = "jwt:revoked:";
const USER_REVOKE_BEFORE_PREFIX: &str = "jwt:user:revoke_before:";

/// Represents an issued token pair (access + refresh) with rotation metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPair {
    /// Access token (short-lived, 1 hour).
    pub access_token: String,
    /// Refresh token (long-lived, 7 days).
    pub refresh_token: String,
    /// Unix timestamp when the refresh token was issued.
    pub issued_at: u64,
    /// Key version at issuance; used to invalidate old tokens if secrets rotate.
    pub key_version: u64,
}

/// Issue a fresh access + refresh pair for `sub`.
pub fn issue_token_pair(
    sub: &str,
    secret: &str,
    now_unix: u64,
    key_version: u64,
) -> Result<TokenPair, String> {
    issue_token_pair_with_version(sub, secret, now_unix, key_version, 0)
}

/// Issue a fresh pair carrying the user's per-user token version (`tv`).
pub fn issue_token_pair_with_version(
    sub: &str,
    secret: &str,
    now_unix: u64,
    key_version: u64,
    token_version: u64,
) -> Result<TokenPair, String> {
    let access_token =
        sign_jwt_with_token_version(sub, secret, now_unix, "access", key_version, token_version)?;
    let refresh_token =
        sign_jwt_with_token_version(sub, secret, now_unix, "refresh", key_version, token_version)?;
    Ok(TokenPair {
        access_token,
        refresh_token,
        issued_at: now_unix,
        key_version,
    })
}

/// Rotate a refresh token: verify it, revoke the presented JTI, issue a new pair.
///
/// Fails closed if Redis is unavailable so a stolen refresh token cannot be
/// replayed after rotation without a durable revocation record.
pub async fn rotate_refresh_token(
    cache: &RedisCache,
    secret: &str,
    refresh_token: &str,
    current_key_version: u64,
    now_unix: u64,
) -> Result<TokenPair, JwtVerifyError> {
    let claims =
        verify_jwt_token_strict(refresh_token, secret, "refresh", current_key_version)?;

    if !cache.is_available() {
        return Err(JwtVerifyError::TokenStoreUnavailable);
    }

    if !claims.jti.is_empty() && is_token_revoked(cache, &claims.jti).await? {
        return Err(JwtVerifyError::Revoked);
    }

    if claims.iat > 0 && !is_token_valid_for_user(cache, &claims.sub, claims.iat).await? {
        return Err(JwtVerifyError::Revoked);
    }

    if !claims.jti.is_empty() {
        let remaining = claims.exp.saturating_sub(now_unix).max(1);
        revoke_token(cache, &claims.jti, remaining)
            .await
            .map_err(|_| JwtVerifyError::TokenStoreUnavailable)?;
    }

    issue_token_pair(&claims.sub, secret, now_unix, current_key_version)
        .map_err(JwtVerifyError::Decode)
}

/// Rotate a refresh token against the durable revocation store (#55).
///
/// * The presented token must not be revoked (per-jti or by user version).
/// * Its jti is revoked in Postgres before a new pair is issued, so the old
///   token fails on every replica.
/// * If the jti was already revoked by a concurrent or earlier rotation, the
///   token is being replayed. That signals theft, so every token of the user
///   is revoked and the request fails.
/// * Fails closed with `TokenStoreUnavailable` when the store is unreachable.
pub async fn rotate_refresh_token_durable(
    store: &crate::token_revocation::RevocationStore,
    secret: &str,
    refresh_token: &str,
    current_key_version: u64,
    now_unix: u64,
) -> Result<TokenPair, JwtVerifyError> {
    use crate::token_revocation::{RevocationReason, RevokeOutcome};

    let claims = verify_jwt_token_strict(refresh_token, secret, "refresh", current_key_version)?;
    let snapshot = store.ensure_not_revoked(&claims, now_unix).await?;

    let token_version = match store
        .revoke(&claims, RevocationReason::RefreshRotated, now_unix)
        .await?
    {
        RevokeOutcome::Revoked => snapshot.token_version,
        RevokeOutcome::EscalatedToUserWide { token_version } => token_version,
        RevokeOutcome::AlreadyRevoked => {
            tracing::warn!(user = %claims.sub, "refresh token reuse detected; revoking all sessions");
            store
                .revoke_all(&claims.sub, RevocationReason::RefreshReuseDetected, now_unix)
                .await?;
            return Err(JwtVerifyError::Revoked);
        }
    };

    issue_token_pair_with_version(&claims.sub, secret, now_unix, current_key_version, token_version)
        .map_err(JwtVerifyError::Decode)
}

/// Checks if a token (by JTI - JWT ID) has been revoked.
///
/// Revoked tokens are stored in Redis under the key pattern:
/// `jwt:revoked:{jti}` with a TTL matching the token's remaining expiry.
pub async fn is_token_revoked(
    cache: &RedisCache,
    token_jti: &str,
) -> Result<bool, JwtVerifyError> {
    if token_jti.is_empty() {
        return Ok(false);
    }

    cache
        .get_session_critical::<u8>(&revoked_key(token_jti))
        .await
        .map(|value| value.is_some())
        .map_err(|_| JwtVerifyError::TokenStoreUnavailable)
}

/// Revoke a token by storing its JTI in Redis.
///
/// The revocation entry expires after `ttl_secs`, matching the token's remaining lifetime.
pub async fn revoke_token(
    cache: &RedisCache,
    token_jti: &str,
    ttl_secs: u64,
) -> Result<(), String> {
    if token_jti.is_empty() {
        return Err("token jti must not be empty".to_string());
    }
    cache
        .set_session_critical(&revoked_key(token_jti), &1u8, ttl_secs)
        .await
        .map_err(|error| format!("session token store failed: {error:?}"))
}

/// Revoke all tokens for a user (on password change, logout, etc.).
///
/// This marks all tokens for `user_address` as revoked by setting a key:
/// `jwt:user:revoke_before:{user_address}` to the current Unix timestamp.
/// Tokens issued before this time are considered revoked.
pub async fn revoke_user_tokens(cache: &RedisCache, user_address: &str) -> Result<(), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    cache
        .set_session_critical(&user_revoke_key(user_address), &now, 86400 * 30)
        .await
        .map_err(|error| format!("session token store failed: {error:?}"))
}

/// Check if a token (issued at `issued_at`) is still valid for a user.
///
/// Returns `true` if the token was issued after the user's revoke cutoff.
/// If no revoke cutoff exists, the token is valid.
pub async fn is_token_valid_for_user(
    cache: &RedisCache,
    user_address: &str,
    issued_at: u64,
) -> Result<bool, JwtVerifyError> {
    cache
        .get_session_critical::<u64>(&user_revoke_key(user_address))
        .await
        .map(|revoke_before| revoke_before.map_or(true, |revoke_before| issued_at >= revoke_before))
        .map_err(|_| JwtVerifyError::TokenStoreUnavailable)
}

fn revoked_key(jti: &str) -> String {
    format!(
        "{REVOKED_JTI_PREFIX}{}",
        crate::redis_cache::sanitize_key_component(jti)
    )
}

fn user_revoke_key(user_address: &str) -> String {
    format!(
        "{USER_REVOKE_BEFORE_PREFIX}{}",
        crate::redis_cache::sanitize_key_component(user_address)
    )
}

/// Access-token lifetime advertised to API clients (`expires_in`).
pub fn access_token_expires_in() -> u64 {
    JWT_ACCESS_TOKEN_EXPIRY_SECS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jwt::{sign_jwt_with_type, verify_jwt_token};

    const TEST_SECRET: &str = "predifi-dev-secret-do-not-use-in-production-32";

    #[test]
    fn issue_token_pair_returns_distinct_typed_tokens() {
        let pair = issue_token_pair(crate::jwt::TEST_WALLET, TEST_SECRET, 1_800_000_000, 0).unwrap();
        let access = verify_jwt_token(&pair.access_token, TEST_SECRET).unwrap();
        let refresh = verify_jwt_token(&pair.refresh_token, TEST_SECRET).unwrap();
        assert_eq!(access.token_type, "access");
        assert_eq!(refresh.token_type, "refresh");
        assert_ne!(access.jti, refresh.jti);
        assert_eq!(pair.key_version, 0);
    }

    #[tokio::test]
    async fn rotate_refresh_token_fails_closed_without_redis() {
        let cache = RedisCache::disabled();
        let refresh =
            sign_jwt_with_type(crate::jwt::TEST_WALLET, TEST_SECRET, 1_800_000_000, "refresh", 0).unwrap();
        let error = rotate_refresh_token(&cache, TEST_SECRET, &refresh, 0, 1_800_000_100)
            .await
            .expect_err("redis required");
        assert_eq!(error, JwtVerifyError::TokenStoreUnavailable);
    }

    #[tokio::test]
    async fn token_state_checks_fail_closed_without_redis() {
        let cache = RedisCache::disabled();
        assert_eq!(
            is_token_revoked(&cache, "token-id").await,
            Err(JwtVerifyError::TokenStoreUnavailable)
        );
        assert_eq!(
            is_token_valid_for_user(&cache, "GABC123", 1_800_000_000).await,
            Err(JwtVerifyError::TokenStoreUnavailable)
        );
        assert!(revoke_token(&cache, "token-id", 60).await.is_err());
    }

    #[tokio::test]
    async fn rotate_refresh_token_rejects_access_token() {
        let cache = RedisCache::disabled();
        let access =
            sign_jwt_with_type(crate::jwt::TEST_WALLET, TEST_SECRET, 1_800_000_000, "access", 0).unwrap();
        let error = rotate_refresh_token(&cache, TEST_SECRET, &access, 0, 1_800_000_100)
            .await
            .expect_err("access token");
        assert_eq!(error, JwtVerifyError::WrongTokenType);
    }

    #[test]
    fn revoked_key_strips_injection_characters() {
        let key = revoked_key("abc\n:*def");
        assert!(!key.contains('\n'));
        assert!(!key.contains('*'));
        assert!(key.starts_with(REVOKED_JTI_PREFIX));
    }
}
