//! Checkpoint persistence for the Stellar event listener.
//!
//! Stores the listener's cursor (the next ledger sequence number to index)
//! atomically alongside indexed event rows within the same database transaction.
//!
//! Startup behavior handles missing or malformed checkpoints gracefully, ensuring
//! the service starts reliably without losing events or crashing.

use sqlx::{PgPool, Postgres, Transaction};
use tracing::{error, info, instrument, warn};

/// Default listener key used in `listener_checkpoints` and `app_state`.
pub const DEFAULT_LISTENER_ID: &str = "stellar_listener";

/// Status of a loaded checkpoint from persistent storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointStatus {
    /// A valid ledger checkpoint was found.
    Valid(u64),
    /// No checkpoint exists in the database.
    Missing,
    /// A checkpoint entry was found, but the value is corrupt, unparseable, or negative.
    Malformed(String),
}

/// Load the last processed ledger checkpoint for `listener_id`.
///
/// First queries `listener_checkpoints`. If not found, falls back to `app_state`
/// for backward compatibility. Validates that the recorded ledger is a valid
/// non-negative integer.
#[instrument(skip(pool), name = "db.load_checkpoint", fields(listener_id = %listener_id))]
pub async fn load_checkpoint(pool: &PgPool, listener_id: &str) -> CheckpointStatus {
    // 1. Try dedicated `listener_checkpoints` table first
    let dedicated_result = sqlx::query_scalar::<_, i64>(
        "SELECT last_ledger FROM listener_checkpoints WHERE id = $1",
    )
    .bind(listener_id)
    .fetch_optional(pool)
    .await;

    match dedicated_result {
        Ok(Some(ledger_i64)) => {
            if ledger_i64 < 0 {
                return CheckpointStatus::Malformed(format!(
                    "negative ledger sequence in listener_checkpoints: {}",
                    ledger_i64
                ));
            }
            return CheckpointStatus::Valid(ledger_i64 as u64);
        }
        Ok(None) => {}
        Err(e) => {
            warn!(
                listener_id,
                error = %e,
                "failed to query listener_checkpoints table; attempting app_state fallback"
            );
        }
    }

    // 2. Fallback to `app_state` table for backward compatibility
    let app_state_result = sqlx::query_scalar::<_, String>(
        "SELECT value FROM app_state WHERE key = $1",
    )
    .bind(listener_id)
    .fetch_optional(pool)
    .await;

    match app_state_result {
        Ok(Some(val_str)) => match val_str.trim().parse::<u64>() {
            Ok(ledger) => CheckpointStatus::Valid(ledger),
            Err(parse_err) => CheckpointStatus::Malformed(format!(
                "invalid integer in app_state for key '{}': '{}' ({})",
                listener_id, val_str, parse_err
            )),
        },
        Ok(None) => CheckpointStatus::Missing,
        Err(e) => {
            error!(
                listener_id,
                error = %e,
                "failed to query app_state table for checkpoint"
            );
            CheckpointStatus::Malformed(format!("database error reading app_state: {}", e))
        }
    }
}

/// Define deterministic startup behavior based on checkpoint status.
///
/// Acceptance criteria:
/// - `Valid(ledger)`: Resumes from the persisted ledger.
/// - `Missing`: Starts from `default_ledger` (typically 1) and logs an informational notice.
/// - `Malformed(reason)`: Logs an error describing the corrupt data, alerts operators,
///   and safely recovers by starting from `default_ledger` without crashing the process.
pub fn resolve_startup_checkpoint(
    status: &CheckpointStatus,
    listener_id: &str,
    default_ledger: u64,
) -> u64 {
    match status {
        CheckpointStatus::Valid(ledger) => {
            info!(
                listener_id,
                cursor = ledger,
                "resuming stellar listener from persisted checkpoint"
            );
            *ledger
        }
        CheckpointStatus::Missing => {
            info!(
                listener_id,
                default_ledger,
                "no existing checkpoint found; starting listener from default ledger"
            );
            default_ledger
        }
        CheckpointStatus::Malformed(reason) => {
            warn!(
                listener_id,
                corruption_reason = %reason,
                fallback_ledger = default_ledger,
                "checkpoint found but malformed or corrupted; safely recovering using default ledger"
            );
            default_ledger
        }
    }
}

/// Persist the latest checkpoint atomically within an existing SQL transaction.
///
/// Upserts both `listener_checkpoints` and `app_state` (for backward compatibility)
/// as part of the caller's transaction.
#[instrument(skip(tx), name = "db.save_checkpoint_tx", fields(listener_id = %listener_id, ledger = ledger))]
pub async fn save_checkpoint_tx(
    tx: &mut Transaction<'_, Postgres>,
    listener_id: &str,
    ledger: u64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO listener_checkpoints (id, last_ledger, updated_at)
        VALUES ($1, $2, NOW())
        ON CONFLICT (id) DO UPDATE
            SET last_ledger = EXCLUDED.last_ledger,
                updated_at  = EXCLUDED.updated_at
        "#,
    )
    .bind(listener_id)
    .bind(ledger as i64)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO app_state (key, value, updated_at)
        VALUES ($1, $2, NOW())
        ON CONFLICT (key) DO UPDATE
            SET value      = EXCLUDED.value,
                updated_at = EXCLUDED.updated_at
        "#,
    )
    .bind(listener_id)
    .bind(ledger.to_string())
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Convenience standalone wrapper to persist a checkpoint in its own transaction.
#[instrument(skip(pool), name = "db.save_checkpoint", fields(listener_id = %listener_id, ledger = ledger))]
pub async fn save_checkpoint(
    pool: &PgPool,
    listener_id: &str,
    ledger: u64,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    save_checkpoint_tx(&mut tx, listener_id, ledger).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_startup_checkpoint_valid() {
        let status = CheckpointStatus::Valid(12345);
        let resolved = resolve_startup_checkpoint(&status, DEFAULT_LISTENER_ID, 1);
        assert_eq!(resolved, 12345);
    }

    #[test]
    fn test_resolve_startup_checkpoint_missing() {
        let status = CheckpointStatus::Missing;
        let resolved = resolve_startup_checkpoint(&status, DEFAULT_LISTENER_ID, 1);
        assert_eq!(resolved, 1);
    }

    #[test]
    fn test_resolve_startup_checkpoint_malformed() {
        let status = CheckpointStatus::Malformed("not a valid integer".to_string());
        let resolved = resolve_startup_checkpoint(&status, DEFAULT_LISTENER_ID, 100);
        assert_eq!(resolved, 100);
    }
}
