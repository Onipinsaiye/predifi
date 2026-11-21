//! Portfolio snapshots generated from committed indexer checkpoints (#311).
//!
//! A snapshot must never mix events from an uncommitted/partially-applied
//! indexer batch. [`generate_portfolio_snapshot`] enforces that by:
//!
//! 1. Reading the listener's committed checkpoint via
//!    [`crate::db::checkpoints::load_checkpoint`]. Generation refuses to run
//!    (returns [`SnapshotError::CheckpointNotReady`]) unless that checkpoint
//!    is [`CheckpointStatus::Valid`] — never against a missing or malformed
//!    checkpoint.
//! 2. Aggregating exclusively from `portfolio_activity_ledger` rows with
//!    `ledger <= committed_checkpoint_ledger`. Ledger rows are written in
//!    the same transaction as the dedupe claim and the domain write (see
//!    `crate::db::event_projection::project_event_tx`), so every row that
//!    passes this filter is guaranteed fully committed; nothing from an
//!    in-flight transaction can ever be included, because an in-flight
//!    transaction has not committed any row at all yet.
//!
//! Because the aggregation query is a deterministic function of
//! `(wallet_address, checkpoint_ledger)` over already-committed rows,
//! recomputing a snapshot for the same checkpoint always yields the same
//! result — recomputation and the original computation use the identical
//! query, so there is no separate "incremental vs. from-scratch" code path
//! to drift apart.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool};

use super::checkpoints::{load_checkpoint, CheckpointStatus, DEFAULT_LISTENER_ID};
use super::portfolio_pnl::PNL_MODEL_VERSION;

/// Why a snapshot could not be generated.
#[derive(Debug)]
pub enum SnapshotError {
    /// The listener checkpoint is missing or malformed, so there is no safe
    /// ledger boundary to compute against.
    CheckpointNotReady(String),
    Database(sqlx::Error),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::CheckpointNotReady(reason) => {
                write!(f, "checkpoint not ready: {reason}")
            }
            SnapshotError::Database(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<sqlx::Error> for SnapshotError {
    fn from(e: sqlx::Error) -> Self {
        SnapshotError::Database(e)
    }
}

/// A persisted portfolio snapshot: its source checkpoint, the model version
/// it was computed under, and the aggregate figures themselves.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PortfolioSnapshot {
    pub id: i64,
    pub user_address: String,
    /// Ledger the snapshot was computed against — its source cursor.
    pub checkpoint_ledger: i64,
    pub pnl_model_version: i32,
    /// Total staked across every `prediction_placed` activity row at or
    /// before `checkpoint_ledger`.
    pub total_staked: i64,
    /// Total realized return across every `claim` activity row at or
    /// before `checkpoint_ledger`.
    pub total_realized_return: i64,
    /// `total_realized_return - total_staked`. Deliberately *not* scoped to
    /// only the claimed positions' stake: it is "return realized so far
    /// minus everything ever staked", which trends toward the true net P&L
    /// as more positions are claimed and is well-defined even with open
    /// positions outstanding.
    pub net_realized_pnl: i64,
    /// Distinct pools staked into that have no recorded claim yet, at or
    /// before `checkpoint_ledger`.
    pub open_positions: i64,
    pub computed_at: DateTime<Utc>,
}

/// Generate (or recompute) `wallet_address`'s portfolio snapshot against
/// the most recently committed listener checkpoint.
///
/// Returns [`SnapshotError::CheckpointNotReady`] when the checkpoint is
/// missing or malformed, rather than guessing a ledger boundary.
pub async fn generate_portfolio_snapshot(
    pool: &PgPool,
    wallet_address: &str,
) -> Result<PortfolioSnapshot, SnapshotError> {
    let checkpoint_ledger = match load_checkpoint(pool, DEFAULT_LISTENER_ID).await {
        CheckpointStatus::Valid(ledger) => ledger as i64,
        CheckpointStatus::Missing => {
            return Err(SnapshotError::CheckpointNotReady(
                "listener checkpoint not yet established".to_string(),
            ));
        }
        CheckpointStatus::Malformed(reason) => {
            return Err(SnapshotError::CheckpointNotReady(reason));
        }
    };

    generate_portfolio_snapshot_at(pool, wallet_address, checkpoint_ledger).await
}

/// Generate a snapshot against an explicit checkpoint ledger. Exposed
/// separately from [`generate_portfolio_snapshot`] so tests can exercise
/// checkpoint-advancement and crash-recovery scenarios deterministically
/// without racing the live listener.
/// The checkpoint-gated aggregation query, shared between
/// [`generate_portfolio_snapshot_at`] (which runs it) and
/// [`explain_portfolio_snapshot_plan`] (which asks Postgres to `EXPLAIN`
/// it) — see `crate::db::predictions::leaderboard_sql` for the same pattern
/// — so the plan a test inspects can never drift from the query actually
/// served (#316). Placeholders: `$1` = wallet address, `$2` = checkpoint
/// ledger.
fn snapshot_aggregation_sql() -> &'static str {
    r#"
        SELECT
            COALESCE(SUM(amount) FILTER (WHERE event_type = 'prediction_placed'), 0)::BIGINT
                AS total_staked,
            COALESCE(SUM(amount) FILTER (WHERE event_type = 'claim'), 0)::BIGINT
                AS total_realized_return,
            COUNT(DISTINCT pool_id) FILTER (
                WHERE event_type = 'prediction_placed'
                  AND pool_id NOT IN (
                      SELECT pool_id FROM portfolio_activity_ledger
                      WHERE user_address = $1 AND event_type = 'claim' AND ledger <= $2
                  )
            )::BIGINT AS open_positions
        FROM portfolio_activity_ledger
        WHERE user_address = $1 AND ledger <= $2
        "#
}

pub async fn generate_portfolio_snapshot_at(
    pool: &PgPool,
    wallet_address: &str,
    checkpoint_ledger: i64,
) -> Result<PortfolioSnapshot, SnapshotError> {
    let row = sqlx::query_as::<_, (i64, i64, i64)>(snapshot_aggregation_sql())
        .bind(wallet_address)
        .bind(checkpoint_ledger)
        .fetch_one(pool)
        .await?;

    let (total_staked, total_realized_return, open_positions) = row;
    let net_realized_pnl = total_realized_return - total_staked;

    let snapshot = sqlx::query_as::<_, PortfolioSnapshot>(
        r#"
        INSERT INTO portfolio_snapshots
            (user_address, checkpoint_ledger, pnl_model_version, total_staked,
             total_realized_return, net_realized_pnl, open_positions, computed_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
        ON CONFLICT (user_address, checkpoint_ledger) DO UPDATE
            SET pnl_model_version      = EXCLUDED.pnl_model_version,
                total_staked           = EXCLUDED.total_staked,
                total_realized_return  = EXCLUDED.total_realized_return,
                net_realized_pnl       = EXCLUDED.net_realized_pnl,
                open_positions         = EXCLUDED.open_positions,
                computed_at            = EXCLUDED.computed_at
        RETURNING id, user_address, checkpoint_ledger, pnl_model_version, total_staked,
                  total_realized_return, net_realized_pnl, open_positions, computed_at
        "#,
    )
    .bind(wallet_address)
    .bind(checkpoint_ledger)
    .bind(PNL_MODEL_VERSION as i32)
    .bind(total_staked)
    .bind(total_realized_return)
    .bind(net_realized_pnl)
    .bind(open_positions)
    .fetch_one(pool)
    .await?;

    Ok(snapshot)
}

/// Run `EXPLAIN` on the checkpoint-gated aggregation query used by
/// [`generate_portfolio_snapshot_at`] and return the plan as a single
/// string (#316), so a test can assert the planner uses
/// `idx_portfolio_activity_ledger_wallet_ledger` rather than a sequential
/// scan as a wallet's ledger grows.
pub async fn explain_portfolio_snapshot_plan(
    pool: &PgPool,
    wallet_address: &str,
    checkpoint_ledger: i64,
) -> Result<String, sqlx::Error> {
    let sql = format!("EXPLAIN {}", snapshot_aggregation_sql());
    let lines: Vec<String> = sqlx::query_scalar(&sql)
        .bind(wallet_address)
        .bind(checkpoint_ledger)
        .fetch_all(pool)
        .await?;
    Ok(lines.join("\n"))
}

#[cfg(all(test, feature = "integration-tests"))]
mod db_tests {
    use super::*;
    use crate::db::checkpoints::save_checkpoint;
    use sqlx::PgPool;

    async fn insert_activity(
        pool: &PgPool,
        user: &str,
        ledger: i64,
        tx_hash: &str,
        event_index: i32,
        event_type: &str,
        pool_id: i64,
        amount: i64,
    ) {
        sqlx::query(
            r#"
            INSERT INTO portfolio_activity_ledger
                (network, contract_id, ledger, tx_hash, event_index, event_type,
                 user_address, pool_id, amount)
            VALUES ('testnet', 'CTEST', $1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(ledger)
        .bind(tx_hash)
        .bind(event_index)
        .bind(event_type)
        .bind(user)
        .bind(pool_id)
        .bind(amount)
        .execute(pool)
        .await
        .expect("insert activity row");
    }

    /// Crash recovery: a checkpoint that didn't advance past ledger 5 must
    /// never see the ledger-10 event, even though the row already exists in
    /// the table (simulating a listener crash between writing rows and
    /// advancing its checkpoint).
    #[sqlx::test]
    async fn snapshot_excludes_events_beyond_an_unadvanced_checkpoint(pool: PgPool) {
        let wallet = "GCRASHRECOVERYTESTWALLET0000000000000000000000000000000";
        insert_activity(&pool, wallet, 5, "tx-a", 0, "prediction_placed", 1, 100).await;
        insert_activity(&pool, wallet, 10, "tx-b", 0, "prediction_placed", 2, 500).await;

        let snapshot = generate_portfolio_snapshot_at(&pool, wallet, 5)
            .await
            .expect("snapshot at ledger 5");
        assert_eq!(snapshot.total_staked, 100, "must not include the ledger-10 row");
    }

    /// Checkpoint advancement: regenerating after the checkpoint advances
    /// includes the newly committed event, and only it.
    #[sqlx::test]
    async fn snapshot_includes_newly_committed_events_after_checkpoint_advances(pool: PgPool) {
        let wallet = "GCHECKPOINTADVANCETESTWALLET000000000000000000000000000";
        insert_activity(&pool, wallet, 5, "tx-a", 0, "prediction_placed", 1, 100).await;

        let before = generate_portfolio_snapshot_at(&pool, wallet, 5)
            .await
            .expect("snapshot before advance");
        assert_eq!(before.total_staked, 100);

        insert_activity(&pool, wallet, 10, "tx-b", 0, "prediction_placed", 2, 500).await;
        let after = generate_portfolio_snapshot_at(&pool, wallet, 10)
            .await
            .expect("snapshot after advance");
        assert_eq!(after.total_staked, 600);
    }

    /// Recomputation for a fixed checkpoint is deterministic: calling it
    /// twice against the same checkpoint and the same committed rows
    /// produces identical values both times.
    #[sqlx::test]
    async fn recomputation_for_a_fixed_checkpoint_is_deterministic(pool: PgPool) {
        let wallet = "GDETERMINISMTESTWALLET0000000000000000000000000000000000";
        insert_activity(&pool, wallet, 5, "tx-a", 0, "prediction_placed", 1, 100).await;
        insert_activity(&pool, wallet, 5, "tx-a", 1, "claim", 1, 150).await;

        let first = generate_portfolio_snapshot_at(&pool, wallet, 5)
            .await
            .expect("first computation");
        let second = generate_portfolio_snapshot_at(&pool, wallet, 5)
            .await
            .expect("second computation (recompute)");

        assert_eq!(first.total_staked, second.total_staked);
        assert_eq!(first.total_realized_return, second.total_realized_return);
        assert_eq!(first.net_realized_pnl, second.net_realized_pnl);
        assert_eq!(first.open_positions, second.open_positions);
        assert_eq!(first.total_staked, 100);
        assert_eq!(first.total_realized_return, 150);
        assert_eq!(first.net_realized_pnl, 50);
        assert_eq!(first.open_positions, 0);
    }

    #[sqlx::test]
    async fn generate_refuses_when_checkpoint_is_missing(pool: PgPool) {
        let err = generate_portfolio_snapshot(&pool, "GSOMEWALLET0000000000000000000000000000000000000000000")
            .await
            .expect_err("missing checkpoint must refuse to generate a snapshot");
        assert!(matches!(err, SnapshotError::CheckpointNotReady(_)));
    }

    #[sqlx::test]
    async fn generate_uses_the_committed_checkpoint(pool: PgPool) {
        let wallet = "GUSESCOMMITTEDCHECKPOINTWALLET00000000000000000000000000";
        insert_activity(&pool, wallet, 3, "tx-a", 0, "prediction_placed", 1, 42).await;
        save_checkpoint(&pool, DEFAULT_LISTENER_ID, 3)
            .await
            .expect("save checkpoint");

        let snapshot = generate_portfolio_snapshot(&pool, wallet)
            .await
            .expect("snapshot against committed checkpoint");
        assert_eq!(snapshot.checkpoint_ledger, 3);
        assert_eq!(snapshot.total_staked, 42);
    }
}
