use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Exposure by market & outcome (#261)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PortfolioExposureRow {
    pub pool_id: Uuid,
    pub market_id: Option<Uuid>,
    pub market_name: Option<String>,
    pub outcome: String,
    pub amount: i64,
    pub share: f64,
    pub kind: String,
    pub price_available: bool,
    pub estimate_unavailable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioExposure {
    pub total: i64,
    pub rows: Vec<PortfolioExposureRow>,
}

/// Aggregate open positions by canonical market (`pool_id`) and outcome.
///
/// - Closed markets are excluded from open exposure.
/// - Each row carries its share of the indexed portfolio total.
/// - Rows are tagged `estimate` (open, priced) or `claimable` (settled/claimable).
/// - Missing price data sets `price_available = false` and `estimate_unavailable = true`.
pub async fn get_portfolio_exposure(
    pool: &PgPool,
    wallet: &str,
) -> Result<PortfolioExposure, sqlx::Error> {
    #[derive(FromRow)]
    struct RawRow {
        pool_id: Uuid,
        market_id: Option<Uuid>,
        market_name: Option<String>,
        outcome: String,
        amount: i64,
        kind: String,
        price_available: bool,
    }

    let raw = sqlx::query_as::<_, RawRow>(
        r#"
        SELECT
            p.pool_id AS pool_id,
            p.market_id AS market_id,
            m.name AS market_name,
            p.outcome AS outcome,
            SUM(p.amount)::BIGINT AS amount,
            CASE
                WHEN m.status IN ('settled', 'resolved', 'closed') THEN 'claimable'
                ELSE 'estimate'
            END AS kind,
            (m.price IS NOT NULL) AS price_available
        FROM portfolio_positions p
        LEFT JOIN markets m ON m.id = p.market_id
        WHERE p.wallet = $1
          AND p.amount > 0
          AND (m.status IS NULL OR m.status NOT IN ('closed'))
        GROUP BY p.pool_id, p.market_id, m.name, p.outcome, m.status, m.price
        ORDER BY amount DESC
        "#,
    )
    .bind(wallet)
    .fetch_all(pool)
    .await?;

    let total: i64 = raw.iter().map(|r| r.amount).sum();

    let rows = raw
        .into_iter()
        .map(|r| {
            let share = if total > 0 {
                r.amount as f64 / total as f64
            } else {
                0.0
            };
            let estimate_unavailable = r.kind == "estimate" && !r.price_available;
            PortfolioExposureRow {
                pool_id: r.pool_id,
                market_id: r.market_id,
                market_name: r.market_name,
                outcome: r.outcome,
                amount: r.amount,
                share,
                kind: r.kind,
                price_available: r.price_available,
                estimate_unavailable,
            }
        })
        .collect();

    Ok(PortfolioExposure { total, rows })
}

// ---------------------------------------------------------------------------
// Claimable rewards work queue (#262)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ClaimableReward {
    pub claim_id: Uuid,
    pub pool_id: Uuid,
    pub market_id: Option<Uuid>,
    pub market_name: Option<String>,
    pub outcome: Option<String>,
    pub estimated_amount: i64,
    pub status: String,
    pub tx_link: Option<String>,
    pub simulation_error: Option<String>,
}

/// List eligible claims for a wallet with bounded selection and cursor pagination.
pub async fn list_claimable_rewards(
    pool: &PgPool,
    wallet: &str,
    limit: i64,
    cursor: Option<Uuid>,
) -> Result<Vec<ClaimableReward>, sqlx::Error> {
    let limit = limit.clamp(1, 200);

    sqlx::query_as::<_, ClaimableReward>(
        r#"
        SELECT
            c.id AS claim_id,
            c.pool_id AS pool_id,
            c.market_id AS market_id,
            m.name AS market_name,
            c.outcome AS outcome,
            c.estimated_amount AS estimated_amount,
            c.status AS status,
            c.tx_link AS tx_link,
            c.simulation_error AS simulation_error
        FROM portfolio_claims c
        LEFT JOIN markets m ON m.id = c.market_id
        WHERE c.wallet = $1
          AND c.status IN ('eligible', 'simulation_failed', 'pending')
          AND ($2::uuid IS NULL OR c.id > $2)
        ORDER BY c.id ASC
        LIMIT $3
        "#,
    )
    .bind(wallet)
    .bind(cursor)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Record a simulation failure without marking the claim complete.
pub async fn record_claim_simulation_failure(
    pool: &PgPool,
    claim_id: Uuid,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE portfolio_claims
        SET status = 'simulation_failed',
            simulation_error = $2,
            updated_at = NOW()
        WHERE id = $1
          AND status <> 'complete'
        "#,
    )
    .bind(claim_id)
    .bind(error)
    .execute(pool)
    .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Claim reconciliation (#263)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ClaimReconciliation {
    pub id: Uuid,
    pub claim_id: Uuid,
    pub pool_id: Uuid,
    pub market_id: Option<Uuid>,
    pub outcome: Option<String>,
    pub claim_tx: String,
    pub payout_amount: i64,
    pub source_event_id: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimReconciliationStatus {
    pub claim_id: Uuid,
    pub status: String,
    pub reconciled: bool,
    pub payout_amount: Option<i64>,
    pub claim_tx: Option<String>,
}

/// Persist a claimed payout reconciled to its originating position and settlement event.
///
/// Keyed on `source_event_id` with `ON CONFLICT DO NOTHING` so retries and
/// claim-event reordering never double-credit the ledger.
pub async fn reconcile_claim_payout(
    pool: &PgPool,
    claim_id: Uuid,
    pool_id: Uuid,
    market_id: Option<Uuid>,
    outcome: Option<&str>,
    claim_tx: &str,
    payout_amount: i64,
    source_event_id: &str,
) -> Result<Option<ClaimReconciliation>, sqlx::Error> {
    let inserted = sqlx::query_as::<_, ClaimReconciliation>(
        r#"
        INSERT INTO portfolio_claim_reconciliation
            (id, claim_id, pool_id, market_id, outcome, claim_tx, payout_amount, source_event_id, status, created_at)
        VALUES
            ($1, $2, $3, $4, $5, $6, $7, $8, 'reconciled', NOW())
        ON CONFLICT (source_event_id) DO NOTHING
        RETURNING id, claim_id, pool_id, market_id, outcome, claim_tx, payout_amount, source_event_id, status, created_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(claim_id)
    .bind(pool_id)
    .bind(market_id)
    .bind(outcome)
    .bind(claim_tx)
    .bind(payout_amount)
    .bind(source_event_id)
    .fetch_optional(pool)
    .await?;

    Ok(inserted)
}

/// Return reconciliation status for a claim, surfacing `pending`/`delayed`
/// when indexing lags behind the wallet transaction.
pub async fn get_claim_reconciliation_status(
    pool: &PgPool,
    claim_id: Uuid,
) -> Result<ClaimReconciliationStatus, sqlx::Error> {
    #[derive(FromRow)]
    struct Row {
        claim_id: Uuid,
        claim_tx: Option<String>,
        payout_amount: Option<i64>,
        status: Option<String>,
    }

    let row = sqlx::query_as::<_, Row>(
        r#"
        SELECT
            c.id AS claim_id,
            c.tx_link AS claim_tx,
            r.payout_amount AS payout_amount,
            r.status AS status
        FROM portfolio_claims c
        LEFT JOIN portfolio_claim_reconciliation r ON r.claim_id = c.id
        WHERE c.id = $1
        "#,
    )
    .bind(claim_id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(ClaimReconciliationStatus {
            claim_id,
            status: "pending".to_string(),
            reconciled: false,
            payout_amount: None,
            claim_tx: None,
        });
    };

    let reconciled = row.status.as_deref() == Some("reconciled");
    let status = if reconciled {
        "reconciled".to_string()
    } else if row.claim_tx.is_some() {
        // Wallet tx exists but indexing has not caught up yet.
        "delayed".to_string()
    } else {
        "pending".to_string()
    };

    Ok(ClaimReconciliationStatus {
        claim_id: row.claim_id,
        status,
        reconciled,
        payout_amount: row.payout_amount,
        claim_tx: row.claim_tx,
    })
}