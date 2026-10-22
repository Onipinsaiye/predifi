//! Idempotent projection of on-chain contract events (#45).
//!
//! Every indexed event carries an [`EventKey`] — network, contract, ledger,
//! transaction hash, and the event's position within that transaction — which
//! uniquely identifies it on-chain. Projecting an event first claims that key
//! in `projected_contract_events` inside the same transaction as the domain
//! writes; a key that is already present means the event was projected before
//! and the whole projection is skipped. Retries, listener restarts, and
//! overlapping ledger windows therefore never create duplicate pools,
//! predictions, stake increments, or notifications.

use sqlx::{PgPool, Postgres, Transaction};
use tracing::instrument;

use crate::notifications::{enqueue_notification, NotifType};

use super::{
    insert_pool_from_event_tx, insert_prediction_from_event, resolve_pool_in_db, PoolCreatedEvent,
    PredictionPlacedEvent,
};

/// Canonical on-chain identity of a contract event.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventKey {
    /// Network passphrase or short network name (e.g. `testnet`).
    pub network: String,
    /// Emitting contract ID (`C...`).
    pub contract_id: String,
    pub ledger: u64,
    pub tx_hash: String,
    /// Position of the event within its transaction.
    pub event_index: u32,
}

impl EventKey {
    /// Single-string form, e.g. for logs and tracing fields.
    pub fn canonical(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}",
            self.network, self.contract_id, self.ledger, self.tx_hash, self.event_index
        )
    }
}

/// Decoded contract event payloads that produce database projections.
#[derive(Debug, Clone)]
pub enum ProjectedEvent {
    PoolCreated(PoolCreatedEvent),
    PredictionPlaced(PredictionPlacedEvent),
    PoolResolved { pool_id: u64, winning_outcome: i32 },
}

impl ProjectedEvent {
    fn event_type(&self) -> &'static str {
        match self {
            ProjectedEvent::PoolCreated(_) => "pool_created",
            ProjectedEvent::PredictionPlaced(_) => "prediction_placed",
            ProjectedEvent::PoolResolved { .. } => "pool_resolved",
        }
    }
}

/// An event together with its deduplication key.
#[derive(Debug, Clone)]
pub struct ContractEvent {
    pub key: EventKey,
    pub event: ProjectedEvent,
}

/// Outcome of projecting a batch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectionSummary {
    pub applied: u64,
    pub duplicates: u64,
}

/// Claim `key` for projection. Returns `false` when it was already projected.
async fn claim_event_key(
    tx: &mut Transaction<'_, Postgres>,
    key: &EventKey,
    event_type: &str,
) -> Result<bool, sqlx::Error> {
    let claimed = sqlx::query(
        r#"
        INSERT INTO projected_contract_events
            (network, contract_id, ledger, tx_hash, event_index, event_type)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (network, contract_id, ledger, tx_hash, event_index) DO NOTHING
        "#,
    )
    .bind(&key.network)
    .bind(&key.contract_id)
    .bind(key.ledger as i64)
    .bind(&key.tx_hash)
    .bind(key.event_index as i32)
    .bind(event_type)
    .execute(&mut **tx)
    .await?;
    Ok(claimed.rows_affected() == 1)
}

/// Project one event inside the caller's transaction.
///
/// Returns `true` if the projection was applied, `false` if the event key had
/// already been projected (nothing is written).
#[instrument(skip(tx, event), name = "db.project_event_tx",
    fields(event_key = %event.key.canonical()))]
pub async fn project_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    event: &ContractEvent,
) -> Result<bool, sqlx::Error> {
    if !claim_event_key(tx, &event.key, event.event.event_type()).await? {
        return Ok(false);
    }

    match &event.event {
        ProjectedEvent::PoolCreated(e) => insert_pool_from_event_tx(tx, e).await?,
        ProjectedEvent::PredictionPlaced(e) => insert_prediction_from_event(tx, e).await?,
        ProjectedEvent::PoolResolved {
            pool_id,
            winning_outcome,
        } => {
            resolve_pool_in_db(&mut **tx, *pool_id, *winning_outcome).await?;
            let users: Vec<String> = sqlx::query_scalar(
                "SELECT DISTINCT user_address FROM predictions WHERE pool_id = $1 ORDER BY 1",
            )
            .bind(*pool_id as i64)
            .fetch_all(&mut **tx)
            .await?;
            for user in &users {
                enqueue_notification(
                    tx,
                    user,
                    NotifType::PoolResolved,
                    "Pool resolved",
                    &format!("Pool #{pool_id} has been resolved. Check your result."),
                    *pool_id as i64,
                )
                .await?;
            }
        }
    }
    Ok(true)
}

/// Project a batch of events atomically. Safe to call repeatedly with the same
/// or overlapping batches: already-projected events are skipped.
#[instrument(skip(pool, events), name = "db.project_events", fields(count = events.len()))]
pub async fn project_events(
    pool: &PgPool,
    events: &[ContractEvent],
) -> Result<ProjectionSummary, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mut summary = ProjectionSummary::default();
    for event in events {
        if project_event_tx(&mut tx, event).await? {
            summary.applied += 1;
        } else {
            summary.duplicates += 1;
        }
    }
    tx.commit().await?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_key_includes_every_component() {
        let key = EventKey {
            network: "testnet".into(),
            contract_id: "CABC".into(),
            ledger: 42,
            tx_hash: "deadbeef".into(),
            event_index: 3,
        };
        assert_eq!(key.canonical(), "testnet:CABC:42:deadbeef:3");
    }
}
