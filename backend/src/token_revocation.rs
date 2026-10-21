//! Durable session revocation and per-user token versions (#55).
//!
//! JWTs stay valid until `exp` unless the server can say otherwise. This
//! module gives the server that ability, consistently across replicas.
//!
//! ## State
//!
//! * **Per-user token version** (`user_token_versions`). Every token carries
//!   the user's version at issuance in its `tv` claim. "Log out everywhere"
//!   or a suspected compromise bumps the version; any token with
//!   `tv < current` is revoked. Versions only increase and never expire, so a
//!   revoked token can never become valid again.
//! * **Single-token revocations** (`revoked_tokens`). Logout of one session
//!   and refresh-token rotation revoke one `jti`. Each record lives until the
//!   token's `exp` plus [`RECORD_GRACE_SECS`] (the verifier's clock-skew
//!   leeway plus a margin). After that the token fails the expiry check on
//!   its own, so purging the record cannot revive it.
//!
//! ## Storage and outage behaviour
//!
//! Postgres is the source of truth. Redis caches one snapshot per user
//! (`version + active revoked jtis`) for [`SNAPSHOT_TTL_SECS`].
//!
//! * Writes go to Postgres first, then the user's snapshot is rebuilt from
//!   Postgres and written to Redis, so every replica sees the revocation on
//!   its next check. If Redis cannot be updated, other replicas may serve the
//!   old snapshot for at most [`SNAPSHOT_TTL_SECS`].
//! * Reads use the Redis snapshot; on a miss or Redis error they read
//!   Postgres. If Postgres is unavailable too (or not configured), the check
//!   **fails closed** with [`JwtVerifyError::TokenStoreUnavailable`], which
//!   the API returns as `503 TOKEN_STORE_UNAVAILABLE` (retryable).
//!
//! ## Bounded hot path
//!
//! A check costs one Redis `GET`, or on a miss two indexed Postgres queries.
//! A snapshot holds at most [`MAX_ACTIVE_REVOCATIONS_PER_USER`] jtis: when a
//! user would exceed that, the store bumps the user's version instead (one
//! revocation covering all tokens) and drops the per-jti rows.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::jwt::{JwtVerifyError, PredifiClaims, JWT_EXP_LEEWAY_SECS};
use crate::redis_cache::{sanitize_key_component, RedisCache};

/// Redis key prefix for cached per-user revocation snapshots.
const SNAPSHOT_PREFIX: &str = "jwt:user:state:";

/// How long a cached snapshot is trusted before Postgres is read again. This
/// is the worst-case cross-replica propagation delay when Redis could not be
/// updated during a revocation.
pub const SNAPSHOT_TTL_SECS: u64 = 30;

/// Extra lifetime of a single-token revocation record past the token's `exp`.
pub const RECORD_GRACE_SECS: u64 = JWT_EXP_LEEWAY_SECS + 60;

/// Most single-token revocations kept per user before escalating to a
/// user-wide version bump.
pub const MAX_ACTIVE_REVOCATIONS_PER_USER: usize = 64;

/// Why a token was revoked (stored for audit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevocationReason {
    Logout,
    RefreshRotated,
    RefreshReuseDetected,
    Compromised,
}

impl RevocationReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Logout => "logout",
            Self::RefreshRotated => "refresh_rotated",
            Self::RefreshReuseDetected => "refresh_reuse_detected",
            Self::Compromised => "compromised",
        }
    }
}

/// Result of revoking a single token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevokeOutcome {
    /// A new revocation record was written.
    Revoked,
    /// The token was already revoked (e.g. a replayed refresh token).
    AlreadyRevoked,
    /// The per-user cap was reached, so the user's token version was bumped.
    EscalatedToUserWide { token_version: u64 },
}

/// Errors from the revocation store.
#[derive(Debug)]
pub enum RevocationError {
    /// No database configured; revocation state cannot be persisted or read.
    NotConfigured,
    Database(sqlx::Error),
}

impl std::fmt::Display for RevocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "revocation store is not configured"),
            Self::Database(e) => write!(f, "revocation store database error: {e}"),
        }
    }
}

impl std::error::Error for RevocationError {}

impl From<sqlx::Error> for RevocationError {
    fn from(e: sqlx::Error) -> Self {
        Self::Database(e)
    }
}

impl From<RevocationError> for JwtVerifyError {
    fn from(e: RevocationError) -> Self {
        tracing::warn!(error = %e, "token revocation store unavailable; failing closed");
        JwtVerifyError::TokenStoreUnavailable
    }
}

/// One revoked token inside a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokedJti {
    pub jti: String,
    /// Unix seconds after which the record (and the token) is irrelevant.
    pub expires_at: u64,
}

/// Everything needed to decide whether a user's token is revoked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationSnapshot {
    pub token_version: u64,
    pub revoked: Vec<RevokedJti>,
}

impl RevocationSnapshot {
    /// Whether `claims` are revoked according to this snapshot.
    pub fn is_revoked(&self, claims: &PredifiClaims) -> bool {
        claims.tv < self.token_version
            || (!claims.jti.is_empty() && self.revoked.iter().any(|r| r.jti == claims.jti))
    }
}

/// Shared revocation store: Postgres for durability, Redis as a cache.
#[derive(Clone)]
pub struct RevocationStore {
    db: Option<PgPool>,
    redis: RedisCache,
}

impl RevocationStore {
    pub fn new(db: Option<PgPool>, redis: RedisCache) -> Self {
        Self { db, redis }
    }

    fn db(&self) -> Result<&PgPool, RevocationError> {
        self.db.as_ref().ok_or(RevocationError::NotConfigured)
    }

    /// Fail with `Revoked` if `claims` were revoked, or with
    /// `TokenStoreUnavailable` if revocation state cannot be read.
    ///
    /// Returns the snapshot so callers can reuse the current token version.
    pub async fn ensure_not_revoked(
        &self,
        claims: &PredifiClaims,
        now_unix: u64,
    ) -> Result<RevocationSnapshot, JwtVerifyError> {
        let snapshot = self.snapshot(&claims.sub, now_unix).await?;
        if snapshot.is_revoked(claims) {
            return Err(JwtVerifyError::Revoked);
        }
        Ok(snapshot)
    }

    /// Current token version for `user`, used when issuing tokens.
    pub async fn current_token_version(
        &self,
        user: &str,
        now_unix: u64,
    ) -> Result<u64, RevocationError> {
        Ok(self.snapshot(user, now_unix).await?.token_version)
    }

    /// Read the user's snapshot from Redis, falling back to Postgres.
    pub async fn snapshot(
        &self,
        user: &str,
        now_unix: u64,
    ) -> Result<RevocationSnapshot, RevocationError> {
        if let Some(cached) = self
            .redis
            .get::<RevocationSnapshot>(&snapshot_key(user))
            .await
        {
            return Ok(cached);
        }
        let snapshot = self.load(user, now_unix).await?;
        self.redis
            .set(&snapshot_key(user), &snapshot, SNAPSHOT_TTL_SECS)
            .await;
        Ok(snapshot)
    }

    /// Revoke the single token described by `claims`.
    pub async fn revoke(
        &self,
        claims: &PredifiClaims,
        reason: RevocationReason,
        now_unix: u64,
    ) -> Result<RevokeOutcome, RevocationError> {
        let db = self.db()?;
        if claims.jti.is_empty() {
            // Legacy token without a jti: only a version bump can revoke it.
            let token_version = self.revoke_all(&claims.sub, reason, now_unix).await?;
            return Ok(RevokeOutcome::EscalatedToUserWide { token_version });
        }

        let (active,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM revoked_tokens \
             WHERE user_address = $1 AND expires_at > to_timestamp($2)",
        )
        .bind(&claims.sub)
        .bind(now_unix as f64)
        .fetch_one(db)
        .await?;

        if active as usize >= MAX_ACTIVE_REVOCATIONS_PER_USER {
            let token_version = self.revoke_all(&claims.sub, reason, now_unix).await?;
            return Ok(RevokeOutcome::EscalatedToUserWide { token_version });
        }

        let expires_at = claims.exp.max(now_unix).saturating_add(RECORD_GRACE_SECS);
        let inserted = sqlx::query(
            "INSERT INTO revoked_tokens (jti, user_address, expires_at, reason) \
             VALUES ($1, $2, to_timestamp($3), $4) \
             ON CONFLICT (jti) DO NOTHING",
        )
        .bind(&claims.jti)
        .bind(&claims.sub)
        .bind(expires_at as f64)
        .bind(reason.as_str())
        .execute(db)
        .await?
        .rows_affected();

        self.refresh_cache(&claims.sub, now_unix).await;
        tracing::info!(
            user = %claims.sub,
            jti = %claims.jti,
            reason = reason.as_str(),
            "token revoked"
        );

        Ok(if inserted == 0 {
            RevokeOutcome::AlreadyRevoked
        } else {
            RevokeOutcome::Revoked
        })
    }

    /// Revoke every token issued to `user` so far by bumping their version.
    /// Returns the new version, which newly issued tokens must carry.
    pub async fn revoke_all(
        &self,
        user: &str,
        reason: RevocationReason,
        now_unix: u64,
    ) -> Result<u64, RevocationError> {
        let db = self.db()?;
        let mut tx = db.begin().await?;
        let (version,): (i64,) = sqlx::query_as(
            "INSERT INTO user_token_versions (user_address, token_version) VALUES ($1, 1) \
             ON CONFLICT (user_address) DO UPDATE \
             SET token_version = user_token_versions.token_version + 1, updated_at = NOW() \
             RETURNING token_version",
        )
        .bind(user)
        .fetch_one(&mut *tx)
        .await?;
        // Every existing token has tv below the new version, so per-jti rows
        // for this user are redundant now.
        sqlx::query("DELETE FROM revoked_tokens WHERE user_address = $1")
            .bind(user)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;

        self.refresh_cache(user, now_unix).await;
        tracing::info!(user = %user, token_version = version, reason = reason.as_str(), "all user tokens revoked");
        Ok(version.max(0) as u64)
    }

    /// Delete revocation records whose tokens have expired. Safe to run on
    /// any replica at any time.
    pub async fn purge_expired(&self, now_unix: u64) -> Result<u64, RevocationError> {
        let db = self.db()?;
        let deleted = sqlx::query("DELETE FROM revoked_tokens WHERE expires_at <= to_timestamp($1)")
            .bind(now_unix as f64)
            .execute(db)
            .await?
            .rows_affected();
        Ok(deleted)
    }

    async fn load(&self, user: &str, now_unix: u64) -> Result<RevocationSnapshot, RevocationError> {
        let db = self.db()?;
        let version: Option<(i64,)> =
            sqlx::query_as("SELECT token_version FROM user_token_versions WHERE user_address = $1")
                .bind(user)
                .fetch_optional(db)
                .await?;
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT jti, EXTRACT(EPOCH FROM expires_at)::BIGINT FROM revoked_tokens \
             WHERE user_address = $1 AND expires_at > to_timestamp($2) \
             ORDER BY expires_at DESC LIMIT $3",
        )
        .bind(user)
        .bind(now_unix as f64)
        .bind(MAX_ACTIVE_REVOCATIONS_PER_USER as i64)
        .fetch_all(db)
        .await?;

        Ok(RevocationSnapshot {
            token_version: version.map(|(v,)| v.max(0) as u64).unwrap_or(0),
            revoked: rows
                .into_iter()
                .map(|(jti, expires_at)| RevokedJti {
                    jti,
                    expires_at: expires_at.max(0) as u64,
                })
                .collect(),
        })
    }

    /// Rebuild the cached snapshot from Postgres after a write. Failures only
    /// widen the propagation window to [`SNAPSHOT_TTL_SECS`].
    async fn refresh_cache(&self, user: &str, now_unix: u64) {
        match self.load(user, now_unix).await {
            Ok(snapshot) => {
                self.redis
                    .set(&snapshot_key(user), &snapshot, SNAPSHOT_TTL_SECS)
                    .await
            }
            Err(error) => {
                tracing::warn!(user = %user, error = %error, "could not refresh revocation cache");
                self.redis.delete(&snapshot_key(user)).await;
            }
        }
    }
}

fn snapshot_key(user: &str) -> String {
    format!("{SNAPSHOT_PREFIX}{}", sanitize_key_component(user))
}

/// Purge expired revocation records every `interval` until the task is
/// aborted.
pub async fn run_purge_loop(store: RevocationStore, interval: std::time::Duration) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        match store.purge_expired(now).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(purged = n, "purged expired token revocation records"),
            Err(error) => tracing::warn!(error = %error, "token revocation purge failed"),
        }
    }
}
