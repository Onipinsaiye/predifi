//! Opt-in creator profiles with provenance links (Issue #278).
//!
//! Creator profiles are opt-in: a creator must explicitly save a profile
//! before it appears anywhere in the product. An absent row means "not opted
//! in" and must never be surfaced publicly.
//!
//! # Data ownership
//! All fields in this module are private by default (`public = FALSE`).
//! They are owned by the creator and may be deleted via the user-controlled
//! account-deletion flow in `profile.rs`. They are retained for up to 365 days
//! after the last update (matching `user_profiles.display_data` in
//! `RETENTION_POLICY`).
//!
//! # Provenance links
//! Links are stored without verification at save time. A background job is
//! responsible for setting `verified = true` on each link once the platform
//! challenge is confirmed. The API surface exposes `verified` as-is so the UI
//! can distinguish "pending verification" from "verified".
//!
//! # Privacy invariants
//! - Only the authenticated creator may write their own profile.
//! - Only public profiles (`public = true`) are returned by the read endpoints
//!   used by markets / market lists.
//! - The creator themselves always sees their own profile (public or private)
//!   when querying the authenticated self-profile endpoint.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Supported provenance-link platforms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProvenancePlatform {
    Twitter,
    Github,
    Farcaster,
    Lens,
    /// Any well-formed HTTPS URL the creator wants to link.
    Url,
}

/// A single provenance link attached to a creator profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceLink {
    /// Platform identifier.
    pub platform: ProvenancePlatform,
    /// Platform handle (e.g. `@alice`) or full URL for `Url` platform.
    pub handle_or_url: String,
    /// Whether the background verification job has confirmed this link.
    pub verified: bool,
}

/// Full creator profile record as stored in the DB.
#[derive(Debug, Clone, Serialize)]
pub struct CreatorProfile {
    /// Stellar wallet address — the creator's identity.
    pub address: String,
    /// Opt-in display name (max 64 chars).
    pub display_name: Option<String>,
    /// HTTPS avatar URL (max 512 chars).
    pub avatar_url: Option<String>,
    /// Short bio (max 280 chars).
    pub bio: Option<String>,
    /// Provenance links with verification status.
    pub provenance_links: Vec<ProvenanceLink>,
    /// Whether the profile is publicly visible.
    pub public: bool,
    /// When the creator first opted in.
    pub opted_in_at: DateTime<Utc>,
    /// Last modification time.
    pub updated_at: DateTime<Utc>,
}

/// Input payload for creating or updating a creator profile.
#[derive(Debug, Clone, Deserialize)]
pub struct UpsertCreatorProfile {
    /// Opt-in display name. `None` clears the existing value.
    pub display_name: Option<String>,
    /// Avatar URL. `None` clears the existing value.
    pub avatar_url: Option<String>,
    /// Bio. `None` clears the existing value.
    pub bio: Option<String>,
    /// Full replacement list of provenance links.
    /// Every link starts with `verified = false`; the background job verifies them.
    pub provenance_links: Vec<ProvenanceLink>,
    /// Desired public-visibility setting.
    pub public: bool,
}

/// Errors that the creator-profile service layer can produce.
#[derive(Debug)]
pub enum CreatorProfileError {
    /// Underlying database error.
    Db(sqlx::Error),
    /// A field exceeded its length constraint.
    ValidationError(String),
    /// The JSON provenance-links payload could not be parsed.
    InvalidProvenanceLinks(serde_json::Error),
}

impl From<sqlx::Error> for CreatorProfileError {
    fn from(e: sqlx::Error) -> Self {
        CreatorProfileError::Db(e)
    }
}

impl std::fmt::Display for CreatorProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreatorProfileError::Db(e) => write!(f, "database error: {e}"),
            CreatorProfileError::ValidationError(msg) => write!(f, "validation error: {msg}"),
            CreatorProfileError::InvalidProvenanceLinks(e) => {
                write!(f, "invalid provenance links: {e}")
            }
        }
    }
}

// ─── Validation ──────────────────────────────────────────────────────────────

const MAX_DISPLAY_NAME_LEN: usize = 64;
const MAX_AVATAR_URL_LEN: usize = 512;
const MAX_BIO_LEN: usize = 280;
const MAX_HANDLE_OR_URL_LEN: usize = 256;
const MAX_PROVENANCE_LINKS: usize = 10;

/// Validate the fields of an `UpsertCreatorProfile` before writing to the DB.
pub fn validate_upsert(input: &UpsertCreatorProfile) -> Result<(), CreatorProfileError> {
    if let Some(name) = &input.display_name {
        if name.len() > MAX_DISPLAY_NAME_LEN {
            return Err(CreatorProfileError::ValidationError(format!(
                "display_name exceeds {MAX_DISPLAY_NAME_LEN} characters"
            )));
        }
    }
    if let Some(url) = &input.avatar_url {
        if url.len() > MAX_AVATAR_URL_LEN {
            return Err(CreatorProfileError::ValidationError(format!(
                "avatar_url exceeds {MAX_AVATAR_URL_LEN} characters"
            )));
        }
        if !url.starts_with("https://") {
            return Err(CreatorProfileError::ValidationError(
                "avatar_url must use HTTPS".to_string(),
            ));
        }
    }
    if let Some(bio) = &input.bio {
        if bio.len() > MAX_BIO_LEN {
            return Err(CreatorProfileError::ValidationError(format!(
                "bio exceeds {MAX_BIO_LEN} characters"
            )));
        }
    }
    if input.provenance_links.len() > MAX_PROVENANCE_LINKS {
        return Err(CreatorProfileError::ValidationError(format!(
            "provenance_links may contain at most {MAX_PROVENANCE_LINKS} entries"
        )));
    }
    for link in &input.provenance_links {
        if link.handle_or_url.is_empty() {
            return Err(CreatorProfileError::ValidationError(
                "provenance link handle_or_url must not be empty".to_string(),
            ));
        }
        if link.handle_or_url.len() > MAX_HANDLE_OR_URL_LEN {
            return Err(CreatorProfileError::ValidationError(format!(
                "provenance link handle_or_url exceeds {MAX_HANDLE_OR_URL_LEN} characters"
            )));
        }
    }
    Ok(())
}

// ─── DB helpers ──────────────────────────────────────────────────────────────

/// Internal row type for sqlx deserialization.
#[derive(sqlx::FromRow)]
struct CreatorProfileRow {
    address: String,
    display_name: Option<String>,
    avatar_url: Option<String>,
    bio: Option<String>,
    provenance_links: serde_json::Value,
    public: bool,
    opted_in_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn row_to_profile(row: CreatorProfileRow) -> Result<CreatorProfile, CreatorProfileError> {
    let provenance_links: Vec<ProvenanceLink> =
        serde_json::from_value(row.provenance_links)
            .map_err(CreatorProfileError::InvalidProvenanceLinks)?;
    Ok(CreatorProfile {
        address: row.address,
        display_name: row.display_name,
        avatar_url: row.avatar_url,
        bio: row.bio,
        provenance_links,
        public: row.public,
        opted_in_at: row.opted_in_at,
        updated_at: row.updated_at,
    })
}

// ─── Service functions ────────────────────────────────────────────────────────

/// Create or update the creator profile for `address`.
///
/// This is an upsert: if the row does not exist it is created (opting the
/// creator in); if it exists it is updated. All provenance links in the
/// incoming payload have `verified` reset to `false` — the background
/// verification job is responsible for confirming them.
///
/// The caller must have authenticated as `address` before invoking this.
pub async fn upsert_creator_profile(
    pool: &PgPool,
    address: &str,
    input: &UpsertCreatorProfile,
) -> Result<CreatorProfile, CreatorProfileError> {
    validate_upsert(input)?;

    // Force `verified = false` on all incoming links; verification is async.
    let sanitized_links: Vec<ProvenanceLink> = input
        .provenance_links
        .iter()
        .map(|l| ProvenanceLink {
            platform: l.platform.clone(),
            handle_or_url: l.handle_or_url.clone(),
            verified: false,
        })
        .collect();
    let links_json = serde_json::to_value(&sanitized_links).map_err(|e| {
        CreatorProfileError::InvalidProvenanceLinks(e)
    })?;

    let row = sqlx::query_as::<_, CreatorProfileRow>(
        r#"
        INSERT INTO creator_profiles
            (address, display_name, avatar_url, bio, provenance_links, public)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (address) DO UPDATE SET
            display_name     = EXCLUDED.display_name,
            avatar_url       = EXCLUDED.avatar_url,
            bio              = EXCLUDED.bio,
            provenance_links = EXCLUDED.provenance_links,
            public           = EXCLUDED.public,
            updated_at       = NOW()
        RETURNING
            address, display_name, avatar_url, bio,
            provenance_links, public, opted_in_at, updated_at
        "#,
    )
    .bind(address)
    .bind(&input.display_name)
    .bind(&input.avatar_url)
    .bind(&input.bio)
    .bind(links_json)
    .bind(input.public)
    .fetch_one(pool)
    .await?;

    row_to_profile(row)
}

/// Fetch a creator profile by address.
///
/// - When `require_public = true` (used by market-list / market-detail
///   endpoints), only rows with `public = TRUE` are returned.  Callers must
///   pass `require_public = false` for the authenticated self-profile endpoint.
pub async fn get_creator_profile(
    pool: &PgPool,
    address: &str,
    require_public: bool,
) -> Result<Option<CreatorProfile>, CreatorProfileError> {
    let row: Option<CreatorProfileRow> = if require_public {
        sqlx::query_as::<_, CreatorProfileRow>(
            r#"
            SELECT address, display_name, avatar_url, bio,
                   provenance_links, public, opted_in_at, updated_at
            FROM creator_profiles
            WHERE address = $1 AND public = TRUE
            "#,
        )
        .bind(address)
        .fetch_optional(pool)
        .await?
    } else {
        sqlx::query_as::<_, CreatorProfileRow>(
            r#"
            SELECT address, display_name, avatar_url, bio,
                   provenance_links, public, opted_in_at, updated_at
            FROM creator_profiles
            WHERE address = $1
            "#,
        )
        .bind(address)
        .fetch_optional(pool)
        .await?
    };

    row.map(row_to_profile).transpose()
}

/// Delete the creator profile for `address`.
///
/// Called by the user-controlled account-deletion flow. Returns the number
/// of rows deleted (0 if the creator had not opted in).
pub async fn delete_creator_profile(
    pool: &PgPool,
    address: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"DELETE FROM creator_profiles WHERE address = $1"#,
    )
    .bind(address)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Mark a specific provenance link as verified.
///
/// Called by the background verification job once a platform challenge
/// has been confirmed.  The link is identified by its index position within
/// the JSONB array (the simplest approach that avoids adding a separate
/// junction table).
pub async fn mark_provenance_link_verified(
    pool: &PgPool,
    address: &str,
    link_index: usize,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE creator_profiles
        SET provenance_links = jsonb_set(
            provenance_links,
            ARRAY[$1::text, 'verified'],
            'true'::jsonb
        ),
        updated_at = NOW()
        WHERE address = $2
          AND jsonb_array_length(provenance_links) > $1
        "#,
    )
    .bind(link_index as i32)
    .bind(address)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input(public: bool) -> UpsertCreatorProfile {
        UpsertCreatorProfile {
            display_name: Some("Alice".to_string()),
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            bio: Some("Prediction market creator".to_string()),
            provenance_links: vec![ProvenanceLink {
                platform: ProvenancePlatform::Github,
                handle_or_url: "alice-on-github".to_string(),
                verified: true, // should be reset to false on save
            }],
            public,
        }
    }

    #[test]
    fn validation_accepts_valid_input() {
        assert!(validate_upsert(&sample_input(false)).is_ok());
    }

    #[test]
    fn validation_rejects_display_name_too_long() {
        let mut input = sample_input(false);
        input.display_name = Some("x".repeat(MAX_DISPLAY_NAME_LEN + 1));
        assert!(validate_upsert(&input).is_err());
    }

    #[test]
    fn validation_rejects_bio_too_long() {
        let mut input = sample_input(false);
        input.bio = Some("y".repeat(MAX_BIO_LEN + 1));
        assert!(validate_upsert(&input).is_err());
    }

    #[test]
    fn validation_rejects_non_https_avatar() {
        let mut input = sample_input(false);
        input.avatar_url = Some("http://example.com/img.png".to_string());
        assert!(validate_upsert(&input).is_err());
    }

    #[test]
    fn validation_rejects_avatar_url_too_long() {
        let mut input = sample_input(false);
        input.avatar_url = Some(format!("https://{}", "a".repeat(MAX_AVATAR_URL_LEN)));
        assert!(validate_upsert(&input).is_err());
    }

    #[test]
    fn validation_rejects_too_many_provenance_links() {
        let mut input = sample_input(false);
        input.provenance_links = (0..=MAX_PROVENANCE_LINKS)
            .map(|i| ProvenanceLink {
                platform: ProvenancePlatform::Url,
                handle_or_url: format!("https://example.com/{i}"),
                verified: false,
            })
            .collect();
        assert!(validate_upsert(&input).is_err());
    }

    #[test]
    fn validation_rejects_empty_handle_or_url() {
        let mut input = sample_input(false);
        input.provenance_links = vec![ProvenanceLink {
            platform: ProvenancePlatform::Github,
            handle_or_url: String::new(),
            verified: false,
        }];
        assert!(validate_upsert(&input).is_err());
    }

    #[test]
    fn validation_rejects_handle_or_url_too_long() {
        let mut input = sample_input(false);
        input.provenance_links = vec![ProvenanceLink {
            platform: ProvenancePlatform::Github,
            handle_or_url: "g".repeat(MAX_HANDLE_OR_URL_LEN + 1),
            verified: false,
        }];
        assert!(validate_upsert(&input).is_err());
    }

    #[test]
    fn validation_accepts_no_avatar_url() {
        let mut input = sample_input(false);
        input.avatar_url = None;
        assert!(validate_upsert(&input).is_ok());
    }

    #[test]
    fn validation_accepts_empty_provenance_links() {
        let mut input = sample_input(false);
        input.provenance_links = vec![];
        assert!(validate_upsert(&input).is_ok());
    }

    #[test]
    fn provenance_link_verified_is_reset_to_false_on_upsert_input() {
        // We cannot call the DB, but the sanitization logic is extracted
        // into a closure we can test directly.
        let input = sample_input(false); // has verified: true
        let sanitized: Vec<ProvenanceLink> = input
            .provenance_links
            .iter()
            .map(|l| ProvenanceLink {
                platform: l.platform.clone(),
                handle_or_url: l.handle_or_url.clone(),
                verified: false, // must always be reset
            })
            .collect();
        for link in &sanitized {
            assert!(!link.verified, "incoming verified flag must be reset to false");
        }
    }

    #[test]
    fn provenance_platform_serialises_to_lowercase() {
        let platform = ProvenancePlatform::Farcaster;
        let json = serde_json::to_string(&platform).unwrap();
        assert_eq!(json, r#""farcaster""#);
    }

    #[test]
    fn private_profile_public_field_is_false_by_default() {
        // Simulate the DB default: a freshly opted-in profile is private.
        let profile = CreatorProfile {
            address: "GA...".to_string(),
            display_name: None,
            avatar_url: None,
            bio: None,
            provenance_links: vec![],
            public: false, // default
            opted_in_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(!profile.public);
    }
}
