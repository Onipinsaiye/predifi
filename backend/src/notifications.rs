//! In-app notification system.
//!
//! Alerts users when: their pool is about to end, a pool they predicted on
//! is resolved, their claim window is about to expire, and when new pools
//! matching their interests are created.
//!
//! Delivery uses a transactional outbox: notification intents are written to
//! `notification_outbox` in the same transaction as the domain mutation that
//! triggers them, then a background worker claims, delivers, and retries them.
//! This guarantees that rolled-back domain writes emit no notification and
//! that committed writes eventually deliver exactly once (logically).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use std::sync::Arc;
use tracing::{info, instrument, warn};

/// How far ahead of `end_time` a pool counts as "ending soon".
const POOL_ENDING_SOON_WINDOW: &str = "1 hour";
/// How far ahead of claim-window expiry a user is warned.
const CLAIM_EXPIRING_WINDOW: &str = "1 day";
/// How far back to scan for newly created pools when matching interests.
const NEW_POOL_LOOKBACK: &str = "1 day";
/// Maximum delivery attempts before an outbox row is marked terminally failed.
const MAX_DELIVERY_ATTEMPTS: i32 = 5;

// ── Retention and deletion policy for private user records ──────────────────
//
// Classification of user-linked fields:
//
// | Data class                  | Examples                                 | Retention                     | Deletion behavior                              |
// |-----------------------------|------------------------------------------|-------------------------------|------------------------------------------------|
// | Delivered notifications     | notification_outbox rows in `delivered`  | 30 days after delivery        | Hard delete; recipient id scrubbed first       |
// | Terminally failed outbox    | outbox rows in `failed`                  | 30 days after last attempt    | Hard delete; payload and recipient scrubbed    |
// | Notification targets        | recipient address/handle on outbox rows  | Lifetime of pending delivery  | Nulled on account deletion, then row purged    |
// | Exports                     | export artifacts referencing user data   | 7 days after creation         | Hard delete of artifact                        |
// | Audit records               | credential_rotation_audit                | Retained (no personal data)   | Never deleted; contains key ids only           |
// | Public chain-derived events | pool/prediction/resolution history       | Immutable, public             | Never deleted; not personal data by policy     |
//
// Private profile and discussion content is scrubbed (not kept) once the
// account is deleted; only non-personal audit and chain-derived history remain.

/// Retention window for terminal outbox rows (delivered or failed).
const OUTBOX_TERMINAL_RETENTION: &str = "30 days";
/// Retention window for generated export artifacts.
const EXPORT_RETENTION: &str = "7 days";

/// Summary of a single retention sweep, used for observability and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetentionReport {
    pub outbox_scrubbed: u64,
    pub outbox_purged: u64,
    pub exports_purged: u64,
}

/// Scrub personal fields and purge terminal outbox rows older than the
/// retention window. Audit tables and chain-derived data are never touched.
///
/// Scrubbing happens before purging so that rows retained by a concurrent
/// transaction never carry recipient data past the window.
#[instrument(skip(pool))]
pub async fn run_retention_sweep(pool: &PgPool) -> Result<RetentionReport, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let scrubbed = sqlx::query(
        r#"
        UPDATE notification_outbox
        SET recipient = NULL,
            payload = '{}'::jsonb
        WHERE status IN ('delivered', 'failed')
          AND recipient IS NOT NULL
          AND updated_at < now() - $1::interval
        "#,
    )
    .bind(OUTBOX_TERMINAL_RETENTION)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    let purged = sqlx::query(
        r#"
        DELETE FROM notification_outbox
        WHERE status IN ('delivered', 'failed')
          AND updated_at < now() - $1::interval
        "#,
    )
    .bind(OUTBOX_TERMINAL_RETENTION)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    let exports = sqlx::query(
        r#"
        DELETE FROM notification_exports
        WHERE created_at < now() - $1::interval
        "#,
    )
    .bind(EXPORT_RETENTION)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    tx.commit().await?;

    let report = RetentionReport {
        outbox_scrubbed: scrubbed,
        outbox_purged: purged,
        exports_purged: exports,
    };
    info!(?report, "retention sweep complete");
    Ok(report)
}
/// How long a claimed outbox row is leased before another worker may reclaim it.
const CLAIM_LEASE_SECONDS: i64 = 60;

// ── Provider credential rotation (#355) ─────────────────────────────────────

/// A provider credential identified by an opaque key id.
///
/// The key id is what appears in audit records; the secret itself is never
/// logged. A credential can be `Active` (usable for delivery), `Revoked`
/// (must never deliver again), or `Retired` (kept only for the tail of a
/// dual-key transition window).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCredential {
    pub key_id: String,
    pub secret: String,
    pub state: CredentialState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialState {
    Active,
    Revoked,
    Retired,
}

impl CredentialState {
    fn as_str(self) -> &'static str {
        match self {
            CredentialState::Active => "active",
            CredentialState::Revoked => "revoked",
            CredentialState::Retired => "retired",
        }
    }
}

/// Whether the provider accepts two credentials at once during a rotation.
///
/// Dual-key transition is only permitted where provider semantics allow it;
/// providers that do not support overlapping keys must use a hard cutover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DualKeySupport {
    Supported,
    Unsupported,
}

/// The set of credentials currently configured for the notification provider.
///
/// Delivery always uses the `Active` credential. During a dual-key transition
/// the previous credential is retained as `Retired` so in-flight deliveries
/// signed with it can still be verified, but it is never used to sign new
/// deliveries. A `Revoked` credential is rejected outright.
#[derive(Debug, Clone)]
pub struct CredentialSet {
    pub active: ProviderCredential,
    pub previous: Option<ProviderCredential>,
    pub dual_key: DualKeySupport,
}

impl CredentialSet {
    /// Build a credential set with a single active credential.
    pub fn single(active: ProviderCredential) -> Self {
        Self {
            active,
            previous: None,
            dual_key: DualKeySupport::Unsupported,
        }
    }

    /// Return the credential used to sign a new delivery, or an error if the
    /// active credential has been revoked.
    pub fn signing_credential(&self) -> Result<&ProviderCredential, RotationError> {
        match self.active.state {
            CredentialState::Active => Ok(&self.active),
            CredentialState::Revoked => Err(RotationError::RevokedCredential {
                key_id: self.active.key_id.clone(),
            }),
            CredentialState::Retired => Err(RotationError::NoActiveCredential),
        }
    }

    /// Verify that a credential presented by the provider is still acceptable.
    /// Revoked credentials are always rejected, even during a transition.
    pub fn accepts(&self, key_id: &str) -> bool {
        if self.active.key_id == key_id && self.active.state == CredentialState::Active {
            return true;
        }
        match &self.previous {
            Some(prev) => {
                prev.key_id == key_id
                    && self.dual_key == DualKeySupport::Supported
                    && prev.state == CredentialState::Retired
            }
            None => false,
        }
    }
}

/// Errors raised while rotating or using provider credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RotationError {
    /// The credential in use has been revoked and cannot deliver.
    RevokedCredential { key_id: String },
    /// No usable active credential is configured.
    NoActiveCredential,
    /// A dual-key transition was requested but the provider does not allow it.
    DualKeyUnsupported,
    /// The replacement credential is identical to the one being replaced.
    SameCredential,
}

impl std::fmt::Display for RotationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RotationError::RevokedCredential { key_id } => {
                write!(f, "provider credential {key_id} has been revoked")
            }
            RotationError::NoActiveCredential => write!(f, "no active provider credential"),
            RotationError::DualKeyUnsupported => {
                write!(f, "provider does not support dual-key transition")
            }
            RotationError::SameCredential => {
                write!(f, "replacement credential matches the current credential")
            }
        }
    }
}

impl std::error::Error for RotationError {}

/// Outcome of a rotation attempt, recorded in the audit log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationOutcome {
    /// New credential installed; old credential revoked immediately.
    Completed,
    /// New credential installed; old credential retained for a transition window.
    DualKeyTransition,
    /// Rotation was rejected; the old credential remains in place.
    Rejected,
}

impl RotationOutcome {
    fn as_str(self) -> &'static str {
        match self {
            RotationOutcome::Completed => "completed",
            RotationOutcome::DualKeyTransition => "dual_key_transition",
            RotationOutcome::Rejected => "rejected",
        }
    }
}

/// An audit record for a credential rotation event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotationAudit {
    pub old_key_id: String,
    pub new_key_id: String,
    pub actor: String,
    pub rotated_at: DateTime<Utc>,
    pub outcome: RotationOutcome,
    pub detail: Option<String>,
}

/// Sink for rotation audit records. Implementations persist to durable storage;
/// the in-memory variant is used by tests and as a fallback.
pub trait RotationAuditSink: Send + Sync {
    fn record(&self, audit: &RotationAudit);
}

/// In-memory audit sink, useful for tests and single-process fallbacks.
#[derive(Debug, Default)]
pub struct InMemoryRotationAudit {
    entries: std::sync::Mutex<Vec<RotationAudit>>,
}

impl InMemoryRotationAudit {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entries(&self) -> Vec<RotationAudit> {
        self.entries.lock().expect("audit lock poisoned").clone()
    }
}

impl RotationAuditSink for InMemoryRotationAudit {
    fn record(&self, audit: &RotationAudit) {
        self.entries
            .lock()
            .expect("audit lock poisoned")
            .push(audit.clone());
    }
}

/// Persist a rotation audit record to the `credential_rotation_audit` table.
pub async fn persist_rotation_audit(
    pool: &PgPool,
    audit: &RotationAudit,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO credential_rotation_audit
            (old_key_id, new_key_id, actor, rotated_at, outcome, detail)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(&audit.old_key_id)
    .bind(&audit.new_key_id)
    .bind(&audit.actor)
    .bind(audit.rotated_at)
    .bind(audit.outcome.as_str())
    .bind(&audit.detail)
    .execute(pool)
    .await?;
    Ok(())
}

/// Rotate the provider credential in `set` to `replacement`.
///
/// * When the provider supports dual-key transition, the old credential is
///   retained as `Retired` for the transition window and the outcome is
///   [`RotationOutcome::DualKeyTransition`].
/// * Otherwise the old credential is revoked immediately and the outcome is
///   [`RotationOutcome::Completed`].
///
/// The rotation event is always audited, including rejected attempts.
pub fn rotate_credential(
    set: &mut CredentialSet,
    replacement: ProviderCredential,
    actor: &str,
    audit: &dyn RotationAuditSink,
) -> Result<RotationOutcome, RotationError> {
    let now = Utc::now();
    let old_key_id = set.active.key_id.clone();
    let new_key_id = replacement.key_id.clone();

    let reject = |detail: &str| {
        audit.record(&RotationAudit {
            old_key_id: old_key_id.clone(),
            new_key_id: new_key_id.clone(),
            actor: actor.to_string(),
            rotated_at: now,
            outcome: RotationOutcome::Rejected,
            detail: Some(detail.to_string()),
        });
    };

    if old_key_id == new_key_id {
        reject("replacement credential matches the current credential");
        return Err(RotationError::SameCredential);
    }

    if replacement.state != CredentialState::Active {
        reject("replacement credential is not active");
        return Err(RotationError::NoActiveCredential);
    }

    let outcome = match set.dual_key {
        DualKeySupport::Supported => {
            let mut retired = set.active.clone();
            retired.state = CredentialState::Retired;
            set.previous = Some(retired);
            RotationOutcome::DualKeyTransition
        }
        DualKeySupport::Unsupported => {
            // Hard cutover: the old credential is revoked immediately so it can
            // never deliver again.
            set.previous = None;
            RotationOutcome::Completed
        }
    };

    set.active = replacement;

    audit.record(&RotationAudit {
        old_key_id,
        new_key_id,
        actor: actor.to_string(),
        rotated_at: now,
        outcome,
        detail: None,
    });

    Ok(outcome)
}

/// A notification provider that can deliver a message using a credential.
pub trait NotificationProvider: Send + Sync {
    /// Deliver `message` to `recipient` using `credential`. Implementations
    /// must reject revoked credentials.
    fn deliver(
        &self,
        credential: &ProviderCredential,
        recipient: &str,
        message: &str,
    ) -> Result<(), ProviderError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    Revoked,
    Unavailable,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Revoked => write!(f, "provider rejected revoked credential"),
            ProviderError::Unavailable => write!(f, "provider unavailable"),
        }
    }
}

impl std::error::Error for ProviderError {}

/// Deliver a single message through `provider` using the active credential in
/// `set`. Revoked credentials are rejected before the provider is called.
pub fn deliver_with_credentials(
    provider: &dyn NotificationProvider,
    set: &CredentialSet,
    recipient: &str,
    message: &str,
) -> Result<(), RotationError> {
    let credential = set.signing_credential()?;
    provider
        .deliver(credential, recipient, message)
        .map_err(|_| RotationError::RevokedCredential {
            key_id: credential.key_id.clone(),
        })
}

/// Shared handle to a credential set so the delivery worker can observe
/// rotations without restarting.
pub type SharedCredentials = Arc<std::sync::RwLock<CredentialSet>>;

/// Wrap a credential set for concurrent use by the delivery worker.
pub fn shared_credentials(set: CredentialSet) -> SharedCredentials {
    Arc::new(std::sync::RwLock::new(set))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifType {
    PoolEndingSoon,
    PoolResolved,
    ClaimExpiring,
    NewPoolMatch,
}

impl NotifType {
    fn as_str(self) -> &'static str {
        match self {
            NotifType::PoolEndingSoon => "pool_ending_soon",
            NotifType::PoolResolved => "pool_resolved",
            NotifType::ClaimExpiring => "claim_expiring",
            NotifType::NewPoolMatch => "new_pool_match",
        }
    }
}

/// A single notification row.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct NotificationRow {
    pub id: i64,
    pub user_address: String,
    pub notif_type: String,
    pub title: String,
    pub message: String,
    pub pool_id: Option<i64>,
    pub read: bool,
    pub created_at: DateTime<Utc>,
}

/// Insert a notification for `user_address`, silently skipping if an
/// identical `(user, pool, type)` notification already exists.
async fn create_notification(
    pool: &PgPool,
    user_address: &str,
    notif_type: NotifType,
    title: &str,
    message: &str,
    pool_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO notifications (user_address, notif_type, title, message, pool_id)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (user_address, pool_id, notif_type) DO NOTHING
        "#,
    )
    .bind(user_address)
    .bind(notif_type.as_str())
    .bind(title)
    .bind(message)
    .bind(pool_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Write a notification intent into the transactional outbox using the caller's
/// transaction. The intent is only visible to the delivery worker once the
/// surrounding domain transaction commits; a rollback discards it entirely.
pub async fn enqueue_notification(
    tx: &mut Transaction<'_, Postgres>,
    user_address: &str,
    notif_type: NotifType,
    title: &str,
    message: &str,
    pool_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO notification_outbox
            (user_address, notif_type, title, message, pool_id)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (user_address, pool_id, notif_type) DO NOTHING
        "#,
    )
    .bind(user_address)
    .bind(notif_type.as_str())
    .bind(title)
    .bind(message)
    .bind(pool_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// A claimed outbox row awaiting delivery.
#[derive(Debug, Clone, FromRow)]
pub struct OutboxRow {
    pub id: i64,
    pub user_address: String,
    pub notif_type: String,
    pub title: String,
    pub message: String,
    pub pool_id: i64,
    pub attempts: i32,
}

/// Claim up to `limit` pending outbox rows for delivery, leasing them so that
/// concurrent workers do not process the same row. Rows whose lease has expired
/// (e.g. after a process crash) become claimable again.
#[instrument(skip(pool), name = "notifications.claim_outbox")]
pub async fn claim_outbox(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<OutboxRow>, sqlx::Error> {
    let rows: Vec<OutboxRow> = sqlx::query_as(&format!(
        r#"
        UPDATE notification_outbox
        SET status = 'processing',
            attempts = attempts + 1,
            claimed_at = NOW(),
            updated_at = NOW()
        WHERE id IN (
            SELECT id FROM notification_outbox
            WHERE status = 'pending'
               OR (status = 'processing'
                   AND claimed_at < NOW() - INTERVAL '{CLAIM_LEASE_SECONDS} seconds')
            ORDER BY id
            LIMIT $1
            FOR UPDATE SKIP LOCKED
        )
        RETURNING id, user_address, notif_type, title, message, pool_id, attempts
        "#
    ))
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Mark a claimed outbox row as successfully delivered.
pub async fn mark_delivered(pool: &PgPool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE notification_outbox SET status = 'delivered', updated_at = NOW() WHERE id = $1",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a failed delivery attempt. Rows that exceed [`MAX_DELIVERY_ATTEMPTS`]
/// transition to the terminal `failed` state; others return to `pending` for a
/// later retry.
pub async fn mark_failed(pool: &PgPool, id: i64, error: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE notification_outbox
        SET status = CASE WHEN attempts >= $2 THEN 'failed' ELSE 'pending' END,
            last_error = $3,
            claimed_at = NULL,
            updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .bind(MAX_DELIVERY_ATTEMPTS)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

/// Deliver a single claimed outbox row by inserting it into `notifications`.
/// The unique `(user_address, pool_id, notif_type)` index makes this idempotent,
/// so a redelivery after a crash never double-notifies a user.
async fn deliver_outbox_row(pool: &PgPool, row: &OutboxRow) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO notifications (user_address, notif_type, title, message, pool_id)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (user_address, pool_id, notif_type) DO NOTHING
        "#,
    )
    .bind(&row.user_address)
    .bind(&row.notif_type)
    .bind(&row.title)
    .bind(&row.message)
    .bind(row.pool_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Claim and deliver pending outbox rows. Returns the number of rows delivered.
#[instrument(skip(pool), name = "notifications.process_outbox")]
pub async fn process_outbox(pool: &PgPool, limit: i64) -> Result<usize, sqlx::Error> {
    let rows = claim_outbox(pool, limit).await?;
    let mut delivered = 0usize;

    for row in &rows {
        match deliver_outbox_row(pool, row).await {
            Ok(()) => {
                mark_delivered(pool, row.id).await?;
                delivered += 1;
            }
            Err(error) => {
                warn!(outbox_id = row.id, error = %error, "notification delivery failed");
                mark_failed(pool, row.id, &error.to_string()).await?;
            }
        }
    }

    Ok(delivered)
}

/// List `address`'s notifications, newest first.
pub async fn list_notifications(
    pool: &PgPool,
    address: &str,
    unread_only: bool,
    limit: i64,
    offset: i64,
) -> Result<Vec<NotificationRow>, sqlx::Error> {
    let sql = r#"
        SELECT id, user_address, notif_type, title, message, pool_id, read, created_at
        FROM notifications
        WHERE user_address = $1 AND ($2 = FALSE OR NOT read)
        ORDER BY created_at DESC
        LIMIT $3 OFFSET $4
    "#;

    sqlx::query_as::<_, NotificationRow>(sql)
        .bind(address)
        .bind(unread_only)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
}

/// Count `address`'s unread notifications (for a badge count).
pub async fn count_unread(pool: &PgPool, address: &str) -> Result<i64, sqlx::Error> {
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM notifications WHERE user_address = $1 AND NOT read")
            .bind(address)
            .fetch_one(pool)
            .await?;
    Ok(count.0)
}

/// Mark specific notification `ids` as read, or every notification for
/// `address` when `ids` is `None`. Returns the number of rows updated.
pub async fn mark_read(
    pool: &PgPool,
    address: &str,
    ids: Option<&[i64]>,
) -> Result<u64, sqlx::Error> {
    let result = match ids {
        Some(ids) => {
            sqlx::query(
                "UPDATE notifications SET read = TRUE WHERE user_address = $1 AND id = ANY($2)",
            )
            .bind(address)
            .bind(ids)
            .execute(pool)
            .await?
        }
        None => {
            sqlx::query("UPDATE notifications SET read = TRUE WHERE user_address = $1")
                .bind(address)
                .execute(pool)
                .await?
        }
    };

    Ok(result.rows_affected())
}

/// Interests `address` has registered (matched against pool category + tags
/// for "new pool matching your interests" alerts).
pub async fn get_user_interests(pool: &PgPool, address: &str) -> Result<Vec<String>, sqlx::Error> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT interest FROM user_interests WHERE user_address = $1 ORDER BY interest")
            .bind(address)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(i,)| i).collect())
}

/// Replace `address`'s interest list wholesale.
pub async fn set_user_interests(
    pool: &PgPool,
    address: &str,
    interests: &[String],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM user_interests WHERE user_address = $1")
        .bind(address)
        .execute(&mut *tx)
        .await?;

    for interest in interests {
        sqlx::query(
            "INSERT INTO user_interests (user_address, interest) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(address)
        .bind(interest)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Request body for `PUT /api/v1/users/:address/interests`.
#[derive(Debug, Deserialize)]
pub struct SetInterestsRequest {
    pub interests: Vec<String>,
}

// ── Sweep: generates notifications from current pool/prediction state ────────

#[derive(Debug, Default, Serialize)]
pub struct SweepSummary {
    pub pools_ending_soon: usize,
    pub pools_resolved: usize,
    pub claims_expiring: usize,
    pub new_pool_matches: usize,
}

#[derive(FromRow)]
struct PoolRef {
    pool_id: i64,
    name: String,
}

#[derive(FromRow)]
struct UserAddr {
    user_address: String,
}

/// Scan current pool/prediction state and create any notifications that are
/// now due. Safe to call on a fixed interval — every insert is deduplicated
/// by the unique `(user_address, pool_id, notif_type)` index.
#[instrument(skip(pool), name = "notifications.run_sweep")]
pub async fn run_notification_sweep(pool: &PgPool) -> Result<SweepSummary, sqlx::Error> {
    let mut summary = SweepSummary::default();

    // 1. Pools ending soon → notify everyone with a stake in them.
    let ending_soon: Vec<PoolRef> = sqlx::query_as(&format!(
        r#"
        SELECT pool_id, name FROM pools
        WHERE state = 'active'
          AND end_time BETWEEN NOW() AND NOW() + INTERVAL '{POOL_ENDING_SOON_WINDOW}'
        "#
    ))
    .fetch_all(pool)
    .await?;

    for p in &ending_soon {
        let users: Vec<UserAddr> = sqlx::query_as(
            "SELECT DISTINCT user_address FROM predictions WHERE pool_id = $1",
        )
        .bind(p.pool_id)
        .fetch_all(pool)
        .await?;

        for u in &users {
            let mut tx = pool.begin().await?;
            enqueue_notification(
                &mut tx,
                &u.user_address,
                NotifType::PoolEndingSoon,
                "Pool ending soon",
                &format!("\"{}\" closes within the hour.", p.name),
                p.pool_id,
            )
            .await?;
            tx.commit().await?;
            summary.pools_ending_soon += 1;
        }
    }

    // 2. Resolved pools → notify everyone who predicted on them.
    let resolved: Vec<PoolRef> = sqlx::query_as(
        "SELECT pool_id, name FROM pools WHERE state = 'settled' AND result IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;

    for p in &resolved {
        let users: Vec<UserAddr> = sqlx::query_as(
            "SELECT DISTINCT user_address FROM predictions WHERE pool_id = $1",
        )
        .bind(p.pool_id)
        .fetch_all(pool)
        .await?;

        for u in &users {
            let mut tx = pool.begin().await?;
            enqueue_notification(
                &mut tx,
                &u.user_address,
                NotifType::PoolResolved,
                "Pool resolved",
                &format!("\"{}\" has been resolved. Check your result.", p.name),
                p.pool_id,
            )
            .await?;
            tx.commit().await?;
            summary.pools_resolved += 1;
        }
    }

    // 3. Claim windows expiring soon → notify unclaimed winners.
    let expiring: Vec<PoolRef> = sqlx::query_as(&format!(
        r#"
        SELECT pool_id, name FROM pools
        WHERE state = 'settled'
          AND resolved_at IS NOT NULL
          AND resolved_at + (claim_window_seconds || ' seconds')::interval
              BETWEEN NOW() AND NOW() + INTERVAL '{CLAIM_EXPIRING_WINDOW}'
        "#
    ))
    .fetch_all(pool)
    .await?;

    for p in &expiring {
        let winners: Vec<UserAddr> = sqlx::query_as(
            r#"
            SELECT DISTINCT p.user_address
            FROM predictions p
            JOIN pools pl ON pl.pool_id = p.pool_id
            WHERE p.pool_id = $1
              AND NOT p.claimed
              AND pl.result IS NOT NULL AND pl.result ~ '^\d+$'
              AND pl.result::int = p.outcome
            "#,
        )
        .bind(p.pool_id)
        .fetch_all(pool)
        .await?;

        for u in &winners {
            let mut tx = pool.begin().await?;
            enqueue_notification(
                &mut tx,
                &u.user_address,
                NotifType::ClaimExpiring,
                "Claim window closing soon",
                &format!("Your winnings from \"{}\" must be claimed soon.", p.name),
                p.pool_id,
            )
            .await?;
            tx.commit().await?;
            summary.claims_expiring += 1;
        }
    }

    // 4. New pools matching a user's registered interests.
    #[derive(FromRow)]
    struct NewPool {
        pool_id: i64,
        name: String,
        category: String,
        tags: Vec<String>,
    }
    let new_pools: Vec<NewPool> = sqlx::query_as(&format!(
        r#"
        SELECT pool_id, name, category, tags FROM pools
        WHERE created_at > NOW() - INTERVAL '{NEW_POOL_LOOKBACK}'
        "#
    ))
    .fetch_all(pool)
    .await?;

    for p in &new_pools {
        let mut interests = p.tags.clone();
        interests.push(p.category.clone());

        let matched_users: Vec<UserAddr> = sqlx::query_as(
            "SELECT DISTINCT user_address FROM user_interests WHERE interest = ANY($1)",
        )
        .bind(&interests)
        .fetch_all(pool)
        .await?;

        for u in &matched_users {
            let mut tx = pool.begin().await?;
            enqueue_notification(
                &mut tx,
                &u.user_address,
                NotifType::NewPoolMatch,
                "New pool matching your interests",
                &format!("\"{}\" was just created in {}.", p.name, p.category),
                p.pool_id,
            )
            .await?;
            tx.commit().await?;
            summary.new_pool_matches += 1;
        }
    }

    info!(
        pools_ending_soon = summary.pools_ending_soon,
        pools_resolved = summary.pools_resolved,
        claims_expiring = summary.claims_expiring,
        new_pool_matches = summary.new_pool_matches,
        "notification sweep complete"
    );

    Ok(summary)
}

/// Run the outbox delivery worker on a fixed interval until the process exits.
/// Errors are logged and skipped rather than aborting the loop.
pub async fn run_outbox_loop(pool: PgPool, interval: std::time::Duration) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        if let Err(error) = process_outbox(&pool, 100).await {
            warn!(error = %error, "notification outbox processing failed");
        }
    }
}

/// Run [`run_notification_sweep`] on a fixed interval until the process exits.
/// Errors are logged and skipped rather than aborting the loop — a single
/// failed sweep shouldn't take the background worker down.
pub async fn run_sweep_loop(pool: PgPool, interval: std::time::Duration) {
    // Crash recovery (#63): before the first tick, re-load any outbound
    // deliveries that were `pending` when the process last exited and whose
    // `next_attempt_at` has since elapsed. Errors are logged and skipped so a
    // broken DB does not block the sweep forever — the next tick will retry.
    match crate::sinks::recover_pending_deliveries(&pool).await {
        Ok(pending) if !pending.is_empty() => {
            info!(
                count = pending.len(),
                "recovered pending outbound deliveries on startup"
            );
            for d in &pending {
                crate::sinks::enqueue_delivery(d);
            }
        }
        Ok(_) => {}
        Err(error) => {
            warn!(error = %error, "outbound delivery recovery failed; will retry on next tick");
        }
    }

    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        if let Err(error) = run_notification_sweep(&pool).await {
            warn!(error = %error, "notification sweep failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notif_type_strings_match_migration_check_constraint() {
        assert_eq!(NotifType::PoolEndingSoon.as_str(), "pool_ending_soon");
        assert_eq!(NotifType::PoolResolved.as_str(), "pool_resolved");
        assert_eq!(NotifType::ClaimExpiring.as_str(), "claim_expiring");
        assert_eq!(NotifType::NewPoolMatch.as_str(), "new_pool_match");
    }

    // ── Credential rotation tests (#355) ────────────────────────────────────

    use std::sync::Mutex;

    /// Mock provider that records deliveries and rejects revoked credentials.
    #[derive(Default)]
    struct MockProvider {
        delivered: Mutex<Vec<(String, String)>>,
    }

    impl MockProvider {
        fn delivered(&self) -> Vec<(String, String)> {
            self.delivered.lock().unwrap().clone()
        }
    }

    impl NotificationProvider for MockProvider {
        fn deliver(
            &self,
            credential: &ProviderCredential,
            recipient: &str,
            message: &str,
        ) -> Result<(), ProviderError> {
            if credential.state != CredentialState::Active {
                return Err(ProviderError::Revoked);
            }
            self.delivered
                .lock()
                .unwrap()
                .push((recipient.to_string(), message.to_string()));
            Ok(())
        }
    }

    fn cred(key_id: &str, state: CredentialState) -> ProviderCredential {
        ProviderCredential {
            key_id: key_id.to_string(),
            secret: format!("secret-{key_id}"),
            state,
        }
    }

    #[test]
    fn revoked_credential_cannot_deliver() {
        let provider = MockProvider::default();
        let set = CredentialSet::single(cred("old-key", CredentialState::Revoked));

        let result = deliver_with_credentials(&provider, &set, "0xabc", "hello");
        assert!(matches!(
            result,
            Err(RotationError::RevokedCredential { .. })
        ));
        assert!(provider.delivered().is_empty());
    }

    #[test]
    fn hard_cutover_revokes_old_credential_and_audits() {
        let audit = InMemoryRotationAudit::new();
        let mut set = CredentialSet::single(cred("old-key", CredentialState::Active));

        let outcome =
            rotate_credential(&mut set, cred("new-key", CredentialState::Active), "ops", &audit)
                .expect("rotation succeeds");
        assert_eq!(outcome, RotationOutcome::Completed);
        assert_eq!(set.active.key_id, "new-key");
        assert!(set.previous.is_none());
        assert!(!set.accepts("old-key"));

        let entries = audit.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].old_key_id, "old-key");
        assert_eq!(entries[0].new_key_id, "new-key");
        assert_eq!(entries[0].actor, "ops");
        assert_eq!(entries[0].outcome, RotationOutcome::Completed);

        // The revoked credential can no longer deliver.
        let provider = MockProvider::default();
        let revoked = CredentialSet::single(cred("old-key", CredentialState::Revoked));
        assert!(deliver_with_credentials(&provider, &revoked, "0xabc", "hi").is_err());
        assert!(provider.delivered().is_empty());

        // The replacement credential delivers.
        assert!(deliver_with_credentials(&provider, &set, "0xabc", "hi").is_ok());
        assert_eq!(provider.delivered().len(), 1);
    }

    #[test]
    fn dual_key_transition_retains_old_key_only_when_supported() {
        let audit = InMemoryRotationAudit::new();
        let mut set = CredentialSet::single(cred("old-key", CredentialState::Active));
        set.dual_key = DualKeySupport::Supported;

        let outcome =
            rotate_credential(&mut set, cred("new-key", CredentialState::Active), "ops", &audit)
                .expect("rotation succeeds");
        assert_eq!(outcome, RotationOutcome::DualKeyTransition);
        assert_eq!(set.active.key_id, "new-key");
        assert_eq!(set.previous.as_ref().unwrap().key_id, "old-key");
        assert_eq!(set.previous.as_ref().unwrap().state, CredentialState::Retired);
        assert!(set.accepts("old-key"));
        assert!(set.accepts("new-key"));

        let entries = audit.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].outcome, RotationOutcome::DualKeyTransition);
    }

    #[test]
    fn rejected_rotation_is_audited() {
        let audit = InMemoryRotationAudit::new();
        let mut set = CredentialSet::single(cred("old-key", CredentialState::Active));

        let result =
            rotate_credential(&mut set, cred("old-key", CredentialState::Active), "ops", &audit);
        assert_eq!(result, Err(RotationError::SameCredential));
        assert_eq!(set.active.key_id, "old-key");

        let entries = audit.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].outcome, RotationOutcome::Rejected);
        assert!(entries[0].detail.is_some());
    }
}
