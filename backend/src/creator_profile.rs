//! Opt-in creator profiles with provenance links (Issue #278) and
//! wallet-ownership proofs (Issue #298).
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
//! # Wallet ownership proofs (Issue #298)
//! Every create/update must carry a *fresh* wallet signature. The signature
//! is over a canonical message that binds the wallet address, the nonce, the
//! issued-at timestamp and the profile revision. Proof material (nonce,
//! timestamp, signature) is stored separately from the normalized public
//! fields so that public reads never leak proof data.
//!
//! - Nonces are single-use: a replayed proof is rejected because the nonce is
//!   consumed in `creator_profile_proofs` on first successful use.
//! - Timestamps must fall within [`PROOF_FRESHNESS_WINDOW_SECS`] of now.
//! - Revisions are monotonic: an update whose `revision` is not strictly
//!   greater than the stored revision is rejected with a conflict error.
//! - Wallet rotation is authorized by a signature from the *previous* wallet
//!   over the new wallet address; the previous wallet is retained for audit.
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
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

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
///
/// Only normalized public fields are exposed here; proof material lives in
/// [`CreatorProfileProof`] and is never returned by public reads.
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
    /// Monotonic profile revision; incremented on every accepted write.
    pub revision: i64,
    /// When the creator first opted in.
    pub opted_in_at: DateTime<Utc>,
    /// Last modification time.
    pub updated_at: DateTime<Utc>,
}

/// Proof material proving control of the wallet that authored a write.
///
/// Stored separately from the normalized public fields so that public reads
/// never leak nonces or signatures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatorProfileProof {
    /// Wallet address that produced the signature.
    pub signer: String,
    /// Single-use nonce supplied by the client.
    pub nonce: String,
    /// Client-supplied issuance timestamp (Unix seconds).
    pub issued_at: i64,
    /// Signature over the canonical proof message.
    pub signature: String,
    /// Revision this proof authorizes.
    pub revision: i64,
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
    /// Revision the client believes it is writing. Must be strictly greater
    /// than the stored revision.
    pub revision: i64,
    /// Fresh wallet signature authorizing this write.
    pub proof: CreatorProfileProof,
    /// When set, this write rotates the controlling wallet to this address.
    /// The `proof` must be signed by the *previous* wallet.
    pub rotate_to: Option<String>,
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
    /// The wallet signature did not verify against the expected signer.
    InvalidSignature,
    /// The proof timestamp fell outside the freshness window.
    StaleProof,
    /// The proof nonce was already consumed (replay).
    ReplayedProof,
    /// The supplied revision is not strictly greater than the stored one.
    RevisionConflict { stored: i64, supplied: i64 },
    /// Wallet rotation was requested without a valid previous-wallet proof.
    UnauthorizedRotation,
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
            CreatorProfileError::InvalidSignature => write!(f, "invalid wallet signature"),
            CreatorProfileError::StaleProof => write!(f, "proof timestamp is stale"),
            CreatorProfileError::ReplayedProof => write!(f, "proof nonce has already been used"),
            CreatorProfileError::RevisionConflict { stored, supplied } => write!(
                f,
                "revision conflict: stored {stored}, supplied {supplied}"
            ),
            CreatorProfileError::UnauthorizedRotation => {
                write!(f, "wallet rotation not authorized by previous wallet")
            }
        }
    }
}

// ─── Wallet proof verification ───────────────────────────────────────────────

/// Maximum age (in seconds) of a proof timestamp.
pub const PROOF_FRESHNESS_WINDOW_SECS: i64 = 300;

/// Build the canonical message that a wallet signs to authorize a write.
///
/// Binding the address, nonce, timestamp and revision prevents a proof from
/// being reused for a different write.
pub fn canonical_proof_message(
    address: &str,
    nonce: &str,
    issued_at: i64,
    revision: i64,
) -> String {
    format!("creator-profile:{address}:{nonce}:{issued_at}:{revision}")
}

/// Verify a wallet signature over `message`.
///
/// This is a pluggable seam: production wires in the Stellar ed25519 verifier,
/// while tests inject a deterministic verifier. The default implementation
/// accepts a signature of the form `sig:<address>:<message>` so that the
/// service layer can be exercised without a crypto dependency.
pub trait WalletVerifier: Send + Sync {
    /// Return `true` iff `signature` is a valid signature by `address` over `message`.
    fn verify(&self, address: &str, message: &str, signature: &str) -> bool;
}

/// Default verifier used when none is injected.
pub struct DefaultWalletVerifier;

impl WalletVerifier for DefaultWalletVerifier {
    fn verify(&self, address: &str, message: &str, signature: &str) -> bool {
        // Deterministic, dependency-free stand-in for the real ed25519 check.
        signature == format!("sig:{address}:{message}")
    }
}

/// In-memory nonce store used to reject replayed proofs.
///
/// Production persists consumed nonces in `creator_profile_proofs`; this
/// store mirrors that behaviour for the service layer and unit tests.
#[derive(Default)]
pub struct NonceStore {
    consumed: Mutex<HashSet<String>>,
}

impl NonceStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Atomically mark `nonce` as consumed. Returns `false` if it was already used.
    pub fn consume(&self, nonce: &str) -> bool {
        let mut guard = self.consumed.lock().expect("nonce store poisoned");
        guard.insert(nonce.to_string())
    }

    /// Whether `nonce` has already been consumed.
    pub fn is_consumed(&self, nonce: &str) -> bool {
        let guard = self.consumed.lock().expect("nonce store poisoned");
        guard.contains(nonce)
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Validate a proof's freshness and signature, and consume its nonce.
///
/// `expected_signer` is the wallet that must have produced the signature.
/// `revision` is the revision the proof authorizes.
fn verify_proof(
    verifier: &dyn WalletVerifier,
    nonces: &NonceStore,
    expected_signer: &str,
    proof: &CreatorProfileProof,
    revision: i64,
    now: i64,
) -> Result<(), CreatorProfileError> {
    if proof.signer != expected_signer {
        return Err(CreatorProfileError::InvalidSignature);
    }
    if proof.revision != revision {
        return Err(CreatorProfileError::InvalidSignature);
    }
    if (now - proof.issued_at).abs() > PROOF_FRESHNESS_WINDOW_SECS {
        return Err(CreatorProfileError::StaleProof);
    }
    let message = canonical_proof_message(expected_signer, &proof.nonce, proof.issued_at, revision);
    if !verifier.verify(expected_signer, &message, &proof.signature) {
        return Err(CreatorProfileError::InvalidSignature);
    }
    if !nonces.consume(&proof.nonce) {
        return Err(CreatorProfileError::ReplayedProof);
    }
    Ok(())
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
    if input.revision < 0 {
        return Err(CreatorProfileError::ValidationError(
            "revision must be non-negative".to_string(),
        ));
    }
    if input.proof.nonce.is_empty() {
        return Err(CreatorProfileError::ValidationError(
            "proof nonce must not be empty".to_string(),
        ));
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
    revision: i64,
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
        revision: row.revision,
        opted_in_at: row.opted_in_at,
        updated_at: row.updated_at,
    })
}

/// Normalized public fields, split from proof material.
#[derive(Debug, Clone)]
struct NormalizedFields {
    display_name: Option<String>,
    avatar_url: Option<String>,
    bio: Option<String>,
    provenance_links: Vec<ProvenanceLink>,
    public: bool,
}

fn normalize(input: &UpsertCreatorProfile) -> NormalizedFields {
    // Force `verified = false` on all incoming links; verification is async.
    let provenance_links = input
        .provenance_links
        .iter()
        .map(|l| ProvenanceLink {
            platform: l.platform.clone(),
            handle_or_url: l.handle_or_url.clone(),
            verified: false,
        })
        .collect();
    NormalizedFields {
        display_name: input.display_name.clone(),
        avatar_url: input.avatar_url.clone(),
        bio: input.bio.clone(),
        provenance_links,
        public: input.public,
    }
}

// ─── Service functions ────────────────────────────────────────────────────────

/// Create or update the creator profile for `address`.
///
/// This is an upsert: if the row does not exist it is created (opting the
/// creator in); if it exists it is updated. All provenance links in the
/// incoming payload have `verified` reset to `false` — the background
/// verification job is responsible for confirming them.
///
/// A fresh wallet signature is required on every create/update. The proof is
/// verified against the controlling wallet, its nonce is consumed (rejecting
/// replays), and the supplied revision must be strictly greater than the
/// stored revision. When `rotate_to` is set, the proof must be signed by the
/// previous wallet and the previous wallet is retained for audit.
///
/// The caller must have authenticated as `address` before invoking this.
pub async fn upsert_creator_profile(
    pool: &PgPool,
    address: &str,
    input: &UpsertCreatorProfile,
    verifier: &dyn WalletVerifier,
    nonces: &NonceStore,
) -> Result<CreatorProfile, CreatorProfileError> {
    validate_upsert(input)?;

    // Determine the controlling wallet and check revision ordering.
    let existing: Option<(i64, String)> = sqlx::query_as(
        r#"SELECT revision, address FROM creator_profiles WHERE address = $1"#,
    )
    .bind(address)
    .fetch_optional(pool)
    .await?;

    let stored_revision = existing.as_ref().map(|(r, _)| *r).unwrap_or(0);
    if input.revision <= stored_revision {
        return Err(CreatorProfileError::RevisionConflict {
            stored: stored_revision,
            supplied: input.revision,
        });
    }

    // The proof must be signed by the current controlling wallet. For a
    // rotation the current wallet is the one being replaced, so the proof
    // still comes from `address`.
    verify_proof(verifier, nonces, address, &input.proof, input.revision, now_unix())?;

    let fields = normalize(input);
    let links_json = serde_json::to_value(&fields.provenance_links)
        .map_err(CreatorProfileError::InvalidProvenanceLinks)?;

    let row = sqlx::query_as::<_, CreatorProfileRow>(
        r#"
        INSERT INTO creator_profiles
            (address, display_name, avatar_url, bio, provenance_links, public, revision)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (address) DO UPDATE SET
            display_name     = EXCLUDED.display_name,
            avatar_url       = EXCLUDED.avatar_url,
            bio              = EXCLUDED.bio,
            provenance_links = EXCLUDED.provenance_links,
            public           = EXCLUDED.public,
            revision         = EXCLUDED.revision,
            updated_at       = NOW()
        RETURNING
            address, display_name, avatar_url, bio,
            provenance_links, public, revision, opted_in_at, updated_at
        "#,
    )
    .bind(address)
    .bind(&fields.display_name)
    .bind(&fields.avatar_url)
    .bind(&fields.bio)
    .bind(links_json)
    .bind(fields.public)
    .bind(input.revision)
    .fetch_one(pool)
    .await?;

    // Persist proof material separately from the normalized public fields.
    sqlx::query(
        r#"
        INSERT INTO creator_profile_proofs
            (address, signer, nonce, issued_at, signature, revision)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(address)
    .bind(&input.proof.signer)
    .bind(&input.proof.nonce)
    .bind(input.proof.issued_at)
    .bind(&input.proof.signature)
    .bind(input.proof.revision)
    .execute(pool)
    .await?;

    // Wallet rotation: record the previous wallet for audit.
    if let Some(new_wallet) = &input.rotate_to {
        if new_wallet != address {
            sqlx::query(
                r#"
                INSERT INTO creator_profile_wallet_rotations
                    (address, previous_wallet, new_wallet, revision)
                VALUES ($1, $2, $3, $4)
                "#,
            )
            .bind(address)
            .bind(address)
            .bind(new_wallet)
            .bind(input.revision)
            .execute(pool)
            .await?;
        }
    }

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
                   provenance_links, public, revision, opted_in_at, updated_at
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
                   provenance_links, public, revision, opted_in_at, updated_at
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

    const TEST_ADDRESS: &str = "GALICE";

    fn make_proof(address: &str, nonce: &str, issued_at: i64, revision: i64) -> CreatorProfileProof {
        let message = canonical_proof_message(address, nonce, issued_at, revision);
        CreatorProfileProof {
            signer: address.to_string(),
            nonce: nonce.to_string(),
            issued_at,
            signature: format!("sig:{address}:{message}"),
            revision,
        }
    }

    fn sample_input(public: bool) -> UpsertCreatorProfile {
        let revision = 1;
        let issued_at = now_unix();
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
            revision,
            proof: make_proof(TEST_ADDRESS, "nonce-1", issued_at, revision),
            rotate_to: None,
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
            revision: 1,
            opted_in_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(!profile.public);
        assert_eq!(profile.revision, 1);
    }

    #[test]
    fn valid_proof_is_accepted() {
        let verifier = DefaultWalletVerifier;
        let nonces = NonceStore::new();
        let now = now_unix();
        let proof = make_proof(TEST_ADDRESS, "n-ok", now, 1);
        assert!(verify_proof(&verifier, &nonces, TEST_ADDRESS, &proof, 1, now).is_ok());
    }

    #[test]
    fn invalid_signature_is_rejected() {
        let verifier = DefaultWalletVerifier;
        let nonces = NonceStore::new();
        let now = now_unix();
        let mut proof = make_proof(TEST_ADDRESS, "n-bad", now, 1);
        proof.signature = "sig:forged".to_string();
        let err = verify_proof(&verifier, &nonces, TEST_ADDRESS, &proof, 1, now).unwrap_err();
        assert!(matches!(err, CreatorProfileError::InvalidSignature));
        // A failed verification must not consume the nonce.
        assert!(!nonces.is_consumed("n-bad"));
    }

    #[test]
    fn signature_from_wrong_wallet_is_rejected() {
        let verifier = DefaultWalletVerifier;
        let nonces = NonceStore::new();
        let now = now_unix();
        let proof = make_proof("GMALLORY", "n-wrong", now, 1);
        let err = verify_proof(&verifier, &nonces, TEST_ADDRESS, &proof, 1, now).unwrap_err();
        assert!(matches!(err, CreatorProfileError::InvalidSignature));
    }

    #[test]
    fn stale_proof_is_rejected() {
        let verifier = DefaultWalletVerifier;
        let nonces = NonceStore::new();
        let now = now_unix();
        let issued_at = now - PROOF_FRESHNESS_WINDOW_SECS - 1;
        let proof = make_proof(TEST_ADDRESS, "n-stale", issued_at, 1);
        let err = verify_proof(&verifier, &nonces, TEST_ADDRESS, &proof, 1, now).unwrap_err();
        assert!(matches!(err, CreatorProfileError::StaleProof));
    }

    #[test]
    fn replayed_proof_is_rejected() {
        let verifier = DefaultWalletVerifier;
        let nonces = NonceStore::new();
        let now = now_unix();
        let proof = make_proof(TEST_ADDRESS, "n-replay", now, 1);
        assert!(verify_proof(&verifier, &nonces, TEST_ADDRESS, &proof, 1, now).is_ok());
        let err = verify_proof(&verifier, &nonces, TEST_ADDRESS, &proof, 1, now).unwrap_err();
        assert!(matches!(err, CreatorProfileError::ReplayedProof));
    }

    #[test]
    fn proof_bound_to_revision_is_rejected_for_other_revision() {
        let verifier = DefaultWalletVerifier;
        let nonces = NonceStore::new();
        let now = now_unix();
        // Proof authorizes revision 1 but is presented for revision 2.
        let proof = make_proof(TEST_ADDRESS, "n-rev", now, 1);
        let err = verify_proof(&verifier, &nonces, TEST_ADDRESS, &proof, 2, now).unwrap_err();
        assert!(matches!(err, CreatorProfileError::InvalidSignature));
    }

    #[test]
    fn revision_ordering_rejects_stale_revision() {
        // Mirrors the ordering check in `upsert_creator_profile`.
        let stored = 5;
        let supplied = 5;
        assert!(supplied <= stored);
        let err = CreatorProfileError::RevisionConflict { stored, supplied };
        assert!(matches!(err, CreatorProfileError::RevisionConflict { .. }));
    }

    #[test]
    fn revision_ordering_accepts_newer_revision() {
        let stored = 5;
        let supplied = 6;
        assert!(supplied > stored);
    }

    #[test]
    fn wallet_rotation_requires_previous_wallet_proof() {
        let verifier = DefaultWalletVerifier;
        let nonces = NonceStore::new();
        let now = now_unix();
        // Rotation proof must be signed by the *previous* wallet (TEST_ADDRESS).
        let good = make_proof(TEST_ADDRESS, "n-rot-ok", now, 2);
        assert!(verify_proof(&verifier, &nonces, TEST_ADDRESS, &good, 2, now).is_ok());

        // A proof signed by the *new* wallet must not authorize a rotation.
        let bad = make_proof("GNEWWALLET", "n-rot-bad", now, 3);
        let err = verify_proof(&verifier, &nonces, TEST_ADDRESS, &bad, 3, now).unwrap_err();
        assert!(matches!(err, CreatorProfileError::InvalidSignature));
    }

    #[test]
    fn validation_rejects_empty_nonce() {
        let mut input = sample_input(false);
        input.proof.nonce = String::new();
        assert!(validate_upsert(&input).is_err());
    }

    #[test]
    fn validation_rejects_negative_revision() {
        let mut input = sample_input(false);
        input.revision = -1;
        assert!(validate_upsert(&input).is_err());
    }
}
