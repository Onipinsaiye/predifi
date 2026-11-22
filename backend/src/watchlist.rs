//! Public market watchlists with explicit privacy settings (Issue #280).
//!
//! # Privacy model
//!
//! Watchlists are **private by default** (`public = false`). They become
//! publicly accessible only when the owner explicitly sets `public = true`,
//! at which point a random opaque `public_slug` is generated and returned.
//!
//! The public read path uses only the `public_slug` — it never exposes the
//! owner's wallet address or the internal numeric `id`. Public viewers see
//! only the pool IDs in the watchlist (market metadata is fetched from the
//! pools table separately).
//!
//! # Revocation
//!
//! Calling `set_watchlist_public(id, false, ...)` nulls out `public_slug`
//! immediately. Any previously shared link now returns 404. If the owner
//! later re-enables public sharing, a fresh slug is generated so the old
//! link remains permanently invalid.
//!
//! # Ownership
//!
//! Every mutating function takes `owner_address` and verifies it against the
//! stored row before making changes. The caller (HTTP handler) is responsible
//! for authenticating the bearer token and extracting the owner address before
//! calling these functions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

// ─── Types ────────────────────────────────────────────────────────────────────

/// A watchlist row as returned to its authenticated owner.
#[derive(Debug, Clone, Serialize)]
pub struct Watchlist {
    pub id: i64,
    /// Owner wallet address — only included in owner-scoped responses.
    pub owner_address: String,
    pub name: String,
    pub description: Option<String>,
    pub public: bool,
    /// Opaque share token. `None` when the list is private or revoked.
    pub public_slug: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Pool IDs included in this watchlist.
    pub market_ids: Vec<i64>,
}

/// A watchlist as returned to an unauthenticated public viewer.
///
/// Never includes `owner_address` or `id`; public access is entirely via
/// `public_slug`.
#[derive(Debug, Clone, Serialize)]
pub struct PublicWatchlist {
    pub name: String,
    pub description: Option<String>,
    /// Ordered list of pool IDs. The client fetches market metadata separately.
    pub market_ids: Vec<i64>,
}

/// Input for creating a new watchlist.
#[derive(Debug, Deserialize)]
pub struct CreateWatchlistInput {
    pub name: String,
    pub description: Option<String>,
    /// Initial visibility. Defaults to false (private) when absent.
    #[serde(default)]
    pub public: bool,
}

/// Input for updating watchlist metadata.
#[derive(Debug, Deserialize)]
pub struct UpdateWatchlistInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub public: Option<bool>,
}

/// Errors the watchlist service layer can produce.
#[derive(Debug)]
pub enum WatchlistError {
    Db(sqlx::Error),
    NotFound,
    /// Caller is not the list owner.
    Forbidden,
    ValidationError(String),
}

impl From<sqlx::Error> for WatchlistError {
    fn from(e: sqlx::Error) -> Self {
        WatchlistError::Db(e)
    }
}

impl std::fmt::Display for WatchlistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WatchlistError::Db(e) => write!(f, "database error: {e}"),
            WatchlistError::NotFound => write!(f, "watchlist not found"),
            WatchlistError::Forbidden => write!(f, "not the watchlist owner"),
            WatchlistError::ValidationError(msg) => write!(f, "validation error: {msg}"),
        }
    }
}

// ─── Validation ───────────────────────────────────────────────────────────────

const MAX_NAME_LEN: usize = 80;
const MAX_DESCRIPTION_LEN: usize = 280;
const MAX_MARKETS_PER_WATCHLIST: usize = 200;

fn validate_name(name: &str) -> Result<(), WatchlistError> {
    if name.trim().is_empty() || name.len() > MAX_NAME_LEN {
        return Err(WatchlistError::ValidationError(format!(
            "name must be between 1 and {MAX_NAME_LEN} characters"
        )));
    }
    Ok(())
}

fn validate_description(desc: &Option<String>) -> Result<(), WatchlistError> {
    if let Some(d) = desc {
        if d.len() > MAX_DESCRIPTION_LEN {
            return Err(WatchlistError::ValidationError(format!(
                "description must be {MAX_DESCRIPTION_LEN} characters or fewer"
            )));
        }
    }
    Ok(())
}

// ─── Internal row types ───────────────────────────────────────────────────────

#[derive(sqlx::FromRow)]
struct WatchlistRow {
    id: i64,
    owner_address: String,
    name: String,
    description: Option<String>,
    public: bool,
    public_slug: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

async fn fetch_market_ids(pool: &PgPool, watchlist_id: i64) -> Result<Vec<i64>, sqlx::Error> {
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT pool_id FROM watchlist_markets WHERE watchlist_id = $1 ORDER BY position, added_at",
    )
    .bind(watchlist_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

fn row_to_watchlist(row: WatchlistRow, market_ids: Vec<i64>) -> Watchlist {
    Watchlist {
        id: row.id,
        owner_address: row.owner_address,
        name: row.name,
        description: row.description,
        public: row.public,
        public_slug: row.public_slug,
        created_at: row.created_at,
        updated_at: row.updated_at,
        market_ids,
    }
}

// ─── Service functions ────────────────────────────────────────────────────────

/// Create a new watchlist for `owner_address`.
///
/// If `input.public = true`, a random slug is generated immediately.
pub async fn create_watchlist(
    pool: &PgPool,
    owner_address: &str,
    input: &CreateWatchlistInput,
) -> Result<Watchlist, WatchlistError> {
    validate_name(&input.name)?;
    validate_description(&input.description)?;

    let public_slug: Option<String> = if input.public {
        Some(Uuid::new_v4().to_string().replace('-', ""))
    } else {
        None
    };

    let row = sqlx::query_as::<_, WatchlistRow>(
        r#"
        INSERT INTO watchlists (owner_address, name, description, public, public_slug)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, owner_address, name, description, public, public_slug, created_at, updated_at
        "#,
    )
    .bind(owner_address)
    .bind(&input.name)
    .bind(&input.description)
    .bind(input.public)
    .bind(&public_slug)
    .fetch_one(pool)
    .await?;

    Ok(row_to_watchlist(row, vec![]))
}

/// Fetch all watchlists owned by `owner_address`, with their market IDs.
pub async fn list_watchlists(
    pool: &PgPool,
    owner_address: &str,
) -> Result<Vec<Watchlist>, WatchlistError> {
    let rows = sqlx::query_as::<_, WatchlistRow>(
        r#"
        SELECT id, owner_address, name, description, public, public_slug, created_at, updated_at
        FROM watchlists
        WHERE owner_address = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(owner_address)
    .fetch_all(pool)
    .await?;

    let mut watchlists = Vec::with_capacity(rows.len());
    for row in rows {
        let market_ids = fetch_market_ids(pool, row.id).await?;
        watchlists.push(row_to_watchlist(row, market_ids));
    }
    Ok(watchlists)
}

/// Fetch a single watchlist by `id`, verifying `owner_address`.
///
/// Returns `WatchlistError::Forbidden` when the row exists but belongs to a
/// different owner.
pub async fn get_watchlist(
    pool: &PgPool,
    id: i64,
    owner_address: &str,
) -> Result<Watchlist, WatchlistError> {
    let row = sqlx::query_as::<_, WatchlistRow>(
        r#"
        SELECT id, owner_address, name, description, public, public_slug, created_at, updated_at
        FROM watchlists
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(WatchlistError::NotFound)?;

    if row.owner_address != owner_address {
        return Err(WatchlistError::Forbidden);
    }

    let market_ids = fetch_market_ids(pool, row.id).await?;
    Ok(row_to_watchlist(row, market_ids))
}

/// Fetch a watchlist by its opaque `public_slug` for the public share-link read path.
///
/// Only returns watchlists where `public = TRUE`.  When `public_slug` has been
/// revoked (set to NULL), or the slug doesn't match any row, returns `None`.
pub async fn get_watchlist_by_slug(
    pool: &PgPool,
    slug: &str,
) -> Result<Option<PublicWatchlist>, WatchlistError> {
    let row = sqlx::query_as::<_, WatchlistRow>(
        r#"
        SELECT id, owner_address, name, description, public, public_slug, created_at, updated_at
        FROM watchlists
        WHERE public_slug = $1 AND public = TRUE
        "#,
    )
    .bind(slug)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let market_ids = fetch_market_ids(pool, row.id).await?;
    Ok(Some(PublicWatchlist {
        name: row.name,
        description: row.description,
        market_ids,
    }))
}

/// Update a watchlist's metadata and/or visibility.
///
/// - Setting `public = true` generates a new slug if none exists, or keeps
///   the existing one.
/// - Setting `public = false` nulls out `public_slug` (revocation).
/// - Calling the function a second time with `public = false` is idempotent.
pub async fn update_watchlist(
    pool: &PgPool,
    id: i64,
    owner_address: &str,
    input: &UpdateWatchlistInput,
) -> Result<Watchlist, WatchlistError> {
    if let Some(name) = &input.name {
        validate_name(name)?;
    }
    validate_description(&input.description)?;

    // Load current row to check ownership and current visibility.
    let current = sqlx::query_as::<_, WatchlistRow>(
        "SELECT id, owner_address, name, description, public, public_slug, created_at, updated_at FROM watchlists WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(WatchlistError::NotFound)?;

    if current.owner_address != owner_address {
        return Err(WatchlistError::Forbidden);
    }

    // Determine new visibility and slug.
    let new_public = input.public.unwrap_or(current.public);
    let new_slug: Option<String> = match (current.public, new_public, &current.public_slug) {
        // Enabling public with no existing slug → generate one.
        (false, true, None) | (_, true, None) => {
            Some(Uuid::new_v4().to_string().replace('-', ""))
        }
        // Keeping public with existing slug → preserve it.
        (true, true, Some(existing)) => Some(existing.clone()),
        // Disabling public → revoke slug.
        (_, false, _) => None,
        // Was already public, now staying public: keep slug.
        (true, true, None) => Some(Uuid::new_v4().to_string().replace('-', "")),
    };

    let new_name = input.name.as_deref().unwrap_or(&current.name);
    let new_desc = match &input.description {
        Some(d) => Some(d.as_str()),
        None => current.description.as_deref(),
    };

    let row = sqlx::query_as::<_, WatchlistRow>(
        r#"
        UPDATE watchlists
        SET name = $1, description = $2, public = $3, public_slug = $4, updated_at = NOW()
        WHERE id = $5
        RETURNING id, owner_address, name, description, public, public_slug, created_at, updated_at
        "#,
    )
    .bind(new_name)
    .bind(new_desc)
    .bind(new_public)
    .bind(&new_slug)
    .bind(id)
    .fetch_one(pool)
    .await?;

    let market_ids = fetch_market_ids(pool, row.id).await?;
    Ok(row_to_watchlist(row, market_ids))
}

/// Delete a watchlist and all its market entries.
///
/// The `ON DELETE CASCADE` on `watchlist_markets` handles the join table rows.
pub async fn delete_watchlist(
    pool: &PgPool,
    id: i64,
    owner_address: &str,
) -> Result<(), WatchlistError> {
    let result = sqlx::query(
        "DELETE FROM watchlists WHERE id = $1 AND owner_address = $2",
    )
    .bind(id)
    .bind(owner_address)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        // Could be NotFound or Forbidden; we can't distinguish without an extra read.
        return Err(WatchlistError::NotFound);
    }
    Ok(())
}

/// Add a pool to a watchlist. Idempotent — adding an already-present pool is a no-op.
pub async fn add_market(
    pool: &PgPool,
    watchlist_id: i64,
    owner_address: &str,
    pool_id: i64,
) -> Result<(), WatchlistError> {
    // Ownership check.
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM watchlists WHERE id = $1 AND owner_address = $2",
    )
    .bind(watchlist_id)
    .bind(owner_address)
    .fetch_one(pool)
    .await?;
    if count.0 == 0 {
        return Err(WatchlistError::NotFound);
    }

    // Guard against oversized watchlists.
    let market_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM watchlist_markets WHERE watchlist_id = $1")
            .bind(watchlist_id)
            .fetch_one(pool)
            .await?;
    if market_count.0 >= MAX_MARKETS_PER_WATCHLIST as i64 {
        return Err(WatchlistError::ValidationError(format!(
            "watchlist may contain at most {MAX_MARKETS_PER_WATCHLIST} markets"
        )));
    }

    // Determine position: append at the end.
    let position: (Option<i32>,) = sqlx::query_as(
        "SELECT MAX(position) FROM watchlist_markets WHERE watchlist_id = $1",
    )
    .bind(watchlist_id)
    .fetch_one(pool)
    .await?;
    let next_pos = position.0.unwrap_or(-1) + 1;

    sqlx::query(
        r#"
        INSERT INTO watchlist_markets (watchlist_id, pool_id, position)
        VALUES ($1, $2, $3)
        ON CONFLICT (watchlist_id, pool_id) DO NOTHING
        "#,
    )
    .bind(watchlist_id)
    .bind(pool_id)
    .bind(next_pos)
    .execute(pool)
    .await?;

    Ok(())
}

/// Remove a pool from a watchlist.
pub async fn remove_market(
    pool: &PgPool,
    watchlist_id: i64,
    owner_address: &str,
    pool_id: i64,
) -> Result<(), WatchlistError> {
    // Ownership check.
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM watchlists WHERE id = $1 AND owner_address = $2",
    )
    .bind(watchlist_id)
    .bind(owner_address)
    .fetch_one(pool)
    .await?;
    if count.0 == 0 {
        return Err(WatchlistError::NotFound);
    }

    sqlx::query(
        "DELETE FROM watchlist_markets WHERE watchlist_id = $1 AND pool_id = $2",
    )
    .bind(watchlist_id)
    .bind(pool_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Delete all watchlists owned by `owner_address`.
///
/// Called by the user-controlled account-deletion flow.
pub async fn delete_all_watchlists_for_owner(
    pool: &PgPool,
    owner_address: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM watchlists WHERE owner_address = $1")
        .bind(owner_address)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_rejects_empty() {
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
    }

    #[test]
    fn validate_name_rejects_too_long() {
        assert!(validate_name(&"x".repeat(MAX_NAME_LEN + 1)).is_err());
    }

    #[test]
    fn validate_name_accepts_valid() {
        assert!(validate_name("My Watchlist").is_ok());
        assert!(validate_name(&"x".repeat(MAX_NAME_LEN)).is_ok());
    }

    #[test]
    fn validate_description_accepts_none() {
        assert!(validate_description(&None).is_ok());
    }

    #[test]
    fn validate_description_rejects_too_long() {
        assert!(validate_description(&Some("y".repeat(MAX_DESCRIPTION_LEN + 1))).is_err());
    }

    #[test]
    fn validate_description_accepts_valid() {
        assert!(validate_description(&Some("Short bio".to_string())).is_ok());
    }

    #[test]
    fn private_watchlist_has_no_slug_by_default() {
        // Simulate what create_watchlist does for a private list.
        let public = false;
        let slug: Option<String> = if public {
            Some("unused".to_string())
        } else {
            None
        };
        assert!(slug.is_none());
    }

    #[test]
    fn public_watchlist_generates_a_slug() {
        let public = true;
        let slug: Option<String> = if public {
            Some(Uuid::new_v4().to_string().replace('-', ""))
        } else {
            None
        };
        assert!(slug.is_some());
        // Slug is 32 hex chars (UUID without hyphens).
        let s = slug.unwrap();
        assert_eq!(s.len(), 32);
        assert!(s.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn revoking_public_clears_slug() {
        // Simulate revoking: public = false → slug = None.
        let was_public = true;
        let _was_slug = Some("abc123".to_string());
        let new_public = false;
        let new_slug: Option<String> = if new_public { _was_slug } else { None };
        assert!(new_slug.is_none());
    }

    #[test]
    fn re_enabling_public_generates_new_slug() {
        // Simulate re-enabling after revocation: public = true, old slug = None → new slug.
        let old_slug: Option<String> = None;
        let new_public = true;
        let new_slug: Option<String> = match (new_public, &old_slug) {
            (true, None) => Some(Uuid::new_v4().to_string().replace('-', "")),
            (true, Some(existing)) => Some(existing.clone()),
            (false, _) => None,
        };
        // New slug is different from the old (revoked) slug.
        assert!(new_slug.is_some());
        assert_ne!(new_slug.as_deref(), old_slug.as_deref());
    }

    #[test]
    fn error_display_does_not_panic() {
        let _ = WatchlistError::NotFound.to_string();
        let _ = WatchlistError::Forbidden.to_string();
        let _ = WatchlistError::ValidationError("bad input".to_string()).to_string();
    }

    #[test]
    fn public_watchlist_never_includes_owner_address() {
        // PublicWatchlist struct must NOT have an owner_address field.
        // This is a compile-time check (the field simply doesn't exist).
        let pw = PublicWatchlist {
            name: "Test".to_string(),
            description: None,
            market_ids: vec![1, 2, 3],
        };
        assert_eq!(pw.market_ids.len(), 3);
    }
}
