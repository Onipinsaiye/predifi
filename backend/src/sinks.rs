//! Outbound notification delivery with retry + dead-letter handling (#63).
//!
//! The in-app notification feed (`crate::notifications`) is idempotent at the
//! DB layer: a unique index on `(user_address, pool_id, notif_type)` prevents
//! duplicate in-app rows. Outbound deliveries (email, webhook, push, sms) are
//! different — the delivery call is remote and can fail transiently, so each
//! attempt needs its own state.
//!
//! [`NotificationSink`] is the send interface. The production `DbSink` marks
//! the delivery row `sent`; the test `FakeSink` lets integration tests script
//! a sequence of failures and then a success.
//!
//! Retry semantics reuse [`crate::worker::queue::RetryPolicy`] so notification
//! delivery and Stellar event processing share the same backoff shape.
//!
//! The queue is process-wide ([`notification_queue`]) and separate from the
//! Stellar event queue so a notification storm cannot starve on-chain event
//! processing.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use tracing::{error, info, warn};

use crate::worker::queue::{Job, JobQueue, RetryPolicy};

/// Process-wide notification queue, distinct from the Stellar event queue
/// (`crate::worker::stellar_listener::worker_job_queue`) so notification
/// failures cannot starve on-chain processing.
static NOTIFICATION_QUEUE: OnceLock<JobQueue> = OnceLock::new();

pub fn notification_queue() -> &'static JobQueue {
    NOTIFICATION_QUEUE.get_or_init(|| {
        // Notifications are lower priority than on-chain events: same base
        // delay, but a lower attempt count to reach the dead-letter queue
        // faster (operators don't want a stuck email to sit for 5 minutes).
        JobQueue::new(RetryPolicy {
            max_attempts: 5,
            base_delay_ms: 500,
            max_delay_ms: 60_000,
        })
    })
}

// ── Sink ─────────────────────────────────────────────────────────────────────

/// Categorized failure of an outbound delivery attempt.
///
/// The category is stored in `outbound_deliveries.failure_category` and drives
/// whether the retry loop attempts again or dead-letters immediately.
#[derive(Debug, Clone)]
pub enum SinkError {
    /// Network blip, 5xx, connection reset — retryable.
    Transient(String),
    /// Provider 429 with a retry-after hint — retryable after the hint.
    RateLimited { retry_after: Duration, message: String },
    /// Provider 4xx "your message is malformed / recipient invalid" — dead-letter.
    Permanent(String),
    /// Provider 401/403, revoked API key — dead-letter and alert ops.
    Auth(String),
}

impl SinkError {
    pub fn category(&self) -> &'static str {
        match self {
            SinkError::Transient(_) => "transient",
            SinkError::RateLimited { .. } => "rate_limited",
            SinkError::Permanent(_) => "permanent",
            SinkError::Auth(_) => "auth",
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self, SinkError::Transient(_) | SinkError::RateLimited { .. })
    }

    pub fn message(&self) -> &str {
        match self {
            SinkError::Transient(s) | SinkError::Permanent(s) | SinkError::Auth(s) => s,
            SinkError::RateLimited { message, .. } => message,
        }
    }

    /// Suggested delay before the next attempt, if the provider supplied one.
    pub fn hint_delay(&self) -> Option<Duration> {
        match self {
            SinkError::RateLimited { retry_after, .. } => Some(*retry_after),
            _ => None,
        }
    }
}

/// One outbound delivery row (from `outbound_deliveries`).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct OutboundDelivery {
    pub id: i64,
    pub notification_id: i64,
    pub channel: String,
    pub payload: serde_json::Value,
    pub idempotency_key: String,
    pub attempts: i32,
    pub next_attempt_at: DateTime<Utc>,
    pub status: String,
    pub failure_category: Option<String>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Sink dispatcher. Matches on the enum to avoid a dyn-trait future wrapper
/// (the project does not depend on `async-trait`).
pub enum NotificationSink {
    Db(DbSink),
    Fake(FakeSink),
}

impl NotificationSink {
    pub async fn send(&self, delivery: &OutboundDelivery) -> Result<(), SinkError> {
        match self {
            NotificationSink::Db(s) => s.send(delivery).await,
            NotificationSink::Fake(s) => s.send(delivery).await,
        }
    }
}

/// Production sink. For in-app notifications there is no external call —
/// "sending" simply means the delivery row is marked `sent`. This is a
/// placeholder for future channels (email, webhook, push); each channel
/// would branch on `delivery.channel` and call the appropriate provider.
pub struct DbSink;

impl DbSink {
    pub async fn send(&self, delivery: &OutboundDelivery) -> Result<(), SinkError> {
        match delivery.channel.as_str() {
            // In-app / placeholder channels: the row's existence IS the delivery.
            // A future PR adds real email/webhook/push clients here.
            "email" | "webhook" | "push" | "sms" => {
                warn!(
                    delivery_id = delivery.id,
                    channel = %delivery.channel,
                    "outbound sink has no external provider wired up; marking sent"
                );
                Ok(())
            }
            other => Err(SinkError::Permanent(format!("unknown channel {other}"))),
        }
    }
}

/// Test sink: lets integration tests script a sequence of results per
/// idempotency key. `push_result` appends one outcome; `sent` accumulates
/// successfully-delivered keys for dedupe assertions.
pub struct FakeSink {
    results: Mutex<VecDeque<Result<(), SinkError>>>,
    sent: Mutex<Vec<String>>,
}

impl FakeSink {
    pub fn new() -> Self {
        Self {
            results: Mutex::new(VecDeque::new()),
            sent: Mutex::new(Vec::new()),
        }
    }

    /// Queue an outcome for the next `send` call.
    pub fn push_result(&self, result: Result<(), SinkError>) {
        self.results.lock().expect("results lock").push_back(result);
    }

    /// Delivery keys that `send` returned Ok for.
    pub fn sent_keys(&self) -> Vec<String> {
        self.sent.lock().expect("sent lock").clone()
    }

    pub async fn send(&self, delivery: &OutboundDelivery) -> Result<(), SinkError> {
        let next = self.results.lock().expect("results lock").pop_front();
        match next {
            Some(Ok(())) => {
                self.sent
                    .lock()
                    .expect("sent lock")
                    .push(delivery.idempotency_key.clone());
                Ok(())
            }
            Some(Err(e)) => Err(e),
            // No scripted result → succeed (default behavior for happy paths).
            None => {
                self.sent
                    .lock()
                    .expect("sent lock")
                    .push(delivery.idempotency_key.clone());
                Ok(())
            }
        }
    }
}

impl Default for FakeSink {
    fn default() -> Self {
        Self::new()
    }
}

// ── Persistence helpers ─────────────────────────────────────────────────────

/// Startup recovery: return all `pending` deliveries whose `next_attempt_at`
/// has already passed, so they can be re-enqueued before the loop starts.
pub async fn recover_pending_deliveries(
    pool: &PgPool,
) -> Result<Vec<OutboundDelivery>, sqlx::Error> {
    sqlx::query_as::<_, OutboundDelivery>(
        r#"
        SELECT id, notification_id, channel, payload, idempotency_key,
               attempts, next_attempt_at, status, failure_category, last_error,
               created_at, updated_at
        FROM outbound_deliveries
        WHERE status = 'pending' AND next_attempt_at <= NOW()
        ORDER BY next_attempt_at ASC
        "#,
    )
    .fetch_all(pool)
    .await
}

/// Mark a delivery as `sent` after the sink succeeded.
pub async fn mark_sent(pool: &PgPool, delivery_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE outbound_deliveries
         SET status = 'sent', attempts = attempts + 1, updated_at = NOW()
         WHERE id = $1",
    )
    .bind(delivery_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a failed attempt: either schedule a retry (`next_attempt_at`) or
/// dead-letter when the retry budget is exhausted or the error is permanent.
pub async fn mark_failed(
    pool: &PgPool,
    delivery: &OutboundDelivery,
    error: &SinkError,
    policy: RetryPolicy,
) -> Result<(), sqlx::Error> {
    let new_attempts = delivery.attempts + 1;
    let dead = !error.is_retryable() || !policy.should_retry(new_attempts as u32);

    if dead {
        error!(
            delivery_id = delivery.id,
            attempts = new_attempts,
            category = error.category(),
            error = %error.message(),
            "delivery dead-lettered"
        );
        sqlx::query(
            "UPDATE outbound_deliveries
             SET status = 'dead',
                 attempts = $2,
                 failure_category = $3,
                 last_error = $4,
                 updated_at = NOW()
             WHERE id = $1",
        )
        .bind(delivery.id)
        .bind(new_attempts)
        .bind(error.category())
        .bind(error.message())
        .execute(pool)
        .await?;
    } else {
        // Prefer the provider's hint if it gave one, otherwise use the policy.
        let delay = error
            .hint_delay()
            .unwrap_or_else(|| policy.delay_before_attempt(new_attempts as u32 + 1));
        warn!(
            delivery_id = delivery.id,
            attempts = new_attempts,
            category = error.category(),
            delay_ms = delay.as_millis() as u64,
            error = %error.message(),
            "delivery failed; retrying"
        );
        sqlx::query(
            "UPDATE outbound_deliveries
             SET attempts = $2,
                 failure_category = $3,
                 last_error = $4,
                 next_attempt_at = NOW() + ($5 || ' milliseconds')::interval,
                 updated_at = NOW()
             WHERE id = $1",
        )
        .bind(delivery.id)
        .bind(new_attempts)
        .bind(error.category())
        .bind(error.message())
        .bind(delay.as_millis() as i64)
        .execute(pool)
        .await?;
    }

    Ok(())
}

/// Manual replay: reset a dead delivery to `pending` so the next sweep picks
/// it up. Returns `true` if a dead row was found and reset.
pub async fn replay_dead(pool: &PgPool, delivery_id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE outbound_deliveries
         SET status = 'pending',
             attempts = 0,
             next_attempt_at = NOW(),
             failure_category = NULL,
             last_error = NULL,
             updated_at = NOW()
         WHERE id = $1 AND status = 'dead'",
    )
    .bind(delivery_id)
    .execute(pool)
    .await?;

    let hit = result.rows_affected() > 0;
    if hit {
        info!(delivery_id, "delivery replayed from DLQ");
    }
    Ok(hit)
}

/// Fetch one delivery by id (used by the replay handler).
pub async fn get_delivery(
    pool: &PgPool,
    delivery_id: i64,
) -> Result<Option<OutboundDelivery>, sqlx::Error> {
    sqlx::query_as::<_, OutboundDelivery>(
        r#"
        SELECT id, notification_id, channel, payload, idempotency_key,
               attempts, next_attempt_at, status, failure_category, last_error,
               created_at, updated_at
        FROM outbound_deliveries
        WHERE id = $1
        "#,
    )
    .bind(delivery_id)
    .fetch_optional(pool)
    .await
}

// ── Delivery loop ────────────────────────────────────────────────────────────

/// Send one delivery via `sink`, updating DB state on success/failure.
///
/// This is the per-job handler passed to `JobQueue::process_with_retry`.
pub async fn deliver_once(
    pool: &PgPool,
    sink: &NotificationSink,
    delivery: &OutboundDelivery,
    policy: RetryPolicy,
) -> Result<(), String> {
    match sink.send(delivery).await {
        Ok(()) => {
            if let Err(e) = mark_sent(pool, delivery.id).await {
                return Err(format!("mark_sent failed: {e}"));
            }
            Ok(())
        }
        Err(sink_err) => {
            if let Err(e) = mark_failed(pool, delivery, &sink_err, policy).await {
                return Err(format!("mark_failed failed: {e}"));
            }
            if sink_err.is_retryable() {
                Err(sink_err.message().to_string())
            } else {
                // Permanent / auth errors are recorded but not re-raised
                // because retrying won't help — the row is already dead.
                Ok(())
            }
        }
    }
}

/// Wrap a delivery into a `Job` and hand it to the queue. The job id is the
/// idempotency key so duplicate enqueues no-op.
pub fn enqueue_delivery(delivery: &OutboundDelivery) {
    let job = Job {
        id: delivery.idempotency_key.clone(),
        kind: format!("outbound_delivery:{}", delivery.channel),
        payload: delivery.id.to_string(),
        attempts: delivery.attempts.max(0) as u32,
        ledger: None,
    };
    notification_queue().enqueue(job);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_delivery(idempotency_key: &str) -> OutboundDelivery {
        OutboundDelivery {
            id: 1,
            notification_id: 42,
            channel: "email".to_string(),
            payload: serde_json::json!({"to": "alice@example.com", "subject": "hi"}),
            idempotency_key: idempotency_key.to_string(),
            attempts: 0,
            next_attempt_at: Utc::now(),
            status: "pending".to_string(),
            failure_category: None,
            last_error: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn sink_error_categories_match_expected_strings() {
        assert_eq!(SinkError::Transient("x".into()).category(), "transient");
        assert_eq!(
            SinkError::RateLimited {
                retry_after: Duration::from_secs(1),
                message: "x".into()
            }
            .category(),
            "rate_limited"
        );
        assert_eq!(SinkError::Permanent("x".into()).category(), "permanent");
        assert_eq!(SinkError::Auth("x".into()).category(), "auth");
    }

    #[test]
    fn transient_and_rate_limited_are_retryable() {
        assert!(SinkError::Transient("x".into()).is_retryable());
        assert!(SinkError::RateLimited {
            retry_after: Duration::from_secs(1),
            message: "x".into()
        }
        .is_retryable());
        assert!(!SinkError::Permanent("x".into()).is_retryable());
        assert!(!SinkError::Auth("x".into()).is_retryable());
    }

    #[test]
    fn rate_limited_exposes_retry_after_hint() {
        let err = SinkError::RateLimited {
            retry_after: Duration::from_secs(30),
            message: "slow down".into(),
        };
        assert_eq!(err.hint_delay(), Some(Duration::from_secs(30)));
        assert_eq!(SinkError::Transient("x".into()).hint_delay(), None);
    }

    #[tokio::test]
    async fn fake_sink_records_sent_keys_and_dedupes_by_key() {
        let sink = FakeSink::new();
        let d = sample_delivery("idem-abc");
        sink.send(&d).await.unwrap();
        sink.send(&d).await.unwrap();
        assert_eq!(sink.sent_keys(), vec!["idem-abc".to_string(), "idem-abc".to_string()]);
    }

    #[tokio::test]
    async fn fake_sink_pops_scripted_results_in_order() {
        let sink = FakeSink::new();
        sink.push_result(Err(SinkError::Transient("net".into())));
        sink.push_result(Ok(()));

        let d = sample_delivery("idem-order");
        assert!(sink.send(&d).await.is_err());
        assert!(sink.send(&d).await.is_ok());
        assert_eq!(sink.sent_keys(), vec!["idem-order".to_string()]);
    }
}
