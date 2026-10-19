//! JWT security hardening: token revocation, refresh token rotation, and key versioning.
//!
//! This module provides:
//! - Token revocation lists (blacklist) backed by Redis
//! - Refresh token lifecycle management with automatic rotation
//! - Key version tracking to support secret rotation
//! - Strict wallet-bound claim validation (iss, aud, exp, subject format)

use crate::constants::JWT_ACCESS_TOKEN_EXPIRY_SECS;
use crate::jwt::{
    sign_jwt_with_type, verify_jwt_token, JwtVerifyError, Claims,
};
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
    let access_token = sign_jwt_with_type(sub, secret, now_unix, "access", key_version)?;
    let refresh_token = sign_jwt_with_type(sub, secret, now_unix, "refresh", key_version)?;
    Ok(TokenPair {
        access_token,
        refresh_token,
        issued_at: now_unix,
        key_version,
    })
}

/// Rotate a refresh token: verify it with strict claim checks, revoke the presented JTI, issue a new pair.
pub async fn rotate_refresh_token(
    cache: &RedisCache,
    secret: &str,
    refresh_token: &str,
    current_key_version: u64,
    now_unix: u64,
) -> Result<TokenPair, JwtVerifyError> {
    // Enforce strict claim validation and wallet-address format
    let claims = verify_jwt_token_strict(
        refresh_token,
        secret,
        "refresh",
        current_key_version,
    )?;

    if !cache.is_available() {
        return Err(JwtVerifyError::TokenStoreUnavailable);
    }

    if !claims.jti.is_empty() && is_token_revoked(cache, &claims.jti).await {
        return Err(JwtVerifyError::Revoked);
    }

    if claims.iat > 0 && !is_token_valid_for_user(cache, &claims.sub, claims.iat).await {
        return Err(JwtVerifyError::Revoked);
    }

    if !claims.jti.is_empty() {
        let remaining = claims.exp.saturating_sub(now_unix).max(1);
        let _ = revoke_token(cache, &claims.jti, remaining).await;
    }

    issue_token_pair(&claims.sub, secret, now_unix, current_key_version)
        .map_err(JwtVerifyError::Decode)
}

/// Strictly verify a JWT token enforcing required claims (iss, aud, exp, sub format, network context).
pub fn verify_jwt_token_strict(
    token: &str,
    secret: &str,
    expected_token_type: &str,
    expected_key_version: u64,
) -> Result<Claims, JwtVerifyError> {
    let claims = verify_jwt_token(token, secret)?;

    // 1. Validate Issuer
    if claims.iss != "predifi-auth" {
        return Err(JwtVerifyError::InvalidIssuer);
    }

    // 2. Validate Audience
    if claims.aud != "predifi-api" {
        return Err(JwtVerifyError::InvalidAudience);
    }

    // 3. Validate Token Type
    if claims.token_type != expected_token_type {
        return Err(JwtVerifyError::WrongTokenType);
    }

    // 4. Validate Key Version
    if claims.key_version != expected_key_version {
        return Err(JwtVerifyError::Decode("Mismatched key version".to_string()));
    }

    // 5. Validate & Normalize Wallet Address Subject Format (Stellar G-address: 56 chars, starts with 'G')
    let normalized_sub = normalize_wallet_address(&claims.sub)?;
    let mut validated_claims = claims;
    validated_claims.sub = normalized_sub;

    Ok(validated_claims)
}

/// Normalizes and validates a Stellar wallet address format.
fn normalize_wallet_address(sub: &str) -> Result<String, JwtVerifyError> {
    let trimmed = sub.trim();
    if trimmed.len() != 56 || !trimmed.starts_with('G') {
        return Err(JwtVerifyError::InvalidSubjectFormat);
    }
    if !trimmed.chars().all(|c| (c.is_ascii_alphabetic() && c.is_uppercase()) || c.is_ascii_digit()) {
        return Err(JwtVerifyError::InvalidSubjectFormat);
    }
    Ok(trimmed.to_string())
}

/// Checks if a token (by JTI - JWT ID) has been revoked.
pub async fn is_token_revoked(cache: &RedisCache, token_jti: &str) -> bool {
    if token_jti.is_empty() || !cache.is_available() {
        return false;
    }

    cache.exists(&revoked_key(token_jti)).await
}

/// Revoke a token by storing its JTI in Redis.
pub async fn revoke_token(
    cache: &RedisCache,
    token_jti: &str,
    ttl_secs: u64,
) -> Result<(), String> {
    if token_jti.is_empty() {
        return Err("token jti must not be empty".to_string());
    }
    if !cache.is_available() {
        return Err("Redis cache not available".to_string());
    }

    cache
        .set(&revoked_key(token_jti), &1u8, ttl_secs)
        .await;
    Ok(())
}

/// Revoke all tokens for a user (on password change, logout, etc.).
pub async fn revoke_user_tokens(cache: &RedisCache, user_address: &str) -> Result<(), String> {
    if !cache.is_available() {
        return Err("Redis cache not available".to_string());
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    cache
        .set(&user_revoke_key(user_address), &now, 86400 * 30)
        .await;
    Ok(())
}

/// Check if a token (issued at `issued_at`) is still valid for a user.
pub async fn is_token_valid_for_user(
    cache: &RedisCache,
    user_address: &str,
    issued_at: u64,
) -> bool {
    if !cache.is_available() {
        return true;
    }

    if let Some(revoke_before) = cache.get::<u64>(&user_revoke_key(user_address)).await {
        return issued_at >= revoke_before;
    }
    true
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

    const TEST_SECRET: &str = "predifi-dev-secret-do-not-use-in-production-32";
    const VALID_WALLET: &str = "GABC1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890ABCDEF";

    #[test]
    fn issue_token_pair_returns_distinct_typed_tokens() {
        let pair = issue_token_pair(VALID_WALLET, TEST_SECRET, 1_800_000_000, 0).unwrap();
        let access = verify_jwt_token_strict(&pair.access_token, TEST_SECRET, "access", 0).unwrap();
        let refresh = verify_jwt_token_strict(&pair.refresh_token, TEST_SECRET, "refresh", 0).unwrap();
        assert_eq!(access.token_type, "access");
        assert_eq!(refresh.token_type, "refresh");
        assert_ne!(access.jti, refresh.jti);
        assert_eq!(pair.key_version, 0);
    }

    #[tokio::test]
    async fn rotate_refresh_token_fails_closed_without_redis() {
        let cache = RedisCache::disabled();
        let pair = issue_token_pair(VALID_WALLET, TEST_SECRET, 1_800_000_000, 0).unwrap();
        let error = rotate_refresh_token(&cache, TEST_SECRET, &pair.refresh_token, 0, 1_800_000_100)
            .await
            .expect_err("redis required");
        assert_eq!(error, JwtVerifyError::TokenStoreUnavailable);
    }

    #[tokio::test]
    async fn rotate_refresh_token_rejects_access_token() {
        let cache = RedisCache::disabled();
        let pair = issue_token_pair(VALID_WALLET, TEST_SECRET, 1_800_000_000, 0).unwrap();
        let error = rotate_refresh_token(&cache, TEST_SECRET, &pair.access_token, 0, 1_800_000_100)
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

    #[test]
    fn test_negative_invalid_subject_format() {
        // Too short / doesn't start with G
        let invalid_token = sign_jwt_with_type("SHORT_WALLET", TEST_SECRET, 1_800_000_000, "access", 0).unwrap();
        let err = verify_jwt_token_strict(&invalid_token, TEST_SECRET, "access", 0).unwrap_err();
        assert_eq!(err, JwtVerifyError::InvalidSubjectFormat);
    }
        }
