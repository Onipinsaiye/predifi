//! Query-plan regression checks for the portfolio ledger, snapshot and
//! export queries (#316).
//!
//! Follows the same pattern as `tests/market_filter_performance.rs` /
//! `crate::db::predictions::explain_leaderboard_plan`: seed a Postgres
//! container, run every migration, seed a small and a high-volume wallet
//! dataset, then assert `EXPLAIN` output uses the expected index rather
//! than a sequential scan for each query, plus a lightweight row-count /
//! timing sanity check across dataset sizes.
//!
//! Run explicitly with:
//! `cargo test --features integration-tests --test portfolio_query_plans -- --ignored --nocapture`

#![cfg(feature = "integration-tests")]

use chrono::Utc;
use predifi_backend::db::{
    checkpoints::save_checkpoint, explain_export_rows_plan, explain_portfolio_snapshot_plan,
    explain_wallet_activity_plan, DEFAULT_LISTENER_ID,
};
use sqlx::PgPool;

/// Row counts for the small vs. high-volume seeded wallet datasets (#316:
/// "seeded against both a small and a high-volume wallet dataset fixture").
const SMALL_DATASET_ROWS: i64 = 50;
const LARGE_DATASET_ROWS: i64 = 8_000;

async fn seed_wallet_ledger(pool: &PgPool, wallet: &str, pool_id: i64, rows: i64) {
    for i in 0..rows {
        sqlx::query(
            r#"
            INSERT INTO portfolio_activity_ledger
                (network, contract_id, ledger, tx_hash, event_index, event_type,
                 user_address, pool_id, amount)
            VALUES ('testnet', 'CTEST', $1, $2, 0, 'prediction_placed', $3, $4, 10)
            "#,
        )
        .bind(i + 1)
        .bind(format!("{wallet}-tx-{i}"))
        .bind(wallet)
        .bind(pool_id)
        .execute(pool)
        .await
        .expect("seed wallet ledger row");
    }
}

async fn migrated_pool(database_url: &str) -> PgPool {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await
        .expect("connect to test postgres");
    sqlx::migrate::Migrator::new(std::path::Path::new("migrations"))
        .await
        .expect("load migrator")
        .run(&pool)
        .await
        .expect("apply migrations");
    pool
}

#[tokio::test]
#[ignore = "Requires Docker container for Postgres"]
async fn ledger_snapshot_and_export_queries_use_their_indexes_at_scale() {
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    let container = Postgres::default()
        .start()
        .await
        .expect("postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("postgres port");
    let database_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pool = migrated_pool(&database_url).await;

    save_checkpoint(
        &pool,
        DEFAULT_LISTENER_ID,
        LARGE_DATASET_ROWS + SMALL_DATASET_ROWS + 10,
    )
    .await
    .expect("save checkpoint");

    let small_wallet = "GSMALLDATASETWALLET00000000000000000000000000000000000";
    let large_wallet = "GLARGEDATASETWALLET00000000000000000000000000000000000";
    seed_wallet_ledger(&pool, small_wallet, 1, SMALL_DATASET_ROWS).await;
    seed_wallet_ledger(&pool, large_wallet, 2, LARGE_DATASET_ROWS).await;

    // ── #309 ledger: must use the wallet-cursor index, never a seq scan ────
    for (wallet, label) in [(small_wallet, "small"), (large_wallet, "large")] {
        let plan = explain_wallet_activity_plan(&pool, wallet, None, 20)
            .await
            .expect("explain wallet activity plan");
        assert!(
            plan.contains("idx_portfolio_activity_ledger_wallet_cursor"),
            "{label} dataset: wallet activity query must use the wallet-cursor index, got:\n{plan}"
        );
        assert!(
            !plan.contains("Seq Scan"),
            "{label} dataset: wallet activity query must not sequentially scan the ledger, got:\n{plan}"
        );
    }

    // ── #311 snapshot: must use the wallet-ledger checkpoint index ─────────
    for (wallet, label) in [(small_wallet, "small"), (large_wallet, "large")] {
        let plan = explain_portfolio_snapshot_plan(&pool, wallet, LARGE_DATASET_ROWS + 10)
            .await
            .expect("explain snapshot plan");
        assert!(
            plan.contains("idx_portfolio_activity_ledger_wallet_ledger"),
            "{label} dataset: snapshot aggregation must use the wallet-ledger index, got:\n{plan}"
        );
        assert!(
            !plan.contains("Seq Scan"),
            "{label} dataset: snapshot aggregation must not sequentially scan the ledger, got:\n{plan}"
        );
    }

    // ── #312 export: the bounded batch query must also use that index ─────
    for (wallet, label) in [(small_wallet, "small"), (large_wallet, "large")] {
        let plan = explain_export_rows_plan(&pool, wallet, LARGE_DATASET_ROWS + 10, 0, 500)
            .await
            .expect("explain export rows plan");
        assert!(
            plan.contains("idx_portfolio_activity_ledger_wallet_ledger"),
            "{label} dataset: export batch query must use the wallet-ledger index, got:\n{plan}"
        );
        assert!(
            !plan.contains("Seq Scan"),
            "{label} dataset: export batch query must not sequentially scan the ledger, got:\n{plan}"
        );
    }

    // ── Lightweight row-count / timing sanity check across dataset sizes ──
    // Not a strict regression benchmark (no criterion suite exists in this
    // crate yet) — just confirms the indexed query stays bounded and fast
    // even as the wallet's history grows roughly 160x.
    let small_elapsed = {
        let started = std::time::Instant::now();
        let page = predifi_backend::db::get_wallet_activity(&pool, small_wallet, None, 100)
            .await
            .expect("fetch small wallet page");
        assert_eq!(page.items.len() as i64, SMALL_DATASET_ROWS.min(100));
        started.elapsed()
    };
    let large_elapsed = {
        let started = std::time::Instant::now();
        let page = predifi_backend::db::get_wallet_activity(&pool, large_wallet, None, 100)
            .await
            .expect("fetch large wallet page");
        assert_eq!(page.items.len(), 100);
        started.elapsed()
    };
    assert!(
        small_elapsed < std::time::Duration::from_secs(1),
        "small dataset query took too long: {small_elapsed:?}"
    );
    assert!(
        large_elapsed < std::time::Duration::from_secs(1),
        "large dataset query took too long despite the index: {large_elapsed:?}"
    );

    let _ = Utc::now();
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn dataset_sizes_are_distinctly_small_and_high_volume() {
        assert!(SMALL_DATASET_ROWS < 100);
        assert!(LARGE_DATASET_ROWS >= 5_000);
        assert!(LARGE_DATASET_ROWS > SMALL_DATASET_ROWS * 50);
    }
}
