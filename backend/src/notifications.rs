//! In-app notification system.
//!
//! Alerts users when: their pool is about to end, a pool they predicted on
//! is resolved, their claim window is about to expire, and when new pools
//! matching their interests are created.
//!
//! Delivery uses a transactional outbox: notification intents are written to
//! `user_notification_outbox` in the same transaction as the domain mutation that
//! triggers them, then a background worker claims, delivers, and retries them.
//! This guarantees that rolled-back domain writes emit no notification and
//! that committed writes eventually deliver exactly once (logically).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, Postgres, Row, Transaction};
use std::{collections::BTreeMap, sync::Arc};
use tracing::{info, instrument, warn};

/// How far ahead of `end_time` a pool counts as "ending soon".
const POOL_ENDING_SOON_WINDOW: &str = "1 hour";
/// How far ahead of claim-window expiry a user is warned.
const CLAIM_EXPIRING_WINDOW: &str = "1 day";
/// How far back to scan for newly created pools when matching interests.
const NEW_POOL_LOOKBACK: &str = "1 day";
// ── Retention and deletion policy for private user records ──────────────────
//
// Classification of user-linked fields:
//
// | Data class                  | Examples                                 | Retention                     | Deletion behavior                              |
// |-----------------------------|------------------------------------------|-------------------------------|------------------------------------------------|
// | Delivered notifications     | user_notification_outbox rows in `delivered` | 30 days after delivery   | Hard delete; recipient id scrubbed first       |
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
        UPDATE user_notification_outbox
        SET user_address = ''
        WHERE status IN ('delivered', 'failed')
          AND user_address <> ''
          AND updated_at < now() - $1::interval
        "#,
    )
    .bind(OUTBOX_TERMINAL_RETENTION)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    let purged = sqlx::query(
        r#"
        DELETE FROM user_notification_outbox
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
        ON CONFLICT (user_address, pool_id, notif_type)
            WHERE pool_id IS NOT NULL DO NOTHING
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
    let event_key = format!("pool:{}:{}", pool_id, notif_type.as_str());
    enqueue_notification_for_event(
        tx,
        user_address,
        notif_type,
        title,
        message,
        pool_id,
        &event_key,
    )
    .await
}

/// Enqueue a notification intent tied to a stable domain-event identity.
/// The `(event_key, recipient)` unique constraint deduplicates retries of the
/// same event without merging unrelated events for the same wallet.
pub async fn enqueue_notification_for_event(
    tx: &mut Transaction<'_, Postgres>,
    user_address: &str,
    notif_type: NotifType,
    title: &str,
    message: &str,
    pool_id: i64,
    event_key: &str,
) -> Result<(), sqlx::Error> {
    let stored_preferences: Option<(i16, Value)> = sqlx::query_as(
        "SELECT schema_version, preferences FROM notification_preferences WHERE user_address = $1",
    )
    .bind(user_address)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((version, value)) = stored_preferences else {
        return Ok(());
    };
    let preferences = migrate_preferences(version, value).map_err(sqlx::Error::Protocol)?;
    if !preferences.channels.get("in_app").copied().unwrap_or(false)
        || !preferences
            .events
            .get(notif_type.as_str())
            .copied()
            .unwrap_or(false)
    {
        return Ok(());
    }

    sqlx::query(
        r#"
        INSERT INTO user_notification_outbox
            (event_key, user_address, notif_type, title, message, pool_id)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (event_key, user_address) DO NOTHING
        "#,
    )
    .bind(event_key)
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
    pub claim_token: String,
}

/// Claim up to `limit` pending outbox rows for delivery, leasing them so that
/// concurrent workers do not process the same row. Rows whose lease has expired
/// (e.g. after a process crash) become claimable again.
#[instrument(skip(pool), name = "notifications.claim_outbox")]
pub async fn claim_outbox(pool: &PgPool, limit: i64) -> Result<Vec<OutboxRow>, sqlx::Error> {
    let claim_token = uuid::Uuid::new_v4().to_string();
    let rows: Vec<OutboxRow> = sqlx::query_as(
        r#"
        WITH exhausted AS (
            UPDATE user_notification_outbox
            SET status = 'failed', claimed_at = NULL, claimed_by = NULL,
                updated_at = NOW()
            WHERE attempts >= max_attempts
              AND (
                  (status = 'pending' AND next_attempt_at <= NOW())
                  OR (status = 'processing'
                      AND claimed_at < NOW() - ($3::bigint * INTERVAL '1 second'))
              )
            RETURNING id
        ),
        candidates AS (
            SELECT id
            FROM user_notification_outbox
            WHERE (
                (status = 'pending' AND next_attempt_at <= NOW())
                OR (status = 'processing'
                    AND claimed_at < NOW() - ($3::bigint * INTERVAL '1 second'))
            )
              AND attempts < max_attempts
            ORDER BY id
            LIMIT $1
            FOR UPDATE SKIP LOCKED
        )
        UPDATE user_notification_outbox AS outbox
        SET status = 'processing',
            attempts = outbox.attempts + 1,
            claimed_at = NOW(),
            claimed_by = $2,
            updated_at = NOW()
        FROM candidates
        WHERE outbox.id = candidates.id
        RETURNING outbox.id, outbox.user_address, outbox.notif_type,
                  outbox.title, outbox.message, outbox.pool_id,
                  outbox.attempts, outbox.claimed_by AS claim_token
        "#,
    )
    .bind(limit.clamp(1, 100))
    .bind(claim_token)
    .bind(CLAIM_LEASE_SECONDS)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Record a failed delivery attempt. Rows that exceed their stored attempt budget
/// transition to the terminal `failed` state; others return to `pending` for a
/// later retry.
pub async fn mark_failed(
    pool: &PgPool,
    id: i64,
    claim_token: &str,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE user_notification_outbox
        SET status = CASE WHEN attempts >= max_attempts THEN 'failed' ELSE 'pending' END,
            last_error = $2,
            claimed_at = NULL,
            claimed_by = NULL,
            next_attempt_at = CASE
                WHEN attempts >= max_attempts THEN next_attempt_at
                ELSE NOW() + LEAST(300, POWER(2::numeric, LEAST(attempts - 1, 8))::integer)
                    * INTERVAL '1 second'
            END,
            updated_at = NOW()
        WHERE id = $1 AND status = 'processing' AND claimed_by = $3
        "#,
    )
    .bind(id)
    .bind(error)
    .bind(claim_token)
    .execute(pool)
    .await?;
    Ok(())
}

/// Deliver a single claimed outbox row by inserting it into `notifications`.
/// The unique `(user_address, pool_id, notif_type)` index makes this idempotent,
/// so a redelivery after a crash never double-notifies a user.
async fn deliver_outbox_row(pool: &PgPool, row: &OutboxRow) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"
        INSERT INTO notifications (user_address, notif_type, title, message, pool_id)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (user_address, pool_id, notif_type)
            WHERE pool_id IS NOT NULL DO NOTHING
        "#,
    )
    .bind(&row.user_address)
    .bind(&row.notif_type)
    .bind(&row.title)
    .bind(&row.message)
    .bind(row.pool_id)
    .execute(&mut *tx)
    .await?;
    let updated = sqlx::query(
        r#"
        UPDATE user_notification_outbox
        SET status = 'delivered', claimed_at = NULL, claimed_by = NULL, updated_at = NOW()
        WHERE id = $1 AND status = 'processing' AND claimed_by = $2
        "#,
    )
    .bind(row.id)
    .bind(&row.claim_token)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if updated != 1 {
        return Err(sqlx::Error::RowNotFound);
    }
    tx.commit().await?;
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
                delivered += 1;
            }
            Err(error) => {
                warn!(outbox_id = row.id, error = %error, "notification delivery failed");
                mark_failed(pool, row.id, &row.claim_token, &error.to_string()).await?;
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
        ORDER BY created_at DESC, id DESC
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

/// Cursor-paginated inbox response with an unread count from the same
/// repeatable-read snapshot as the notification page.
#[derive(Debug, Clone, Serialize)]
pub struct InboxPage {
    pub notifications: Vec<NotificationRow>,
    pub unread_count: i64,
    pub next_cursor: Option<String>,
}

/// List one deterministic reverse-chronological inbox page.
pub async fn list_notifications_page(
    pool: &PgPool,
    address: &str,
    unread_only: bool,
    limit: i64,
    cursor: Option<&str>,
) -> Result<InboxPage, sqlx::Error> {
    let cursor = cursor
        .map(decode_notification_cursor)
        .transpose()
        .map_err(|_| sqlx::Error::Protocol("invalid notification cursor".to_string()))?;
    let limit = limit.clamp(1, 100);
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;
    let unread_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM notifications WHERE user_address = $1 AND NOT read")
            .bind(address)
            .fetch_one(&mut *tx)
            .await?;
    let (cursor_created_at, cursor_id) = cursor.map_or((None, None), |(created_at, id)| {
        (Some(created_at), Some(id))
    });
    let mut rows = sqlx::query_as::<_, NotificationRow>(
        r#"
        SELECT id, user_address, notif_type, title, message, pool_id, read, created_at
        FROM notifications
        WHERE user_address = $1
          AND ($2 = FALSE OR NOT read)
          AND ($3::timestamptz IS NULL OR (created_at, id) < ($3, $4))
        ORDER BY created_at DESC, id DESC
        LIMIT $5
        "#,
    )
    .bind(address)
    .bind(unread_only)
    .bind(cursor_created_at)
    .bind(cursor_id)
    .bind(limit + 1)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = if has_more {
        rows.last().map(encode_notification_cursor)
    } else {
        None
    };
    Ok(InboxPage {
        notifications: rows,
        unread_count: unread_count.0,
        next_cursor,
    })
}

fn encode_notification_cursor(row: &NotificationRow) -> String {
    format!(
        "{:016x}{:016x}",
        row.created_at.timestamp_micros() as u64,
        row.id as u64
    )
}

fn decode_notification_cursor(cursor: &str) -> Result<(DateTime<Utc>, i64), ()> {
    if cursor.len() != 32 || !cursor.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(());
    }
    let timestamp = i64::from_str_radix(&cursor[..16], 16).map_err(|_| ())? as u64 as i64;
    let id = i64::from_str_radix(&cursor[16..], 16).map_err(|_| ())? as u64 as i64;
    if id <= 0 {
        return Err(());
    }
    DateTime::from_timestamp_micros(timestamp)
        .map(|created_at| (created_at, id))
        .ok_or(())
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
                "UPDATE notifications SET read = TRUE WHERE user_address = $1 AND id = ANY($2) AND NOT read",
            )
            .bind(address)
            .bind(ids)
            .execute(pool)
            .await?
        }
        None => {
            sqlx::query("UPDATE notifications SET read = TRUE WHERE user_address = $1 AND NOT read")
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

// ── Wallet-scoped notification preferences (#301) ────────────────────────────

pub const NOTIFICATION_PREFERENCES_SCHEMA_VERSION: i16 = 2;
pub const MAX_NOTIFICATION_PREFERENCES_BODY_BYTES: usize = 16 * 1024;
pub const MAX_PUSH_SUBSCRIPTION_BODY_BYTES: usize = 16 * 1024;
const PREFERENCE_CHANNELS: &[&str] = &["in_app", "push", "email"];
const PREFERENCE_EVENTS: &[&str] = &[
    "pool_ending_soon",
    "pool_resolved",
    "claim_expiring",
    "new_pool_match",
];

/// Opt-in schedule for the weekly market digest (#289).
///
/// The digest is disabled by default. When enabled, `day_of_week` (0 = Monday,
/// 6 = Sunday), `send_time_local` (`HH:MM`), and `timezone` (IANA name) define
/// the local send window. The next send instant is computed DST-safely.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DigestPreferences {
    #[serde(default)]
    pub enabled: bool,
    /// 0 = Monday … 6 = Sunday.
    #[serde(default)]
    pub day_of_week: u8,
    /// Local wall-clock send time, `HH:MM`.
    #[serde(default = "default_digest_time")]
    pub send_time_local: String,
    /// IANA timezone name, e.g. `Europe/London`.
    #[serde(default = "default_digest_timezone")]
    pub timezone: String,
}

fn default_digest_time() -> String {
    "09:00".to_string()
}

fn default_digest_timezone() -> String {
    "UTC".to_string()
}

impl Default for DigestPreferences {
    fn default() -> Self {
        Self {
            enabled: false,
            day_of_week: 0,
            send_time_local: default_digest_time(),
            timezone: default_digest_timezone(),
        }
    }
}

/// Versioned, default-deny preferences for one wallet.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct NotificationPreferences {
    pub schema_version: i16,
    #[serde(default)]
    pub channels: BTreeMap<String, bool>,
    #[serde(default)]
    pub events: BTreeMap<String, bool>,
    #[serde(default)]
    pub digest: DigestPreferences,
}

impl Default for NotificationPreferences {
    fn default() -> Self {
        Self {
            schema_version: NOTIFICATION_PREFERENCES_SCHEMA_VERSION,
            channels: default_disabled(PREFERENCE_CHANNELS),
            events: default_disabled(PREFERENCE_EVENTS),
            digest: DigestPreferences::default(),
        }
    }
}

fn default_disabled(names: &[&str]) -> BTreeMap<String, bool> {
    names
        .iter()
        .map(|name| ((*name).to_string(), false))
        .collect()
}

fn normalize_preferences(mut preferences: NotificationPreferences) -> NotificationPreferences {
    let channels = PREFERENCE_CHANNELS
        .iter()
        .map(|name| {
            (
                (*name).to_string(),
                preferences.channels.get(*name).copied().unwrap_or(false),
            )
        })
        .collect();
    let events = PREFERENCE_EVENTS
        .iter()
        .map(|name| {
            (
                (*name).to_string(),
                preferences.events.get(*name).copied().unwrap_or(false),
            )
        })
        .collect();
    preferences.schema_version = NOTIFICATION_PREFERENCES_SCHEMA_VERSION;
    preferences.channels = channels;
    preferences.events = events;
    preferences.digest = normalize_digest(preferences.digest);
    preferences
}

/// Clamp digest schedule fields into valid ranges so a malformed stored value
/// can never produce an unschedulable digest.
fn normalize_digest(mut digest: DigestPreferences) -> DigestPreferences {
    if digest.day_of_week > 6 {
        digest.day_of_week = 0;
    }
    if parse_local_time(&digest.send_time_local).is_none() {
        digest.send_time_local = default_digest_time();
    }
    if digest.timezone.parse::<chrono_tz::Tz>().is_err() {
        digest.timezone = default_digest_timezone();
    }
    digest
}

/// Parse an `HH:MM` local time into `(hour, minute)`.
fn parse_local_time(value: &str) -> Option<(u32, u32)> {
    let (hour, minute) = value.split_once(':')?;
    if hour.len() != 2 || minute.len() != 2 {
        return None;
    }
    let hour: u32 = hour.parse().ok()?;
    let minute: u32 = minute.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute))
}

/// Migrate the original unversioned/flat preference object to schema v1.
/// Unknown fields are discarded, so new channels and event types stay disabled.
fn migrate_preferences(
    schema_version: i16,
    value: Value,
) -> Result<NotificationPreferences, String> {
    if schema_version > NOTIFICATION_PREFERENCES_SCHEMA_VERSION || schema_version < 0 {
        return Err(format!(
            "unsupported notification preference schema version {schema_version}"
        ));
    }
    if schema_version == NOTIFICATION_PREFERENCES_SCHEMA_VERSION {
        let preferences: NotificationPreferences =
            serde_json::from_value(value).map_err(|error| error.to_string())?;
        return Ok(normalize_preferences(preferences));
    }

    // v1 → v2: the digest block did not exist, so it stays disabled. Preserve
    // the v1 channels/events maps and let normalization fill in defaults.
    if schema_version == 1 {
        let mut preferences: NotificationPreferences =
            serde_json::from_value(value).map_err(|error| error.to_string())?;
        preferences.schema_version = NOTIFICATION_PREFERENCES_SCHEMA_VERSION;
        preferences.digest = DigestPreferences::default();
        return Ok(normalize_preferences(preferences));
    }

    let empty = serde_json::Map::new();
    let object = value.as_object().unwrap_or(&empty);
    let read_flags = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_object)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|(name, value)| value.as_bool().map(|enabled| (name.clone(), enabled)))
                    .collect()
            })
            .unwrap_or_default()
    };
    let events = if object.contains_key("events") {
        read_flags("events")
    } else {
        object
            .iter()
            .filter_map(|(name, value)| {
                value.as_bool().map(|enabled| (name.clone(), enabled))
            })
            .collect()
    };
    Ok(normalize_preferences(NotificationPreferences {
        schema_version: NOTIFICATION_PREFERENCES_SCHEMA_VERSION,
        channels: read_flags("channels"),
        events,
    }))
}

pub async fn get_notification_preferences(
    pool: &PgPool,
    address: &str,
) -> Result<NotificationPreferences, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let row: Option<(i16, Value)> = sqlx::query_as(
        "SELECT schema_version, preferences FROM notification_preferences WHERE user_address = $1 FOR UPDATE",
    )
    .bind(address)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((version, value)) = row else {
        tx.commit().await?;
        return Ok(NotificationPreferences::default());
    };
    let migrated =
        migrate_preferences(version, value).map_err(|error| sqlx::Error::Protocol(error))?;
    if version != NOTIFICATION_PREFERENCES_SCHEMA_VERSION {
        let json = serde_json::to_value(&migrated)
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        sqlx::query(
            "UPDATE notification_preferences SET schema_version = $2, preferences = $3, updated_at = NOW() WHERE user_address = $1",
        )
        .bind(address)
        .bind(NOTIFICATION_PREFERENCES_SCHEMA_VERSION)
        .bind(json)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(migrated)
}

pub async fn save_notification_preferences(
    pool: &PgPool,
    address: &str,
    preferences: NotificationPreferences,
) -> Result<NotificationPreferences, PreferenceUpdateError> {
    if preferences.schema_version != NOTIFICATION_PREFERENCES_SCHEMA_VERSION {
        return Err(PreferenceUpdateError::InvalidSchemaVersion);
    }
    let preferences = normalize_preferences(preferences);
    let value = serde_json::to_value(&preferences)
        .map_err(|error| PreferenceUpdateError::Serialization(error.to_string()))?;
    sqlx::query(
        r#"
        INSERT INTO notification_preferences (user_address, schema_version, preferences)
        VALUES ($1, $2, $3)
        ON CONFLICT (user_address) DO UPDATE
        SET schema_version = EXCLUDED.schema_version,
            preferences = EXCLUDED.preferences,
            updated_at = NOW()
        "#,
    )
    .bind(address)
    .bind(NOTIFICATION_PREFERENCES_SCHEMA_VERSION)
    .bind(value)
    .execute(pool)
    .await
    .map_err(PreferenceUpdateError::Database)?;
    Ok(preferences)
}

#[derive(Debug)]
pub enum PreferenceUpdateError {
    InvalidSchemaVersion,
    Serialization(String),
    Database(sqlx::Error),
}

pub async fn reset_notification_preferences(
    pool: &PgPool,
    address: &str,
) -> Result<NotificationPreferences, sqlx::Error> {
    sqlx::query("DELETE FROM notification_preferences WHERE user_address = $1")
        .bind(address)
        .execute(pool)
        .await?;
    Ok(NotificationPreferences::default())
}

// ── Browser push subscription management (#304) ──────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct PushSubscriptionRequest {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub expires_at: Option<DateTime<Utc>>,
    /// Previous endpoint to revoke atomically when the browser rotates it.
    pub previous_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PushSubscriptionSummary {
    pub id: i64,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PushSubscriptionPayload {
    endpoint: String,
    p256dh: String,
    auth: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivePushSubscription {
    pub id: i64,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug)]
pub enum PushSubscriptionError {
    InvalidInput(&'static str),
    EndpointInUse,
    Encryption,
    Database(sqlx::Error),
}

impl From<sqlx::Error> for PushSubscriptionError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

fn validate_push_endpoint(endpoint: &str) -> Result<(), PushSubscriptionError> {
    let parsed = reqwest::Url::parse(endpoint)
        .map_err(|_| PushSubscriptionError::InvalidInput("endpoint must be a valid HTTPS URL"))?;
    if endpoint.len() > 2048
        || parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(PushSubscriptionError::InvalidInput(
            "endpoint must be an HTTPS URL without embedded credentials",
        ));
    }
    Ok(())
}

fn push_endpoint_hash(endpoint: &str) -> String {
    let digest = Sha256::digest(endpoint.as_bytes());
    const HEX: &[u8; 16] = b"0123456789abcdef";
    digest
        .iter()
        .flat_map(|byte| {
            [
                HEX[(byte >> 4) as usize] as char,
                HEX[(byte & 0x0f) as usize] as char,
            ]
        })
        .collect()
}

fn push_encryption_key(secret: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"predifi:browser-push:v1:");
    hasher.update(secret.as_bytes());
    hasher.finalize().into()
}

fn encrypt_push_payload(secret: &str, payload: &[u8]) -> Result<Vec<u8>, PushSubscriptionError> {
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
    use ring::rand::{SecureRandom, SystemRandom};

    let key = UnboundKey::new(&AES_256_GCM, &push_encryption_key(secret))
        .map(LessSafeKey::new)
        .map_err(|_| PushSubscriptionError::Encryption)?;
    let mut nonce_bytes = [0u8; 12];
    SystemRandom::new()
        .fill(&mut nonce_bytes)
        .map_err(|_| PushSubscriptionError::Encryption)?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let mut ciphertext = payload.to_vec();
    key.seal_in_place_append_tag(nonce, Aad::empty(), &mut ciphertext)
        .map_err(|_| PushSubscriptionError::Encryption)?;
    let mut stored = nonce_bytes.to_vec();
    stored.extend(ciphertext);
    Ok(stored)
}

fn decrypt_push_payload(secret: &str, stored: &[u8]) -> Result<Vec<u8>, PushSubscriptionError> {
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};

    if stored.len() < 12 + 16 {
        return Err(PushSubscriptionError::Encryption);
    }
    let key = UnboundKey::new(&AES_256_GCM, &push_encryption_key(secret))
        .map(LessSafeKey::new)
        .map_err(|_| PushSubscriptionError::Encryption)?;
    let nonce_bytes: [u8; 12] = stored[..12]
        .try_into()
        .map_err(|_| PushSubscriptionError::Encryption)?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let mut plaintext = stored[12..].to_vec();
    let cleartext = key
        .open_in_place(nonce, Aad::empty(), &mut plaintext)
        .map_err(|_| PushSubscriptionError::Encryption)?;
    Ok(cleartext.to_vec())
}

pub async fn save_push_subscription(
    pool: &PgPool,
    address: &str,
    request: PushSubscriptionRequest,
    secret: &str,
) -> Result<PushSubscriptionSummary, PushSubscriptionError> {
    validate_push_endpoint(&request.endpoint)?;
    if request.p256dh.is_empty()
        || request.auth.is_empty()
        || request.p256dh.len() > 256
        || request.auth.len() > 256
        || !request
            .p256dh
            .bytes()
            .chain(request.auth.bytes())
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'='))
    {
        return Err(PushSubscriptionError::InvalidInput(
            "push encryption keys are invalid",
        ));
    }
    let now = Utc::now();
    let expires_at = request
        .expires_at
        .unwrap_or_else(|| now + chrono::Duration::days(90));
    if expires_at <= now {
        return Err(PushSubscriptionError::InvalidInput(
            "subscription expiration must be in the future",
        ));
    }
    let payload = serde_json::to_vec(&PushSubscriptionPayload {
        endpoint: request.endpoint.clone(),
        p256dh: request.p256dh,
        auth: request.auth,
    })
    .map_err(|_| PushSubscriptionError::Encryption)?;
    let encrypted = encrypt_push_payload(secret, &payload)?;
    let endpoint_hash = push_endpoint_hash(&request.endpoint);
    let previous_hash = request
        .previous_endpoint
        .as_deref()
        .map(|endpoint| {
            validate_push_endpoint(endpoint)?;
            Ok::<_, PushSubscriptionError>(push_endpoint_hash(endpoint))
        })
        .transpose()?;

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM browser_push_subscriptions WHERE expires_at <= NOW()")
        .execute(&mut *tx)
        .await?;
    if let Some(previous_hash) = previous_hash {
        sqlx::query(
            "DELETE FROM browser_push_subscriptions WHERE user_address = $1 AND endpoint_hash = $2",
        )
        .bind(address)
        .bind(previous_hash)
        .execute(&mut *tx)
        .await?;
    }
    let inserted: Option<(i64, DateTime<Utc>)> = sqlx::query_as(
        r#"
        INSERT INTO browser_push_subscriptions
            (user_address, endpoint_hash, encrypted_payload, expires_at)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (endpoint_hash) DO UPDATE
        SET encrypted_payload = EXCLUDED.encrypted_payload,
            expires_at = EXCLUDED.expires_at,
            updated_at = NOW()
        WHERE browser_push_subscriptions.user_address = EXCLUDED.user_address
        RETURNING id, expires_at
        "#,
    )
    .bind(address)
    .bind(endpoint_hash)
    .bind(encrypted)
    .bind(expires_at)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, expires_at)) = inserted else {
        return Err(PushSubscriptionError::EndpointInUse);
    };
    tx.commit().await?;
    Ok(PushSubscriptionSummary { id, expires_at })
}

pub async fn revoke_push_subscription(
    pool: &PgPool,
    address: &str,
    endpoint: Option<&str>,
) -> Result<u64, PushSubscriptionError> {
    let result = if let Some(endpoint) = endpoint {
        validate_push_endpoint(endpoint)?;
        sqlx::query(
            "DELETE FROM browser_push_subscriptions WHERE user_address = $1 AND endpoint_hash = $2",
        )
        .bind(address)
        .bind(push_endpoint_hash(endpoint))
        .execute(pool)
        .await?
    } else {
        sqlx::query("DELETE FROM browser_push_subscriptions WHERE user_address = $1")
            .bind(address)
            .execute(pool)
            .await?
    };
    Ok(result.rows_affected())
}

pub async fn load_active_push_subscriptions(
    pool: &PgPool,
    address: &str,
    secret: &str,
) -> Result<Vec<ActivePushSubscription>, PushSubscriptionError> {
    sqlx::query("DELETE FROM browser_push_subscriptions WHERE expires_at <= NOW()")
        .execute(pool)
        .await?;
    let rows = sqlx::query(
        "SELECT id, encrypted_payload, expires_at FROM browser_push_subscriptions WHERE user_address = $1 AND expires_at > NOW() ORDER BY id",
    )
    .bind(address)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            let payload = decrypt_push_payload(
                secret,
                &row.try_get::<Vec<u8>, _>("encrypted_payload")
                    .map_err(PushSubscriptionError::Database)?,
            )?;
            let payload: PushSubscriptionPayload =
                serde_json::from_slice(&payload).map_err(|_| PushSubscriptionError::Encryption)?;
            Ok(ActivePushSubscription {
                id: row.try_get("id").map_err(PushSubscriptionError::Database)?,
                endpoint: payload.endpoint,
                p256dh: payload.p256dh,
                auth: payload.auth,
                expires_at: row
                    .try_get("expires_at")
                    .map_err(PushSubscriptionError::Database)?,
            })
        })
        .collect()
}

pub async fn prune_expired_push_subscriptions(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM browser_push_subscriptions WHERE expires_at <= NOW()")
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

// ── Account data export and deletion (#267) ─────────────────────────────────
//
// Off-chain notification data linked to a wallet is exported and deleted here.
// Only private, user-owned records are touched:
//
// * `notification_preferences` — private channel/event opt-ins.
// * `notifications`            — delivered in-app inbox entries.
// * `user_notification_outbox` — pending/terminal delivery intents.
// * `user_interests`           — private interest tags used for matching.
// * `browser_push_subscriptions` — encrypted push endpoints.
//
// Chain-derived history (pools, predictions, resolutions, claims) is public and
// immutable; it is never read or written by these functions. Audit tables
// (`credential_rotation_audit`) hold no personal data and are retained.

/// One exported push subscription. The encrypted payload is decrypted so the
/// user receives the endpoint and keys they registered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportedPushSubscription {
    pub id: i64,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub expires_at: DateTime<Utc>,
}

/// One exported outbox intent (pending or terminal delivery record).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportedOutboxEntry {
    pub id: i64,
    pub event_key: String,
    pub notif_type: String,
    pub title: String,
    pub message: String,
    pub pool_id: i64,
    pub status: String,
    pub attempts: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Complete export of a wallet's off-chain notification data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotificationDataExport {
    pub user_address: String,
    pub exported_at: DateTime<Utc>,
    pub preferences: NotificationPreferences,
    pub notifications: Vec<NotificationRow>,
    pub outbox: Vec<ExportedOutboxEntry>,
    pub interests: Vec<String>,
    pub push_subscriptions: Vec<ExportedPushSubscription>,
    /// Human-readable explanation of what is (and is not) included, so the
    /// user understands that on-chain history is immutable and public.
    pub on_chain_data_note: String,
}

/// Explanation attached to every export describing immutable on-chain data.
pub const ON_CHAIN_DATA_NOTE: &str = "Public on-chain history (pools, predictions, \
resolutions and claims) is immutable and is not part of this export or of account \
deletion. Only private off-chain profile and notification data is exported or removed.";

/// Export every off-chain notification record linked to `address`.
///
/// The read runs in a single repeatable-read transaction so the export is a
/// consistent snapshot. Push subscription payloads are decrypted with `secret`.
pub async fn export_notification_data(
    pool: &PgPool,
    address: &str,
    secret: &str,
) -> Result<NotificationDataExport, PushSubscriptionError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;

    let preferences = get_notification_preferences(pool, address)
        .await
        .map_err(PushSubscriptionError::Database)?;

    let notifications: Vec<NotificationRow> = sqlx::query_as(
        r#"
        SELECT id, user_address, notif_type, title, message, pool_id, read, created_at
        FROM notifications
        WHERE user_address = $1
        ORDER BY created_at ASC, id ASC
        "#,
    )
    .bind(address)
    .fetch_all(&mut *tx)
    .await?;

    let outbox: Vec<ExportedOutboxEntry> = sqlx::query_as(
        r#"
        SELECT id, event_key, notif_type, title, message, pool_id,
               status, attempts, created_at, updated_at
        FROM user_notification_outbox
        WHERE user_address = $1
        ORDER BY id ASC
        "#,
    )
    .bind(address)
    .fetch_all(&mut *tx)
    .await?;

    let interests: Vec<String> = sqlx::query_as(
        "SELECT interest FROM user_interests WHERE user_address = $1 ORDER BY interest",
    )
    .bind(address)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(interest,)| interest)
    .collect();

    let push_rows = sqlx::query(
        "SELECT id, encrypted_payload, expires_at FROM browser_push_subscriptions WHERE user_address = $1 ORDER BY id",
    )
    .bind(address)
    .fetch_all(&mut *tx)
    .await?;
    let mut push_subscriptions = Vec::with_capacity(push_rows.len());
    for row in push_rows {
        let payload = decrypt_push_payload(
            secret,
            &row.try_get::<Vec<u8>, _>("encrypted_payload")
                .map_err(PushSubscriptionError::Database)?,
        )?;
        let payload: PushSubscriptionPayload =
            serde_json::from_slice(&payload).map_err(|_| PushSubscriptionError::Encryption)?;
        push_subscriptions.push(ExportedPushSubscription {
            id: row.try_get("id").map_err(PushSubscriptionError::Database)?,
            endpoint: payload.endpoint,
            p256dh: payload.p256dh,
            auth: payload.auth,
            expires_at: row
                .try_get("expires_at")
                .map_err(PushSubscriptionError::Database)?,
        });
    }

    tx.commit().await?;

    Ok(NotificationDataExport {
        user_address: address.to_string(),
        exported_at: Utc::now(),
        preferences,
        notifications,
        outbox,
        interests,
        push_subscriptions,
        on_chain_data_note: ON_CHAIN_DATA_NOTE.to_string(),
    })
}

/// Counts of private records removed by [`delete_notification_data`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct NotificationDeletionReport {
    pub preferences_deleted: u64,
    pub notifications_deleted: u64,
    pub outbox_deleted: u64,
    pub interests_deleted: u64,
    pub push_subscriptions_deleted: u64,
}

impl NotificationDeletionReport {
    /// Total number of private rows removed.
    pub fn total(&self) -> u64 {
        self.preferences_deleted
            + self.notifications_deleted
            + self.outbox_deleted
            + self.interests_deleted
            + self.push_subscriptions_deleted
    }
}

/// Delete every off-chain notification record linked to `address`.
///
/// Deletion is idempotent: running it again removes nothing and returns a
/// zeroed report. Chain-derived tables are never touched. The whole operation
/// runs in one transaction so a partial failure leaves no half-deleted state.
pub async fn delete_notification_data(
    pool: &PgPool,
    address: &str,
) -> Result<NotificationDeletionReport, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let preferences_deleted =
        sqlx::query("DELETE FROM notification_preferences WHERE user_address = $1")
            .bind(address)
            .execute(&mut *tx)
            .await?
            .rows_affected();

    let notifications_deleted = sqlx::query("DELETE FROM notifications WHERE user_address = $1")
        .bind(address)
        .execute(&mut *tx)
        .await?
        .rows_affected();

    let outbox_deleted =
        sqlx::query("DELETE FROM user_notification_outbox WHERE user_address = $1")
            .bind(address)
            .execute(&mut *tx)
            .await?
            .rows_affected();

    let interests_deleted = sqlx::query("DELETE FROM user_interests WHERE user_address = $1")
        .bind(address)
        .execute(&mut *tx)
        .await?
        .rows_affected();

    let push_subscriptions_deleted =
        sqlx::query("DELETE FROM browser_push_subscriptions WHERE user_address = $1")
            .bind(address)
            .execute(&mut *tx)
            .await?
            .rows_affected();

    tx.commit().await?;

    let report = NotificationDeletionReport {
        preferences_deleted,
        notifications_deleted,
        outbox_deleted,
        interests_deleted,
        push_subscriptions_deleted,
    };
    info!(?report, "notification data deleted for account");
    Ok(report)
}

// ── Weekly market digest (#289) ──────────────────────────────────────────────

/// How long an unsubscribe token remains redeemable after issuance.
const DIGEST_UNSUBSCRIBE_TOKEN_TTL_HOURS: i64 = 24 * 30;

/// A single public market status change for a followed market.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestEvent {
    pub market_id: i64,
    pub market_name: String,
    /// Public status, e.g. `active`, `settled`.
    pub status: String,
    /// Monotonic version of the status so repeated scans dedupe correctly.
    pub status_version: i64,
}

impl DigestEvent {
    /// Stable identity used for deduplication across digest runs.
    fn event_key(&self) -> String {
        format!("market:{}:{}", self.market_id, self.status_version)
    }
}

/// A rendered digest ready to email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedDigest {
    pub subject: String,
    pub body: String,
    pub events: Vec<DigestEvent>,
}

/// Compute the next digest send instant for `prefs` strictly after `now`.
///
/// The computation is DST-safe: it walks forward day by day in the user's
/// timezone and only accepts a candidate whose local wall-clock time matches
/// the configured send time. On a spring-forward day the configured time may
/// not exist; on a fall-back day it may occur twice. In both cases the first
/// instant whose local time is at or after the configured time on the target
/// weekday is chosen, so a digest is never skipped or duplicated.
pub fn next_digest_send(
    prefs: &DigestPreferences,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    use chrono::{Datelike, Duration, NaiveTime, TimeZone, Timelike, Weekday};

    let tz: chrono_tz::Tz = prefs.timezone.parse().ok()?;
    let (hour, minute) = parse_local_time(&prefs.send_time_local)?;
    let target_weekday = match prefs.day_of_week {
        0 => Weekday::Mon,
        1 => Weekday::Tue,
        2 => Weekday::Wed,
        3 => Weekday::Thu,
        4 => Weekday::Fri,
        5 => Weekday::Sat,
        _ => Weekday::Sun,
    };
    let target_time = NaiveTime::from_hms_opt(hour, minute, 0)?;

    let local_now = now.with_timezone(&tz);
    // Walk up to 8 days forward to cover the current week plus one full cycle.
    for offset in 0..=8i64 {
        let day = local_now.date_naive() + Duration::days(offset);
        if day.weekday() != target_weekday {
            continue;
        }
        // Resolve the local wall-clock time to an instant. `from_local_datetime`
        // returns `None` for a nonexistent (spring-forward) time and `Ambiguous`
        // for a repeated (fall-back) time.
        match tz.from_local_datetime(&day.and_time(target_time)) {
            chrono::LocalResult::Single(dt) => {
                let dt = dt.with_timezone(&Utc);
                if dt > now {
                    return Some(dt);
                }
            }
            chrono::LocalResult::Ambiguous(earliest, _latest) => {
                // Fall-back: the wall-clock time occurs twice. Pick the earliest
                // occurrence that is still in the future.
                let earliest = earliest.with_timezone(&Utc);
                if earliest > now {
                    return Some(earliest);
                }
            }
            chrono::LocalResult::None => {
                // Spring-forward: the configured time does not exist. Send at
                // the first valid instant after the gap on that day.
                let after_gap = tz
                    .from_local_datetime(&day.and_time(target_time + Duration::hours(1)))
                    .earliest()?
                    .with_timezone(&Utc);
                if after_gap > now {
                    return Some(after_gap);
                }
            }
        }
    }
    None
}

/// Deduplicate `events` by their stable event key, preserving first-seen order.
pub fn dedupe_digest_events(events: Vec<DigestEvent>) -> Vec<DigestEvent> {
    let mut seen = std::collections::BTreeSet::new();
    events
        .into_iter()
        .filter(|event| seen.insert(event.event_key()))
        .collect()
}

/// Render a digest email body. Returns `None` for an empty digest so callers
/// can skip sending entirely.
pub fn render_digest(events: &[DigestEvent], unsubscribe_url: &str) -> Option<RenderedDigest> {
    let events = dedupe_digest_events(events.to_vec());
    if events.is_empty() {
        return None;
    }
    let mut body = String::from("Your weekly market digest\n\n");
    for event in &events {
        body.push_str(&format!(
            "- {} is now {}.\n",
            event.market_name, event.status
        ));
    }
    body.push_str(&format!("\nUnsubscribe: {unsubscribe_url}\n"));
    Some(RenderedDigest {
        subject: "Your weekly market digest".to_string(),
        body,
        events,
    })
}

/// Hash an unsubscribe token so only the digest is stored server-side.
fn hash_unsubscribe_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    const HEX: &[u8; 16] = b"0123456789abcdef";
    digest
        .iter()
        .flat_map(|byte| {
            [
                HEX[(byte >> 4) as usize] as char,
                HEX[(byte & 0x0f) as usize] as char,
            ]
        })
        .collect()
}

/// Issue a signed, expiring unsubscribe token for `address`.
///
/// The raw token is returned to the caller (embedded in the email link); only
/// its hash is persisted, so a database leak cannot be used to unsubscribe
/// other users.
pub async fn issue_unsubscribe_token(
    pool: &PgPool,
    address: &str,
    now: DateTime<Utc>,
) -> Result<String, sqlx::Error> {
    let token = uuid::Uuid::new_v4().to_string();
    let token_hash = hash_unsubscribe_token(&token);
    let expires_at = now + chrono::Duration::hours(DIGEST_UNSUBSCRIBE_TOKEN_TTL_HOURS);
    sqlx::query(
        r#"
        INSERT INTO digest_unsubscribe_tokens (token_hash, user_address, expires_at)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(token_hash)
    .bind(address)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(token)
}

/// Redeem an unsubscribe token. Returns the address whose digest was disabled,
/// or `None` if the token is unknown, already used, or expired.
pub async fn redeem_unsubscribe_token(
    pool: &PgPool,
    token: &str,
    now: DateTime<Utc>,
) -> Result<Option<String>, sqlx::Error> {
    let token_hash = hash_unsubscribe_token(token);
    let mut tx = pool.begin().await?;
    let row: Option<(String,)> = sqlx::query_as(
        r#"
        UPDATE digest_unsubscribe_tokens
        SET used_at = $2
        WHERE token_hash = $1 AND used_at IS NULL AND expires_at > $2
        RETURNING user_address
        "#,
    )
    .bind(token_hash)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((address,)) = row else {
        tx.commit().await?;
        return Ok(None);
    };
    sqlx::query(
        r#"
        UPDATE notification_preferences
        SET preferences = jsonb_set(preferences, '{digest,enabled}', 'false'::jsonb),
            updated_at = NOW()
        WHERE user_address = $1
        "#,
    )
    .bind(&address)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(address))
}

/// Record that `event` was included in a digest for `address`. Returns `true`
/// when the row was newly inserted (i.e. the event had not been delivered).
async fn record_digest_delivery(
    tx: &mut Transaction<'_, Postgres>,
    address: &str,
    event: &DigestEvent,
) -> Result<bool, sqlx::Error> {
    let inserted = sqlx::query(
        r#"
        INSERT INTO digest_deliveries (user_address, market_id, status_version)
        VALUES ($1, $2, $3)
        ON CONFLICT (user_address, market_id, status_version) DO NOTHING
        "#,
    )
    .bind(address)
    .bind(event.market_id)
    .bind(event.status_version)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(inserted == 1)
}

/// Build the digest for `address` from followed-market public status changes,
/// excluding events already delivered. Returns `None` when nothing is new.
pub async fn build_digest(
    pool: &PgPool,
    address: &str,
    unsubscribe_url: &str,
) -> Result<Option<RenderedDigest>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT m.market_id, m.name, m.status, m.status_version
        FROM followed_markets f
        JOIN markets m ON m.market_id = f.market_id
        WHERE f.user_address = $1
          AND m.is_public = TRUE
          AND NOT EXISTS (
              SELECT 1 FROM digest_deliveries d
              WHERE d.user_address = $1
                AND d.market_id = m.market_id
                AND d.status_version = m.status_version
          )
        ORDER BY m.market_id
        "#,
    )
    .bind(address)
    .fetch_all(pool)
    .await?;

    let events: Vec<DigestEvent> = rows
        .into_iter()
        .map(|row| {
            Ok(DigestEvent {
                market_id: row.try_get("market_id")?,
                market_name: row.try_get("name")?,
                status: row.try_get("status")?,
                status_version: row.try_get("status_version")?,
            })
        })
        .collect::<Result<_, sqlx::Error>>()?;

    Ok(render_digest(&events, unsubscribe_url))
}

/// Send the weekly digest for `address` if one is due and non-empty.
///
/// Returns the number of events delivered (0 when the digest is disabled,
/// not due, or empty). Each delivered event is recorded so a later run never
/// repeats it.
pub async fn send_digest_if_due(
    pool: &PgPool,
    address: &str,
    now: DateTime<Utc>,
    unsubscribe_url: &str,
) -> Result<usize, sqlx::Error> {
    let preferences = get_notification_preferences(pool, address).await?;
    if !preferences.digest.enabled {
        return Ok(0);
    }
    let Some(next) = next_digest_send(&preferences.digest, now) else {
        return Ok(0);
    };
    // Only send within the hour window leading up to the scheduled instant.
    if next - now > chrono::Duration::hours(1) {
        return Ok(0);
    }
    let Some(rendered) = build_digest(pool, address, unsubscribe_url).await? else {
        return Ok(0);
    };

    let mut tx = pool.begin().await?;
    let mut delivered = 0usize;
    for event in &rendered.events {
        if record_digest_delivery(&mut tx, address, event).await? {
            delivered += 1;
        }
    }
    tx.commit().await?;
    Ok(delivered)
}
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
        if let Err(error) = prune_expired_push_subscriptions(&pool).await {
            warn!(error = %error, "expired push subscription cleanup failed");
        }
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

    #[test]
    fn preference_defaults_disable_every_known_channel_and_event() {
        let preferences = NotificationPreferences::default();
        assert_eq!(preferences.schema_version, NOTIFICATION_PREFERENCES_SCHEMA_VERSION);
        assert!(preferences.channels.values().all(|enabled| !enabled));
        assert!(preferences.events.values().all(|enabled| !enabled));
    }

    #[test]
    fn preference_migration_preserves_known_flags_and_disables_new_types() {
        let migrated = migrate_preferences(
            0,
            serde_json::json!({
                "pool_resolved": true,
                "future_event": true,
                "channels": { "push": true, "future_channel": true }
            }),
        )
        .expect("legacy preferences should migrate");
        assert!(migrated.events["pool_resolved"]);
        assert!(!migrated.events["pool_ending_soon"]);
        assert!(migrated.channels["push"]);
        assert!(!migrated.events.contains_key("future_event"));
        assert!(!migrated.channels.contains_key("future_channel"));
    }

    #[test]
    fn future_preference_schema_is_rejected() {
        assert!(migrate_preferences(3, serde_json::json!({})).is_err());
    }

    #[test]
    fn v1_preferences_migrate_to_v2_with_digest_disabled() {
        let migrated = migrate_preferences(
            1,
            serde_json::json!({
                "schema_version": 1,
                "channels": { "in_app": true },
                "events": { "pool_resolved": true }
            }),
        )
        .expect("v1 preferences should migrate");
        assert_eq!(migrated.schema_version, NOTIFICATION_PREFERENCES_SCHEMA_VERSION);
        assert!(migrated.channels["in_app"]);
        assert!(migrated.events["pool_resolved"]);
        assert!(!migrated.digest.enabled);
        assert_eq!(migrated.digest.timezone, "UTC");
    }

    #[test]
    fn notification_cursor_round_trips_without_exposing_raw_values() {
        let row = NotificationRow {
            id: 42,
            user_address: "GABC".to_string(),
            notif_type: "pool_resolved".to_string(),
            title: "resolved".to_string(),
            message: "done".to_string(),
            pool_id: Some(7),
            read: false,
            created_at: Utc::now(),
        };
        let cursor = encode_notification_cursor(&row);
        assert_eq!(cursor.len(), 32);
        assert!(!cursor.contains(':'));
        let (created_at, id) = decode_notification_cursor(&cursor).expect("valid cursor");
        assert_eq!(created_at, row.created_at);
        assert_eq!(id, row.id);
        assert!(decode_notification_cursor("not-a-cursor").is_err());
    }

    #[test]
    fn push_subscription_payload_is_authenticated_encryption() {
        let payload = br#"{"endpoint":"https://push.example.test/sub","p256dh":"key","auth":"secret"}"#;
        let ciphertext = encrypt_push_payload("stable-app-secret", payload)
            .expect("push payload should encrypt");
        assert_ne!(&ciphertext[12..], payload);
        assert_eq!(
            decrypt_push_payload("stable-app-secret", &ciphertext).expect("decrypt"),
            payload.to_vec()
        );
        assert!(decrypt_push_payload("different-secret", &ciphertext).is_err());

        let mut tampered = ciphertext;
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(decrypt_push_payload("stable-app-secret", &tampered).is_err());
    }

    #[test]
    fn push_endpoints_must_use_https_without_embedded_credentials() {
        assert!(validate_push_endpoint("https://push.example.test/sub").is_ok());
        assert!(validate_push_endpoint("http://push.example.test/sub").is_err());
        assert!(validate_push_endpoint("https://user:pass@push.example.test/sub").is_err());
    }

    #[test]
    fn deletion_report_total_sums_all_private_records() {
        let report = NotificationDeletionReport {
            preferences_deleted: 1,
            notifications_deleted: 2,
            outbox_deleted: 3,
            interests_deleted: 4,
            push_subscriptions_deleted: 5,
        };
        assert_eq!(report.total(), 15);
        assert_eq!(NotificationDeletionReport::default().total(), 0);
    }

    #[test]
    fn export_note_explains_immutable_on_chain_data() {
        assert!(ON_CHAIN_DATA_NOTE.contains("immutable"));
        assert!(ON_CHAIN_DATA_NOTE.contains("on-chain"));
    }

    // ── Weekly digest tests (#289) ──────────────────────────────────────────

    fn digest_prefs(day: u8, time: &str, tz: &str) -> DigestPreferences {
        DigestPreferences {
            enabled: true,
            day_of_week: day,
            send_time_local: time.to_string(),
            timezone: tz.to_string(),
        }
    }

    #[test]
    fn digest_next_send_respects_dst_spring_forward() {
        // 2024-03-31 is the EU spring-forward day; 02:30 does not exist in
        // Europe/London. The digest must still fire that day, after the gap.
        let prefs = digest_prefs(6, "02:30", "Europe/London"); // Sunday
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2024, 3, 30, 12, 0, 0)
            .unwrap();
        let next = next_digest_send(&prefs, now).expect("digest should schedule");
        let local = next.with_timezone(&chrono_tz::Europe::London);
        assert_eq!(local.date_naive(), chrono::NaiveDate::from_ymd_opt(2024, 3, 31).unwrap());
        // 02:30 local does not exist; the send lands at/after 03:00 local.
        assert!(local.time() >= chrono::NaiveTime::from_hms_opt(3, 0, 0).unwrap());
    }

    #[test]
    fn digest_next_send_respects_dst_fall_back() {
        // 2024-10-27 is the EU fall-back day; 01:30 occurs twice. The digest
        // must fire exactly once, at the earliest future occurrence.
        let prefs = digest_prefs(6, "01:30", "Europe/London"); // Sunday
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2024, 10, 26, 12, 0, 0)
            .unwrap();
        let next = next_digest_send(&prefs, now).expect("digest should schedule");
        let local = next.with_timezone(&chrono_tz::Europe::London);
        assert_eq!(local.date_naive(), chrono::NaiveDate::from_ymd_opt(2024, 10, 27).unwrap());
        assert_eq!(local.time(), chrono::NaiveTime::from_hms_opt(1, 30, 0).unwrap());
    }

    #[test]
    fn digest_next_send_is_strictly_in_the_future() {
        let prefs = digest_prefs(0, "09:00", "UTC"); // Monday
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2024, 4, 1, 9, 0, 0).unwrap();
        let next = next_digest_send(&prefs, now).expect("digest should schedule");
        assert!(next > now);
        assert_eq!(next, chrono::TimeZone::with_ymd_and_hms(&Utc, 2024, 4, 8, 9, 0, 0).unwrap());
    }

    #[test]
    fn empty_digest_renders_nothing() {
        assert!(render_digest(&[], "https://example.test/unsub").is_none());
    }

    #[test]
    fn digest_deduplicates_events_by_market_and_version() {
        let events = vec![
            DigestEvent {
                market_id: 1,
                market_name: "A".to_string(),
                status: "active".to_string(),
                status_version: 1,
            },
            DigestEvent {
                market_id: 1,
                market_name: "A".to_string(),
                status: "active".to_string(),
                status_version: 1,
            },
            DigestEvent {
                market_id: 1,
                market_name: "A".to_string(),
                status: "settled".to_string(),
                status_version: 2,
            },
        ];
        let deduped = dedupe_digest_events(events);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].status_version, 1);
        assert_eq!(deduped[1].status_version, 2);

        let rendered = render_digest(&deduped, "https://example.test/unsub").unwrap();
        assert_eq!(rendered.events.len(), 2);
        assert!(rendered.body.contains("Unsubscribe: https://example.test/unsub"));
    }

    #[test]
    fn unsubscribe_token_hash_is_stable_and_opaque() {
        let hash = hash_unsubscribe_token("token-abc");
        assert_eq!(hash, hash_unsubscribe_token("token-abc"));
        assert_ne!(hash, "token-abc");
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn digest_preferences_normalize_invalid_schedule() {
        let normalized = normalize_digest(DigestPreferences {
            enabled: true,
            day_of_week: 9,
            send_time_local: "25:99".to_string(),
            timezone: "Not/AZone".to_string(),
        });
        assert_eq!(normalized.day_of_week, 0);
        assert_eq!(normalized.send_time_local, "09:00");
        assert_eq!(normalized.timezone, "UTC");
    }
}
