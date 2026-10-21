//! Notification outbox domain types.
//!
//! A [`NotificationIntent`] is a pending user notification produced by a
//! domain event (pool resolved, prediction settled, referral paid…). It is
//! serialised to a JSON payload for delivery through the transactional
//! outbox implemented in [`crate::notifications`], which owns the
//! `notification_outbox` table, claiming, retries, and terminal failure
//! handling.

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}
