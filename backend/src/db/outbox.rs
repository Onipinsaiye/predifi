//! Notification outbox domain types.
//!
//! A [`NotificationIntent`] is a pending user notification produced by a
//! domain event (pool resolved, prediction settled, referral paid…). It is
//! serialised to a JSON payload for delivery through the transactional
//! outbox implemented in [`crate::notifications`], which owns the
//! `notification_outbox` table, claiming, retries, and terminal failure
//! handling.
//!
//! # Retention and deletion policy
//!
//! Every user-linked field carried by this module is classified below so
//! that scheduled cleanup can be applied deterministically.
//!
//! | Field / record                         | Class            | Retention                         | Deletion behaviour                                  |
//! |----------------------------------------|------------------|-----------------------------------|-----------------------------------------------------|
//! | `NotificationIntent::user_id`          | Personal (PII)   | Until outbox row is delivered/failed | Row purged by [`RetentionPolicy::eligible_for_purge`] after `outbox_ttl` |
//! | `NotificationIntent::payload`          | Personal (PII)   | Same as outbox row               | Purged with the row; never copied to audit records  |
//! | `NotificationIntent::kind`             | Non-personal     | Same as outbox row               | Purged with the row                                 |
//! | `ClaimedNotification::*` (transient)   | Derived          | In-memory only                   | Not persisted; nothing to delete                    |
//! | `CredentialRotationAudit::provider`    | Operational      | Audit retention (`audit_ttl`)    | Retained for audit; contains no user data           |
//! | `CredentialRotationAudit::old/new_key_id` | Operational   | Audit retention                  | Retained; identifiers only, never secrets           |
//! | `CredentialRotationAudit::actor`       | Operator PII    | Audit retention                  | Retained as required audit record                   |
//! | `CredentialRotationAudit::reason/outcome/timestamp` | Operational | Audit retention            | Retained as required audit record                   |
//!
//! Public chain-derived history is not stored in this module and is therefore
//! out of scope for deletion; it remains reproducible from chain data.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Classification of a user-linked or audit field for retention purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldClass {
    /// Personal data about a user; subject to deletion once no longer needed.
    Personal,
    /// Operational data that is not personal but may reference operators.
    Operational,
    /// Derived or transient data that is never persisted.
    Transient,
    /// Data reproducible from public chain events; never deleted by this policy.
    ChainDerived,
}

/// Retention windows in seconds for the notification outbox and audit records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// How long a terminal (delivered or failed) outbox row is kept.
    pub outbox_ttl_secs: i64,
    /// How long credential rotation audit records are kept.
    pub audit_ttl_secs: i64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            // 30 days for personal notification data.
            outbox_ttl_secs: 30 * 24 * 60 * 60,
            // 7 years for security audit records.
            audit_ttl_secs: 7 * 365 * 24 * 60 * 60,
        }
    }
}

impl RetentionPolicy {
    /// Whether a terminal outbox row created at `created_at` is eligible for
    /// deletion at `now` (both Unix seconds).
    pub fn outbox_row_eligible_for_purge(&self, created_at: i64, now: i64) -> bool {
        now.saturating_sub(created_at) >= self.outbox_ttl_secs
    }

    /// Whether an audit record stamped `timestamp` is eligible for deletion at
    /// `now`. Audit records are only purged after the longer audit window.
    pub fn audit_record_eligible_for_purge(&self, timestamp: i64, now: i64) -> bool {
        now.saturating_sub(timestamp) >= self.audit_ttl_secs
    }

    /// Return a copy of `payload` with personal fields removed, suitable for
    /// retention in places where the full payload must not persist.
    pub fn redact_payload(payload: &Value) -> Value {
        match payload {
            Value::Object(map) => {
                let mut out = serde_json::Map::new();
                for (k, v) in map {
                    if is_personal_key(k) {
                        continue;
                    }
                    out.insert(k.clone(), v.clone());
                }
                Value::Object(out)
            }
            other => other.clone(),
        }
    }
}

/// Keys in a notification payload that carry personal data and must be
/// stripped before long-term retention.
fn is_personal_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "email" | "phone" | "address" | "name" | "ip" | "user_agent"
    )
}

/// Outcome of a notification provider credential rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationOutcome {
    /// Rotation started but has not yet been verified.
    Pending,
    /// New credential verified and old credential revoked.
    Completed,
    /// Rotation failed; the previous credential may still be active.
    Failed,
    /// Rotation was rolled back to the previous credential.
    RolledBack,
}

/// Audit record for a notification provider credential rotation.
///
/// Recorded whenever a provider credential is rotated (for example after a
/// suspected compromise) so that detection, revocation, replacement,
/// deployment, and verification steps are traceable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialRotationAudit {
    /// Notification provider whose credential was rotated.
    pub provider: String,
    /// Identifier of the credential being replaced (never the secret itself).
    pub old_key_id: Option<String>,
    /// Identifier of the replacement credential (never the secret itself).
    pub new_key_id: Option<String>,
    /// Actor who initiated the rotation.
    pub actor: String,
    /// Reason for the rotation (e.g. `compromise_suspected`).
    pub reason: String,
    /// Unix timestamp (seconds) when the rotation event was recorded.
    pub timestamp: i64,
    /// Outcome of the rotation at the time of recording.
    pub outcome: RotationOutcome,
}

impl CredentialRotationAudit {
    /// Create a new audit record with a `Pending` outcome.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: impl Into<String>,
        old_key_id: Option<String>,
        new_key_id: Option<String>,
        actor: impl Into<String>,
        reason: impl Into<String>,
        timestamp: i64,
    ) -> Self {
        Self {
            provider: provider.into(),
            old_key_id,
            new_key_id,
            actor: actor.into(),
            reason: reason.into(),
            timestamp,
            outcome: RotationOutcome::Pending,
        }
    }

    /// Serialize the audit record to a JSON string for storage.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("credential rotation audit is serializable")
    }

    /// Deserialize an audit record from its stored JSON representation.
    pub fn from_json(raw: &str) -> serde_json::Result<Self> {
        serde_json::from_str(raw)
    }
}

/// A pending notification intent awaiting delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationIntent {
    /// Identifier of the user the notification is addressed to.
    pub user_id: String,
    /// Logical notification kind (e.g. `pool_resolved`).
    pub kind: String,
    /// Arbitrary JSON payload delivered alongside the notification.
    pub payload: Value,
    /// Delivery attempts so far.
    pub attempts: u32,
}

impl NotificationIntent {
    /// Create a new intent with zero delivery attempts.
    pub fn new(user_id: impl Into<String>, kind: impl Into<String>, payload: Value) -> Self {
        Self {
            user_id: user_id.into(),
            kind: kind.into(),
            payload,
            attempts: 0,
        }
    }

    /// Serialize the payload to a JSON string for storage in the outbox.
    pub fn payload_json(&self) -> String {
        self.payload.to_string()
    }
}

/// A notification intent claimed by a delivery worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimedNotification {
    /// Outbox row identifier.
    pub id: i64,
    /// Identifier of the user the notification is addressed to.
    pub user_id: String,
    /// Logical notification kind.
    pub kind: String,
    /// Delivery payload.
    pub payload: Value,
    /// Delivery attempts made so far.
    pub attempts: u32,
    /// Maximum delivery attempts before terminal failure.
    pub max_attempts: u32,
}

impl ClaimedNotification {
    /// Whether another delivery attempt is still allowed.
    pub fn can_retry(&self) -> bool {
        self.attempts < self.max_attempts
    }

    /// Delivery attempts remaining before terminal failure.
    pub fn remaining_attempts(&self) -> u32 {
        self.max_attempts.saturating_sub(self.attempts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_defaults_to_zero_attempts() {
        let intent = NotificationIntent::new("GABC", "pool_resolved", serde_json::json!({}));
        assert_eq!(intent.attempts, 0);
        assert_eq!(intent.user_id, "GABC");
    }

    #[test]
    fn claimed_retry_budget_is_saturating() {
        let claimed = ClaimedNotification {
            id: 1,
            user_id: "GABC".into(),
            kind: "referral_paid".into(),
            payload: serde_json::json!({}),
            attempts: 5,
            max_attempts: 5,
        };
        assert!(!claimed.can_retry());
        assert_eq!(claimed.remaining_attempts(), 0);
    }

    #[test]
    fn claimed_retry_allowed_below_max() {
        let claimed = ClaimedNotification {
            id: 2,
            user_id: "GABC".into(),
            kind: "pool_resolved".into(),
            payload: serde_json::json!({"pool": 7}),
            attempts: 1,
            max_attempts: 3,
        };
        assert!(claimed.can_retry());
        assert_eq!(claimed.remaining_attempts(), 2);
    }

    #[test]
    fn rotation_audit_round_trips_through_json() {
        let audit = CredentialRotationAudit::new(
            "push",
            Some("key-old".into()),
            Some("key-new".into()),
            "ops@example.com",
            "compromise_suspected",
            1_700_000_000,
        );
        assert_eq!(audit.outcome, RotationOutcome::Pending);

        let raw = audit.to_json();
        let decoded = CredentialRotationAudit::from_json(&raw).expect("valid audit json");
        assert_eq!(decoded.provider, "push");
        assert_eq!(decoded.old_key_id.as_deref(), Some("key-old"));
        assert_eq!(decoded.new_key_id.as_deref(), Some("key-new"));
        assert_eq!(decoded.actor, "ops@example.com");
        assert_eq!(decoded.reason, "compromise_suspected");
        assert_eq!(decoded.timestamp, 1_700_000_000);
        assert_eq!(decoded.outcome, RotationOutcome::Pending);
    }

    #[test]
    fn rotation_audit_serializes_outcome_as_snake_case() {
        let mut audit = CredentialRotationAudit::new(
            "email",
            None,
            Some("key-new".into()),
            "ops@example.com",
            "scheduled_rotation",
            1_700_000_100,
        );
        audit.outcome = RotationOutcome::Completed;

        let raw = audit.to_json();
        assert!(raw.contains("\"outcome\":\"completed\""));
        assert!(raw.contains("\"old_key_id\":null"));
    }

    #[test]
    fn outbox_purge_respects_ttl() {
        let policy = RetentionPolicy::default();
        let created = 1_700_000_000;
        assert!(!policy.outbox_row_eligible_for_purge(created, created + 1));
        assert!(policy.outbox_row_eligible_for_purge(
            created,
            created + policy.outbox_ttl_secs
        ));
    }

    #[test]
    fn audit_purge_uses_longer_window() {
        let policy = RetentionPolicy::default();
        let ts = 1_700_000_000;
        // Past the outbox window but well within the audit window.
        let now = ts + policy.outbox_ttl_secs + 1;
        assert!(!policy.audit_record_eligible_for_purge(ts, now));
        assert!(policy.audit_record_eligible_for_purge(ts, ts + policy.audit_ttl_secs));
    }

    #[test]
    fn redact_payload_strips_personal_keys() {
        let payload = serde_json::json!({
            "pool": 7,
            "email": "user@test.invalid",
            "ip": "203.0.113.1",
        });
        let redacted = RetentionPolicy::redact_payload(&payload);
        assert_eq!(redacted, serde_json::json!({"pool": 7}));
    }
}
