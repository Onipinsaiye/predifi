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
//!
//! Event topic names and payload layouts are decoded through versioned,
//! release-pinned schemas (see [`SchemaRegistry`]). Unknown topics or payload
//! layouts produce explicit [`DecodeError`] results instead of guessing.

use sqlx::{PgPool, Postgres, Transaction};
use tracing::instrument;

use crate::notifications::{enqueue_notification_for_event, NotifType};

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

// ---------------------------------------------------------------------------
// Versioned schema registry
// ---------------------------------------------------------------------------

/// Field type of a topic/payload slot in a pinned contract schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    U64,
    I32,
    Str,
    Address,
}

/// A single named field in an event payload layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSpec {
    pub name: &'static str,
    pub kind: FieldKind,
}

/// Pinned layout of one event topic within one contract release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventSchema {
    /// Topic symbol as emitted on-chain (e.g. `pool_created`).
    pub topic: &'static str,
    pub fields: &'static [FieldSpec],
}

/// Release-pinned contract metadata: every event topic the release emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContractSchema {
    /// Contract release identifier (e.g. `v1`, `v2`).
    pub version: &'static str,
    pub events: &'static [EventSchema],
}

const POOL_CREATED_V1: &[FieldSpec] = &[
    FieldSpec { name: "pool_id", kind: FieldKind::U64 },
    FieldSpec { name: "creator", kind: FieldKind::Address },
    FieldSpec { name: "question", kind: FieldKind::Str },
    FieldSpec { name: "options", kind: FieldKind::Str },
    FieldSpec { name: "deadline", kind: FieldKind::U64 },
    FieldSpec { name: "created_at", kind: FieldKind::U64 },
];

const PREDICTION_PLACED_V1: &[FieldSpec] = &[
    FieldSpec { name: "pool_id", kind: FieldKind::U64 },
    FieldSpec { name: "user", kind: FieldKind::Address },
    FieldSpec { name: "outcome", kind: FieldKind::I32 },
    FieldSpec { name: "amount", kind: FieldKind::U64 },
];

const POOL_RESOLVED_V1: &[FieldSpec] = &[
    FieldSpec { name: "pool_id", kind: FieldKind::U64 },
    FieldSpec { name: "winning_outcome", kind: FieldKind::I32 },
];

const SCHEMA_V1: ContractSchema = ContractSchema {
    version: "v1",
    events: &[
        EventSchema { topic: "pool_created", fields: POOL_CREATED_V1 },
        EventSchema { topic: "prediction_placed", fields: PREDICTION_PLACED_V1 },
        EventSchema { topic: "pool_resolved", fields: POOL_RESOLVED_V1 },
    ],
};

/// All contract releases this indexer can decode, oldest first.
pub const SUPPORTED_SCHEMAS: &[ContractSchema] = &[SCHEMA_V1];

/// Errors produced when decoding raw events against pinned schemas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// No supported release is known for this contract version tag.
    UnknownVersion { version: String },
    /// The release is known but does not define this topic.
    UnknownTopic { version: &'static str, topic: String },
    /// Payload field count or kinds do not match the pinned layout.
    SchemaDrift {
        version: &'static str,
        topic: &'static str,
        expected: usize,
        actual: usize,
    },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::UnknownVersion { version } => {
                write!(f, "unknown contract schema version: {version}")
            }
            DecodeError::UnknownTopic { version, topic } => {
                write!(f, "topic `{topic}` not defined in schema {version}")
            }
            DecodeError::SchemaDrift {
                version,
                topic,
                expected,
                actual,
            } => write!(
                f,
                "schema drift in {version}/{topic}: expected {expected} fields, got {actual}"
            ),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Look up a pinned contract schema by release version.
///
/// Returns an explicit [`DecodeError::UnknownVersion`] for any version that is
/// not in [`SUPPORTED_SCHEMAS`]; callers must never fall back to a guess.
pub fn schema_for_version(version: &str) -> Result<&'static ContractSchema, DecodeError> {
    SUPPORTED_SCHEMAS
        .iter()
        .find(|s| s.version == version)
        .ok_or_else(|| DecodeError::UnknownVersion {
            version: version.to_string(),
        })
}

/// Resolve the pinned layout of `topic` within a release.
pub fn event_schema<'a>(
    schema: &'a ContractSchema,
    topic: &str,
) -> Result<&'a EventSchema, DecodeError> {
    schema
        .events
        .iter()
        .find(|e| e.topic == topic)
        .ok_or_else(|| DecodeError::UnknownTopic {
            version: schema.version,
            topic: topic.to_string(),
        })
}

/// Verify that a raw payload's field count matches the pinned layout.
///
/// Any mismatch is reported as [`DecodeError::SchemaDrift`]; the decoder never
/// silently truncates or pads a payload.
pub fn check_payload_shape(
    schema: &ContractSchema,
    event: &EventSchema,
    actual_fields: usize,
) -> Result<(), DecodeError> {
    let expected = event.fields.len();
    if expected != actual_fields {
        return Err(DecodeError::SchemaDrift {
            version: schema.version,
            topic: event.topic,
            expected,
            actual: actual_fields,
        });
    }
    Ok(())
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

/// Record a wallet-attributable activity row in the same transaction as the
/// domain write (#309). `ON CONFLICT DO NOTHING` makes this idempotent
/// alongside the dedupe claim above: a retried/overlapping batch never
/// duplicates a wallet's activity entry.
#[allow(clippy::too_many_arguments)]
async fn record_activity(
    tx: &mut Transaction<'_, Postgres>,
    key: &EventKey,
    event_type: &str,
    user_address: &str,
    pool_id: Option<i64>,
    counterparty: Option<&str>,
    outcome: Option<i32>,
    amount: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO portfolio_activity_ledger
            (network, contract_id, ledger, tx_hash, event_index, event_type,
             user_address, pool_id, counterparty, outcome, amount)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (network, contract_id, ledger, tx_hash, event_index, user_address)
        DO NOTHING
        "#,
    )
    .bind(&key.network)
    .bind(&key.contract_id)
    .bind(key.ledger as i64)
    .bind(&key.tx_hash)
    .bind(key.event_index as i32)
    .bind(event_type)
    .bind(user_address)
    .bind(pool_id)
    .bind(counterparty)
    .bind(outcome)
    .bind(amount)
    .execute(&mut **tx)
    .await?;
    Ok(())
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
        ProjectedEvent::PoolCreated(e) => {
            insert_pool_from_event_tx(tx, e).await?;
            record_activity(
                tx,
                &event.key,
                "pool_created",
                &e.creator,
                Some(e.pool_id as i64),
                Some(&e.creator),
                None,
                None,
            )
            .await?;
        }
        ProjectedEvent::PredictionPlaced(e) => {
            insert_prediction_from_event(tx, e).await?;
            record_activity(
                tx,
                &event.key,
                "prediction_placed",
                &e.user_address,
                Some(e.pool_id as i64),
                None,
                Some(e.outcome),
                Some(e.amount),
            )
            .await?;
        }
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
                record_activity(
                    tx,
                    &event.key,
                    "pool_resolved",
                    user,
                    Some(*pool_id as i64),
                    None,
                    Some(*winning_outcome),
                    None,
                )
                .await?;
                enqueue_notification_for_event(
                    tx,
                    user,
                    NotifType::PoolResolved,
                    "Pool resolved",
                    &format!("Pool #{pool_id} has been resolved. Check your result."),
                    *pool_id as i64,
                    &event.key.canonical(),
                )
                .await?;
            }
        }
    }
    Ok(true)
}

/// Project a batch of events atomically. Safe to call repeatedly with the same
/// or overlapping batches: al
                    tx,
                    user,
                    NotifType::PoolResolved,
                    "Pool resolved",
                    &format!("Pool #{pool_id} has been resolved. Check your result."),
                    *pool_id as i64,
                    &event.key.canonical(),
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

    // --- Data ownership contract tests (#356) -------------------------------
    //
    // Ownership matrix (see docs/data-ownership.md):
    //   * `projected_contract_events` is the authoritative record of which
    //     chain events have been projected. The chain is the source of truth
    //     for event identity; the indexer never rewrites a claimed key.
    //   * Domain rows (pools, predictions, resolutions) are derived from the
    //     chain event and must reflect the chain values verbatim.
    //   * Conflict behavior: a duplicate event key is a no-op — the first
    //     projection wins and later conflicting payloads are ignored.

    fn sample_key(event_index: u32) -> EventKey {
        EventKey {
            network: "testnet".into(),
            contract_id: "CABC".into(),
            ledger: 42,
            tx_hash: "deadbeef".into(),
            event_index,
        }
    }

    #[test]
    fn event_key_identity_is_stable_across_retries() {
        // The same on-chain event must always produce the same canonical key,
        // so retries and overlapping ledger windows dedupe to one projection.
        let a = sample_key(3);
        let b = sample_key(3);
        assert_eq!(a, b);
        assert_eq!(a.canonical(), b.canonical());
    }

    #[test]
    fn distinct_event_positions_are_distinct_keys() {
        // Two events in the same tx are distinct on-chain facts and must not
        // collapse into one projection.
        assert_ne!(sample_key(0), sample_key(1));
        assert_ne!(sample_key(0).canonical(), sample_key(1).canonical());
    }

    #[test]
    fn event_type_labels_match_ownership_matrix() {
        // The persisted `event_type` column is part of the ownership contract;
        // these labels are consumed by analytics and must stay stable.
        let pool_created = ProjectedEvent::PoolCreated(PoolCreatedEvent {
            pool_id: 1,
            creator: "GABC".into(),
            end_time: 0,
            token: "CTOKEN".into(),
            category: "Crypto".into(),
            description: "q".into(),
        });
        assert_eq!(pool_created.event_type(), "pool_created");

        let resolved = ProjectedEvent::PoolResolved {
            pool_id: 1,
            winning_outcome: 0,
        };
        assert_eq!(resolved.event_type(), "pool_resolved");
    }

    #[test]
    fn projection_summary_defaults_to_zero() {
        // A batch with no events must report no applied/duplicate projections.
        let summary = ProjectionSummary::default();
        assert_eq!(summary.applied, 0);
        assert_eq!(summary.duplicates, 0);
    }

    // --- Versioned schema decoding -----------------------------------------

    #[test]
    fn unknown_version_is_explicit_not_guessed() {
        let err = schema_for_version("v999").unwrap_err();
        assert_eq!(
            err,
            DecodeError::UnknownVersion {
                version: "v999".into()
            }
        );
    }

    #[test]
    fn unknown_topic_is_explicit_not_guessed() {
        let schema = schema_for_version("v1").unwrap();
        let err = event_schema(schema, "not_a_topic").unwrap_err();
        assert!(matches!(err, DecodeError::UnknownTopic { version: "v1", .. }));
    }

    #[test]
    fn payload_shape_drift_is_detected() {
        let schema = schema_for_version("v1").unwrap();
        let event = event_schema(schema, "pool_resolved").unwrap();
        assert!(check_payload_shape(schema, event, 2).is_ok());
        let err = check_payload_shape(schema, event, 3).unwrap_err();
        assert_eq!(
            err,
            DecodeError::SchemaDrift {
                version: "v1",
                topic: "pool_resolved",
                expected: 2,
                actual: 3,
            }
        );
    }

    #[test]
    fn every_supported_release_declares_all_projected_topics() {
        // Every release we keep decoders for must cover the topics the
        // projection layer consumes, so archived vectors never silently lose
        // a projection path.
        for schema in SUPPORTED_SCHEMAS {
            for topic in ["pool_created", "prediction_placed", "pool_resolved"] {
                assert!(
                    event_schema(schema, topic).is_ok(),
                    "schema {} missing topic {topic}",
                    schema.version
                );
            }
        }
    }
}
