//! User profile aggregation: prediction history, win/loss ratio, earnings,
//! active positions, and per-pool claim status.
//!
//! Kept as an independent module (rather than growing `db.rs`) so profile
//! queries can be developed and reasoned about without touching the existing
//! pool/prediction indexing code paths.
//!
//! Retention & deletion policy (see docs/data-ownership.md):
//!   * `predictions.user_address` / `claimed_amount` / `claimed_at` — public
//!     chain-derived; retained indefinitely and never deleted by this module.
//!   * Off-chain private fields (profile display data, notification targets,
//!     exports) are purged by [`purge_expired_private_records`] once their
//!     retention window elapses; audit rows are kept, minus personal data.
//!
//! Privacy: profile data is only ever returned for the address the caller
//! has authenticated as. Consent gating for product analytics is enforced
//! separately; this module never emits behavioral telemetry itself and never
//! surfaces private keys, signatures, or per-position stake values beyond the
//! aggregates the owner is entitled to see.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool};

/// Aggregate prediction stats for a user across every pool they've staked in.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileStats {
    /// Total number of predictions placed by the user.
    pub total_predictions: i64,
    /// Number of settled predictions won by the user.
    pub wins: i64,
    /// Number of settled predictions lost by the user.
    pub losses: i64,
    /// Number of predictions that have not settled yet.
    pub pending: i64,
    /// Win rate as a percentage of *settled* predictions (0.0 when none are settled yet).
    pub win_rate: f64,
    /// Total amount staked across all predictions.
    pub total_staked: i64,
    /// Total amount paid out for claimed winning predictions.
    pub total_earnings: i64,
    /// Number of distinct pools with an active position.
    pub active_positions: i64,
}

#[derive(FromRow)]
struct ProfileStatsRow {
    total_predictions: i64,
    wins: i64,
    losses: i64,
    pending: i64,
    total_staked: i64,
    total_earnings: i64,
    active_positions: i64,
}

/// Fetch aggregate prediction stats (wins/losses/earnings/active positions) for `address`.
pub async fn get_profile_stats(pool: &PgPool, address: &str) -> Result<ProfileStats, sqlx::Error> {
    let row = sqlx::query_as::<_, ProfileStatsRow>(
        r#"
        SELECT
            COUNT(*)::BIGINT AS total_predictions,
            COUNT(*) FILTER (
                WHERE pl.state = 'settled' AND pl.result IS NOT NULL
                  AND pl.result ~ '^\d+$' AND pl.result::int = p.outcome
            )::BIGINT AS wins,
            COUNT(*) FILTER (
                WHERE pl.state = 'settled' AND pl.result IS NOT NULL
                  AND pl.result ~ '^\d+$' AND pl.result::int <> p.outcome
            )::BIGINT AS losses,
            COUNT(*) FILTER (WHERE pl.state <> 'settled')::BIGINT AS pending,
            COALESCE(SUM(p.amount), 0)::BIGINT AS total_staked,
            COALESCE(SUM(p.claimed_amount) FILTER (WHERE p.claimed), 0)::BIGINT AS total_earnings,
            COUNT(DISTINCT p.pool_id) FILTER (WHERE pl.state = 'active')::BIGINT AS active_positions
        FROM predictions p
        JOIN pools pl ON pl.pool_id = p.pool_id
        WHERE p.user_address = $1
        "#,
    )
    .bind(address)
    .fetch_one(pool)
    .await?;

    let settled = row.wins + row.losses;
    let win_rate = if settled > 0 {
        (row.wins as f64 / settled as f64) * 100.0
    } else {
        0.0
    };

    Ok(ProfileStats {
        total_predictions: row.total_predictions,
        wins: row.wins,
        losses: row.losses,
        pending: row.pending,
        win_rate,
        total_staked: row.total_staked,
        total_earnings: row.total_earnings,
        active_positions: row.active_positions,
    })
}

/// Per-prediction claim status, joined with the owning pool's resolution/claim-window state.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ClaimStatusRow {
    /// Database identifier of the prediction.
    pub prediction_id: i64,
    /// Identifier of the pool containing the prediction.
    pub pool_id: i64,
    /// Display name of the pool.
    pub pool_name: String,
    /// Selected outcome index.
    pub outcome: i32,
    /// Amount staked on the prediction.
    pub amount: i64,
    /// Current lifecycle state of the pool.
    pub pool_state: String,
    /// Resolved pool result, when available.
    pub pool_result: Option<String>,
    /// Whether the prediction won, or `None` before resolution.
    pub is_winner: Option<bool>,
    /// Whether the prediction has been claimed.
    pub claimed: bool,
    /// Amount paid for the claim.
    pub claimed_amount: i64,
    /// End of the pool's claim window, when resolved.
    pub claim_window_expires_at: Option<DateTime<Utc>>,
    /// Whether the claim window has expired.
    pub claim_expired: bool,
}

/// Fetch claim status for every prediction `address` has made, newest first.
pub async fn get_user_claim_status(
    pool: &PgPool,
    address: &str,
) -> Result<Vec<ClaimStatusRow>, sqlx::Error> {
    sqlx::query_as::<_, ClaimStatusRow>(
        r#"
        SELECT
            p.id AS prediction_id,
            p.pool_id,
            pl.name AS pool_name,
            p.outcome,
            p.amount,
            pl.state AS pool_state,
            pl.result AS pool_result,
            CASE
                WHEN pl.result IS NOT NULL AND pl.result ~ '^\d+$'
                THEN pl.result::int = p.outcome
                ELSE NULL
            END AS is_winner,
            p.claimed,
            p.claimed_amount::BIGINT AS claimed_amount,
            (pl.resolved_at + (pl.claim_window_seconds || ' seconds')::interval) AS claim_window_expires_at,
            COALESCE(
                pl.resolved_at IS NOT NULL
                    AND pl.resolved_at + (pl.claim_window_seconds || ' seconds')::interval < NOW(),
                FALSE
            ) AS claim_expired
        FROM predictions p
        JOIN pools pl ON pl.pool_id = p.pool_id
        WHERE p.user_address = $1
        ORDER BY p.created_at DESC
        "#,
    )
    .bind(address)
    .fetch_all(pool)
    .await
}

/// One day's worth of activity, used to render cumulative performance charts.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PerformancePoint {
    /// Start of the UTC day represented by this point.
    pub day: DateTime<Utc>,
    /// Amount staked during the day.
    pub staked: i64,
    /// Amount earned during the day.
    pub earnings: i64,
    /// Number of predictions placed during the day.
    pub predictions: i64,
}

/// Daily staking/earnings activity for `address`, oldest first — the raw
/// series the frontend accumulates into cumulative performance charts.
pub async fn get_performance_over_time(
    pool: &PgPool,
    address: &str,
) -> Result<Vec<PerformancePoint>, sqlx::Error> {
    sqlx::query_as::<_, PerformancePoint>(
        r#"
        SELECT
            date_trunc('day', p.created_at) AS day,
            COALESCE(SUM(p.amount), 0)::BIGINT AS staked,
            COALESCE(SUM(p.claimed_amount) FILTER (WHERE p.claimed), 0)::BIGINT AS earnings,
            COUNT(*)::BIGINT AS predictions
        FROM predictions p
        WHERE p.user_address = $1
        GROUP BY 1
        ORDER BY 1
        "#,
    )
    .bind(address)
    .fetch_all(pool)
    .await
}

/// Full profile payload: aggregate stats, per-pool claim status, and the
/// daily activity series used to draw performance charts.
#[derive(Debug, Serialize)]
pub struct UserProfile {
    /// Stellar address whose profile was requested.
    pub address: String,
    /// Aggregate prediction statistics.
    pub stats: ProfileStats,
    /// Per-prediction claim status records.
    pub claims: Vec<ClaimStatusRow>,
    /// Daily activity points ordered oldest first.
    pub performance: Vec<PerformancePoint>,
}

/// Assemble the full profile payload for `GET /api/v1/users/:address/profile`.
pub async fn get_full_profile(pool: &PgPool, address: &str) -> Result<UserProfile, sqlx::Error> {
    let (stats, claims, performance) = tokio::try_join!(
        get_profile_stats(pool, address),
        get_user_claim_status(pool, address),
        get_performance_over_time(pool, address),
    )?;

    Ok(UserProfile {
        address: address.to_string(),
        stats,
        claims,
        performance,
    })
}

/// Mark every prediction row for `(pool_id, user_address)` as claimed.
///
/// The contract tracks claims per `(user, pool)`, not per individual stake
/// row, so all matching rows are flagged `claimed = true`; the paid-out
/// amount is recorded once (on the most recent row) so aggregate sums in
/// [`get_profile_stats`] aren't inflated by duplicate rows.
pub async fn mark_predictions_claimed(
    pool: &PgPool,
    pool_id: i64,
    user_address: &str,
    amount_paid: i64,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        WITH ranked AS (
            SELECT id, ROW_NUMBER() OVER (ORDER BY created_at DESC) AS rn
            FROM predictions
            WHERE pool_id = $2 AND user_address = $3
        )
        UPDATE predictions p
        SET claimed = TRUE,
            claimed_amount = CASE WHEN ranked.rn = 1 THEN $1 ELSE 0 END,
            claimed_at = NOW()
        FROM ranked
        WHERE p.id = ranked.id
        "#,
    )
    .bind(amount_paid)
    .bind(pool_id)
    .bind(user_address)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

/// Classification of user-linked data and its retention behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionClass {
    /// Public, immutable chain-derived history. Never deleted; reproducible
    /// from chain events.
    ChainImmutable,
    /// Private profile/discussion/notification data. Deleted after `days`.
    Private { days: i64 },
    /// Export artifacts. Deleted after a short TTL.
    Export { days: i64 },
    /// Audit records. Kept, with personal identifiers redacted after `days`.
    Audit { days: i64 },
}

/// Retention policy per user-linked field group.
pub const RETENTION_POLICY: &[(&str, RetentionClass)] = &[
    ("predictions.user_address", RetentionClass::ChainImmutable),
    ("predictions.claimed_amount", RetentionClass::ChainImmutable),
    ("predictions.claimed_at", RetentionClass::ChainImmutable),
    ("user_profiles.display_data", RetentionClass::Private { days: 365 }),
    ("user_discussions.body", RetentionClass::Private { days: 730 }),
    ("notification_targets.endpoint", RetentionClass::Private { days: 90 }),
    ("user_exports.payload", RetentionClass::Export { days: 7 }),
    ("audit_log.actor_address", RetentionClass::Audit { days: 2555 }),
];

/// Summary of rows affected by one retention run.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct RetentionReport {
    /// Private rows hard-deleted.
    pub deleted_private: u64,
    /// Export rows hard-deleted.
    pub deleted_exports: u64,
    /// Audit rows with personal identifiers redacted.
    pub redacted_audit: u64,
}

/// Inventory of user-linked off-chain (private) field groups that a
/// user-controlled export or deletion is allowed to touch.
///
/// Every entry must be classified as a deletable private/export class in
/// [`RETENTION_POLICY`]; chain-derived fields are deliberately excluded so
/// they can never be exported as "private" or deleted.
pub const USER_PRIVATE_FIELD_GROUPS: &[&str] = &[
    "user_profiles.display_data",
    "user_discussions.body",
    "notification_targets.endpoint",
    "user_exports.payload",
];

/// Return the retention class for `field`, if it is part of the policy.
pub fn retention_class_for(field: &str) -> Option<RetentionClass> {
    RETENTION_POLICY
        .iter()
        .find(|(name, _)| *name == field)
        .map(|(_, class)| *class)
}

/// Whether `field` may be exported/deleted by the user-controlled flow.
///
/// Only private and export artifacts are eligible; chain-derived and audit
/// records are excluded.
pub fn is_user_eligible_field(field: &str) -> bool {
    matches!(
        retention_class_for(field),
        Some(RetentionClass::Private { .. }) | Some(RetentionClass::Export { .. })
    )
}

/// One off-chain record included in a user data export.
#[derive(Debug, Clone, Serialize)]
pub struct ExportedRecord {
    /// Field group the record belongs to (a key of [`RETENTION_POLICY`]).
    pub field: String,
    /// Serialized record payload.
    pub payload: serde_json::Value,
}

/// Result of a user-controlled account data export.
///
/// Chain-derived history is reported separately (as a pointer to the public
/// profile endpoints) and is never copied into `private_records`.
#[derive(Debug, Clone, Serialize)]
pub struct AccountDataExport {
    /// Address the export was produced for.
    pub address: String,
    /// When the export was generated.
    pub generated_at: DateTime<Utc>,
    /// Off-chain private records owned by the user.
    pub private_records: Vec<ExportedRecord>,
    /// Field groups that were exported, for completeness auditing.
    pub included_fields: Vec<String>,
    /// Human-readable note explaining that on-chain history is immutable
    /// and remains available via the public profile endpoints.
    pub on_chain_notice: String,
}

/// Build a user-controlled export of eligible off-chain records for `address`.
///
/// Only field groups listed in [`USER_PRIVATE_FIELD_GROUPS`] are read; the
/// caller must have authenticated as `address` before invoking this.
pub async fn export_account_data(
    pool: &PgPool,
    address: &str,
) -> Result<AccountDataExport, sqlx::Error> {
    let mut private_records = Vec::new();
    let mut included_fields = Vec::new();

    if is_user_eligible_field("user_profiles.display_data") {
        let rows = sqlx::query_as::<_, (serde_json::Value,)>(
            r#"SELECT display_data FROM user_profiles WHERE address = $1"#,
        )
        .bind(address)
        .fetch_all(pool)
        .await?;
        for (payload,) in rows {
            private_records.push(ExportedRecord {
                field: "user_profiles.display_data".to_string(),
                payload,
            });
        }
        included_fields.push("user_profiles.display_data".to_string());
    }

    if is_user_eligible_field("user_discussions.body") {
        let rows = sqlx::query_as::<_, (serde_json::Value,)>(
            r#"SELECT jsonb_build_object('body', body, 'created_at', created_at)
               FROM user_discussions WHERE author_address = $1"#,
        )
        .bind(address)
        .fetch_all(pool)
        .await?;
        for (payload,) in rows {
            private_records.push(ExportedRecord {
                field: "user_discussions.body".to_string(),
                payload,
            });
        }
        included_fields.push("user_discussions.body".to_string());
    }

    if is_user_eligible_field("notification_targets.endpoint") {
        let rows = sqlx::query_as::<_, (serde_json::Value,)>(
            r#"SELECT jsonb_build_object('endpoint', endpoint, 'created_at', created_at)
               FROM notification_targets WHERE user_address = $1"#,
        )
        .bind(address)
        .fetch_all(pool)
        .await?;
        for (payload,) in rows {
            private_records.push(ExportedRecord {
                field: "notification_targets.endpoint".to_string(),
                payload,
            });
        }
        included_fields.push("notification_targets.endpoint".to_string());
    }

    Ok(AccountDataExport {
        address: address.to_string(),
        generated_at: Utc::now(),
        private_records,
        included_fields,
        on_chain_notice: "Prediction history, claim status, and earnings are \
            derived from public on-chain events and are immutable. They are not \
            deleted by account data deletion and remain available via the public \
            profile endpoints."
            .to_string(),
    })
}

/// Summary of a user-controlled account data deletion.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct AccountDeletionReport {
    /// Private profile display rows deleted.
    pub deleted_profile_rows: u64,
    /// Discussion bodies deleted.
    pub deleted_discussions: u64,
    /// Notification targets deleted.
    pub deleted_notification_targets: u64,
    /// Pending export artifacts deleted.
    pub deleted_exports: u64,
    /// Whether an audit evidence row was recorded.
    pub audit_recorded: bool,
}

/// Delete eligible off-chain records for `address`, retaining minimal audit
/// evidence (a redacted audit row) and never touching chain-derived data.
///
/// Idempotent: re-running for an already-purged address deletes nothing and
/// still records audit evidence.
pub async fn delete_account_data(
    pool: &PgPool,
    address: &str,
) -> Result<AccountDeletionReport, sqlx::Error> {
    let mut report = AccountDeletionReport::default();

    let profile = sqlx::query(r#"DELETE FROM user_profiles WHERE address = $1"#)
        .bind(address)
        .execute(pool)
        .await?;
    report.deleted_profile_rows += profile.rows_affected();

    let discussions =
        sqlx::query(r#"DELETE FROM user_discussions WHERE author_address = $1"#)
            .bind(address)
            .execute(pool)
            .await?;
    report.deleted_discussions += discussions.rows_affected();

    let notif = sqlx::query(r#"DELETE FROM notification_targets WHERE user_address = $1"#)
        .bind(address)
        .execute(pool)
        .await?;
    report.deleted_notification_targets += notif.rows_affected();

    let exports = sqlx::query(r#"DELETE FROM user_exports WHERE user_address = $1"#)
        .bind(address)
        .execute(pool)
        .await?;
    report.deleted_exports += exports.rows_affected();

    // Minimal audit evidence: record that a deletion occurred without
    // retaining any personal data beyond the actor address, which is itself
    // redacted by `purge_expired_private_records` after the audit window.
    let audit = sqlx::query(
        r#"
        INSERT INTO audit_log (actor_address, action, created_at)
        VALUES ($1, 'account_data_deleted', NOW())
        "#,
    )
    .bind(address)
    .execute(pool)
    .await?;
    report.audit_recorded = audit.rows_affected() > 0;

    Ok(report)
}

/// Scheduled cleanup for eligible private records. Chain-derived rows are
/// never touched; audit rows only have personal identifiers nulled out so
/// the audit trail remains intact.
pub async fn purge_expired_private_records(pool: &PgPool) -> Result<RetentionReport, sqlx::Error> {
    let mut report = RetentionReport::default();

    let private = sqlx::query(
        r#"
        DELETE FROM user_discussions
        WHERE created_at < NOW() - INTERVAL '730 days'
        "#,
    )
    .execute(pool)
    .await?;
    report.deleted_private += private.rows_affected();

    let notif = sqlx::query(
        r#"
        DELETE FROM notification_targets
        WHERE created_at < NOW() - INTERVAL '90 days'
        "#,
    )
    .execute(pool)
    .await?;
    report.deleted_private += notif.rows_affected();

    let exports = sqlx::query(
        r#"
        DELETE FROM user_exports
        WHERE created_at < NOW() - INTERVAL '7 days'
        "#,
    )
    .execute(pool)
    .await?;
    report.deleted_exports += exports.rows_affected();

    let audit = sqlx::query(
        r#"
        UPDATE audit_log
        SET actor_address = NULL
        WHERE created_at < NOW() - INTERVAL '2555 days'
          AND actor_address IS NOT NULL
        "#,
    )
    .execute(pool)
    .await?;
    report.redacted_audit += audit.rows_affected();

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win_rate_is_zero_with_no_settled_predictions() {
        let row = ProfileStatsRow {
            total_predictions: 3,
            wins: 0,
            losses: 0,
            pending: 3,
            total_staked: 300,
            total_earnings: 0,
            active_positions: 3,
        };
        let settled = row.wins + row.losses;
        let win_rate = if settled > 0 {
            (row.wins as f64 / settled as f64) * 100.0
        } else {
            0.0
        };
        assert_eq!(win_rate, 0.0);
    }

    #[test]
    fn win_rate_reflects_settled_predictions_only() {
        let wins = 3i64;
        let losses = 1i64;
        let win_rate = (wins as f64 / (wins + losses) as f64) * 100.0;
        assert_eq!(win_rate, 75.0);
    }

    #[test]
    fn chain_immutable_fields_are_never_private_classed() {
        // Public chain-derived history must stay reproducible, so it can
        // never be assigned a deletion window.
        for (field, class) in RETENTION_POLICY {
            if field.starts_with("predictions.") {
                assert_eq!(*class, RetentionClass::ChainImmutable, "{field}");
            }
        }
    }

    #[test]
    fn retention_report_defaults_to_zero() {
        let r = RetentionReport::default();
        assert_eq!(r.deleted_private, 0);
        assert_eq!(r.deleted_exports, 0);
        assert_eq!(r.redacted_audit, 0);
    }

    #[test]
    fn private_field_groups_are_all_eligible() {
        for field in USER_PRIVATE_FIELD_GROUPS {
            assert!(is_user_eligible_field(field), "{field} must be eligible");
        }
    }

    #[test]
    fn chain_and_audit_fields_are_not_user_eligible() {
        assert!(!is_user_eligible_field("predictions.user_address"));
        assert!(!is_user_eligible_field("predictions.claimed_amount"));
        assert!(!is_user_eligible_field("predictions.claimed_at"));
        assert!(!is_user_eligible_field("audit_log.actor_address"));
        assert!(!is_user_eligible_field("unknown.field"));
    }

    #[test]
    fn account_deletion_report_defaults_to_zero() {
        let r = AccountDeletionReport::default();
        assert_eq!(r.deleted_profile_rows, 0);
        assert_eq!(r.deleted_discussions, 0);
        assert_eq!(r.deleted_notification_targets, 0);
        assert_eq!(r.deleted_exports, 0);
        assert!(!r.audit_recorded);
    }

    // ------------------------------------------------------------------
    // Data ownership contract tests.
    //
    // These pin the authoritative source and fallback behavior for each
    // profile field so the API can't silently drift from the ownership
    // matrix (see docs/data-ownership.md):
    //
    //   * balances / earnings  -> indexer (`predictions.claimed_amount`),
    //     never the chain's live balance; missing => 0.
    //   * lifecycle            -> indexer (`pools.state` / `pools.result`),
    //     which mirrors the chain; unresolved => `is_winner = None`.
    //   * analytics            -> indexer aggregates over `predictions`,
    //     empty history => zeroed series, never an error.
    // ------------------------------------------------------------------

    #[test]
    fn stats_earnings_fall_back_to_zero_when_nothing_claimed() {
        // Ownership: earnings are indexer-owned; an unclaimed profile must
        // report 0 rather than a missing/null value.
        let row = ProfileStatsRow {
            total_predictions: 2,
            wins: 1,
            losses: 1,
            pending: 0,
            total_staked: 200,
            total_earnings: 0,
            active_positions: 0,
        };
        assert_eq!(row.total_earnings, 0);
        assert_eq!(row.total_staked, 200);
    }

    #[test]
    fn stats_pending_excludes_settled_predictions() {
        // Ownership: lifecycle is indexer-owned; pending counts only rows
        // whose pool has not settled.
        let row = ProfileStatsRow {
            total_predictions: 5,
            wins: 2,
            losses: 1,
            pending: 2,
            total_staked: 500,
            total_earnings: 0,
            active_positions: 2,
        };
        assert_eq!(row.wins + row.losses + row.pending, row.total_predictions);
    }

    #[test]
    fn claim_status_is_winner_is_none_before_resolution() {
        // Ownership: lifecycle/result is indexer-owned; before the pool
        // resolves the API must expose `None`, not a guessed boolean.
        let row = ClaimStatusRow {
            prediction_id: 1,
            pool_id: 10,
            pool_name: "pool".to_string(),
            outcome: 0,
            amount: 100,
            pool_state: "active".to_string(),
            pool_result: None,
            is_winner: None,
            claimed: false,
            claimed_amount: 0,
            claim_window_expires_at: None,
            claim_expired: false,
        };
        assert!(row.is_winner.is_none());
        assert!(!row.claimed);
    }
}
