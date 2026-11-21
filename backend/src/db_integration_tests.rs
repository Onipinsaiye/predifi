//! Integration tests for the database layer using testcontainers-rs.
//!
//! Each test spins up a throwaway Postgres container, runs all migrations,
//! and exercises the real SQL queries — no pre-configured database required.
//!
//! # Resource lifecycle
//!
//! `setup()` returns both the connection pool **and** the container handle.
//! Tests must bind the container to a named variable (not `_`) so that it
//! stays alive for the entire test body.  At the end of each test the
//! container is dropped, which stops the Docker container and releases the
//! ephemeral port.  The pool is closed explicitly with `pool.close().await`
//! before the container is dropped so that all in-flight connections are
//! cleanly terminated first.

#[cfg(test)]
mod tests {
    use sqlx::{postgres::PgPoolOptions, PgPool, Row};
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    /// Boot a Postgres container, run all migrations, and return the pool
    /// together with the container handle.
    ///
    /// # Important
    ///
    /// The caller **must** keep the returned `ContainerAsync` bound to a named
    /// variable for the full duration of the test.  Binding it to `_` would
    /// drop it immediately, stopping the container before any queries run.
    ///
    /// Always close the pool before dropping the container:
    ///
    /// ```rust,ignore
    /// let (pool, container) = setup().await;
    /// // … run queries …
    /// pool.close().await;
    /// drop(container); // container stops here
    /// ```
    async fn setup() -> (PgPool, testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default()
            .start()
            .await
            .expect("postgres container");
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("postgres port");
        let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .expect("connect to postgres");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");

        (pool, container)
    }

    #[tokio::test]
    #[ignore = "Requires Docker container for Postgres"]
    async fn migrations_run_cleanly() {
        let (pool, container) = setup().await;
        // If setup() completes without panic the migrations are valid.
        pool.close().await;
        drop(container);
    }

    #[tokio::test]
    #[ignore = "Requires Docker container for Postgres"]
    async fn insert_and_query_pool() {
        let (pool, container) = setup().await;

        sqlx::query(
            "INSERT INTO pools (metadata_url, start_time, end_time) \
             VALUES ($1, NOW(), NOW() + INTERVAL '1 day')",
        )
        .bind("https://example.com/pool/1")
        .execute(&pool)
        .await
        .expect("insert pool");

        let count: i64 = sqlx::query("SELECT COUNT(*) FROM pools")
            .fetch_one(&pool)
            .await
            .expect("count pools")
            .get(0);

        assert_eq!(count, 1);

        pool.close().await;
        drop(container);
    }

    #[tokio::test]
    #[ignore = "Requires Docker container for Postgres"]
    async fn insert_and_query_prediction() {
        let (pool, container) = setup().await;

        let pool_id: i64 = sqlx::query(
            "INSERT INTO pools (metadata_url, start_time, end_time) \
             VALUES ($1, NOW(), NOW() + INTERVAL '1 day') RETURNING id",
        )
        .bind("https://example.com/pool/2")
        .fetch_one(&pool)
        .await
        .expect("insert pool")
        .get(0);

        sqlx::query(
            "INSERT INTO predictions (pool_id, user_address, outcome, amount) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(pool_id)
        .bind("GABC1234")
        .bind(1_i32)
        .bind(500_i64)
        .execute(&pool)
        .await
        .expect("insert prediction");

        let count: i64 = sqlx::query("SELECT COUNT(*) FROM predictions WHERE user_address = $1")
            .bind("GABC1234")
            .fetch_one(&pool)
            .await
            .expect("count predictions")
            .get(0);

        assert_eq!(count, 1);

        pool.close().await;
        drop(container);
    }

    /// The pool-scoped leaderboard query (`/pools/:id/leaderboard`) must use
    /// an index on `predictions.pool_id`, not a sequential scan, once the
    /// table holds enough rows to make the difference measurable (#1733).
    ///
    /// Seeds many pools with many predictions each so that filtering to one
    /// `pool_id` is highly selective (~0.3%), which is well within the range
    /// where Postgres' planner should prefer `idx_predictions_pool_id` (or
    /// one of the other `pool_id`-prefixed indexes from later migrations)
    /// over scanning the whole table. If that index is ever dropped, this
    /// selectivity gap means the planner falls back to a sequential scan and
    /// the assertion below fails.
    #[tokio::test]
    #[ignore = "Requires Docker container for Postgres"]
    async fn leaderboard_query_uses_index_not_seq_scan_on_predictions() {
        let (pool, container) = setup().await;

        const POOL_COUNT: i64 = 300;
        const PREDICTIONS_PER_POOL: i64 = 300;

        sqlx::query(
            r#"
            INSERT INTO pools (pool_id, name, category, total_stake, end_time, state, creator, token)
            SELECT g, 'Pool ' || g, 'Test', 0, NOW() + INTERVAL '1 day', 'active', 'GCREATOR', 'XLM'
            FROM generate_series(1, $1) AS g
            "#,
        )
        .bind(POOL_COUNT)
        .execute(&pool)
        .await
        .expect("seed pools");

        sqlx::query(
            r#"
            INSERT INTO predictions (pool_id, user_address, outcome, amount)
            SELECT g, 'GUSER' || ((g * 100000) + s)::text, (s % 2), 100
            FROM generate_series(1, $1) AS g, generate_series(1, $2) AS s
            "#,
        )
        .bind(POOL_COUNT)
        .bind(PREDICTIONS_PER_POOL)
        .execute(&pool)
        .await
        .expect("seed predictions");

        // Fresh statistics are essential: without them the planner may still
        // guess a sequential scan for a table it thinks is tiny/default-sized.
        sqlx::query("ANALYZE predictions")
            .execute(&pool)
            .await
            .expect("analyze predictions");
        sqlx::query("ANALYZE pools")
            .execute(&pool)
            .await
            .expect("analyze pools");

        let plan = crate::db::explain_leaderboard_plan(&pool, "volume", "all", Some(1), 20, 0)
            .await
            .expect("explain leaderboard query");

        assert!(
            !plan.to_lowercase().contains("seq scan on predictions"),
            "leaderboard query for a single pool_id must use an index on predictions, \
             not a sequential scan; got plan:\n{plan}"
        );

        pool.close().await;
        drop(container);
    }

     #[tokio::test]
 #[ignore = "Requires Docker container for Postgres"]
 async fn retention_cleanup_deletes_expired_private_records_and_keeps_public_history() {
 let (pool, container) = setup().await;

 // Synthetic private user record linked to a prediction.
 sqlx::query(
 "INSERT INTO pools (pool_id, name, end_time) VALUES (950, 'retention', NOW() - INTERVAL '400 days')",
 )
 .execute(&pool)
 .await
 .expect("insert pool");

 sqlx::query(
 "INSERT INTO predictions (pool_id, user_address, outcome, amount, created_at) \
 VALUES (950, 'GOLD', 0, 10, NOW() - INTERVAL '400 days')",
 )
 .execute(&pool)
 .await
 .expect("insert prediction");

 // Run the retention job (expected to purge private, expired personal data).
 crate::db::run_retention_cleanup(&pool)
 .await
 .expect("retention cleanup");

 // Private personal-linked row past retention is removed.
 let remaining: i64 =
 sqlx::query("SELECT COUNT(*) FROM predictions WHERE user_address = 'GOLD'")
 .fetch_one(&pool)
 .await
 .expect("count private")
 .get(0);
 assert_eq!(remaining, 0, "expired private user record must be deleted");

 // Public, chain-derived pool history remains and stays reproducible.
 let pool_count: i64 = sqlx::query("SELECT COUNT(*) FROM pools WHERE pool_id = 950")
 .fetch_one(&pool)
 .await
 .expect("count pool")
 .get(0);
 assert_eq!(pool_count, 1, "public chain-derived history must be retained");

 pool.close().await;
 drop(container);
 }

 #[tokio::test]
 #[ignore = "Requires Docker container for Postgres"]
 async fn pool_status_defaults_to_open() {
        let (pool, container) = setup().await;

        sqlx::query(
            "INSERT INTO pools (metadata_url, start_time, end_time) \
             VALUES ($1, NOW(), NOW() + INTERVAL '1 day')",
        )
        .bind("https://example.com/pool/3")
        .execute(&pool)
        .await
        .expect("insert pool");

        let status: String = sqlx::query("SELECT status FROM pools ORDER BY id DESC LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("fetch status")
            .get(0);

        assert_eq!(status, "Open");

        pool.close().await;
        drop(container);
    }

    // ------------------------------------------------------------------
    // #261 — Portfolio exposure by market and outcome
    // ------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "Requires Docker container for Postgres"]
    async fn portfolio_exposure_aggregates_repeated_positions_and_flags_missing_prices() {
        let (pool, container) = setup().await;

        // Open market with a price, closed market, and a market with no price.
        sqlx::query(
            "INSERT INTO pools (pool_id, name, end_time, state) VALUES \
             (801, 'open', NOW() + INTERVAL '1 day', 'active'), \
             (802, 'closed', NOW() - INTERVAL '1 day', 'closed'), \
             (803, 'unpriced', NOW() + INTERVAL '1 day', 'active')",
        )
        .execute(&pool)
        .await
        .expect("insert pools");

        // Repeated positions in the same market/outcome must be summed.
        for (pool_id, outcome, amount) in [
            (801_i64, 0_i32, 100_i64),
            (801, 0, 50),
            (801, 1, 25),
            (802, 0, 40),
            (803, 0, 10),
        ] {
            sqlx::query(
                "INSERT INTO predictions (pool_id, user_address, outcome, amount) \
                 VALUES ($1, 'GEXPOSURE', $2, $3)",
            )
            .bind(pool_id)
            .bind(outcome)
            .bind(amount)
            .execute(&pool)
            .await
            .expect("insert prediction");
        }

        let rows = crate::db::get_portfolio_exposure(&pool, "GEXPOSURE")
            .await
            .expect("portfolio exposure");

        let open_outcome_0 = rows
            .iter()
            .find(|r| r.pool_id == 801 && r.outcome == 0)
            .expect("aggregated open outcome 0");
        assert_eq!(open_outcome_0.amount, 150, "repeated positions are summed");

        let open_outcome_1 = rows
            .iter()
            .find(|r| r.pool_id == 801 && r.outcome == 1)
            .expect("aggregated open outcome 1");
        assert_eq!(open_outcome_1.amount, 25);

        // Shares are expressed against the whole indexed portfolio.
        let total: i64 = rows.iter().map(|r| r.amount).sum();
        assert_eq!(total, 225);
        assert!((open_outcome_0.share - 150.0 / 225.0).abs() < 1e-9);

        // Closed markets are reported but marked as not open.
        let closed = rows
            .iter()
            .find(|r| r.pool_id == 802)
            .expect("closed market position");
        assert!(!closed.is_open, "closed market must be flagged");

        // Missing price data yields an estimate of `None`, never a fake zero.
        let unpriced = rows
            .iter()
            .find(|r| r.pool_id == 803)
            .expect("unpriced market position");
        assert!(unpriced.estimated_value.is_none());

        pool.close().await;
        drop(container);
    }

    // ------------------------------------------------------------------
    // #262 — Claimable-rewards work queue
    // ------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "Requires Docker container for Postgres"]
    async fn claimable_rewards_queue_mixes_eligibility_and_survives_partial_failures() {
        let (pool, container) = setup().await;

        sqlx::query(
            "INSERT INTO pools (pool_id, name, end_time, state) VALUES \
             (811, 'eligible', NOW() - INTERVAL '1 day', 'closed'), \
             (812, 'ineligible', NOW() + INTERVAL '1 day', 'active')",
        )
        .execute(&pool)
        .await
        .expect("insert pools");

        for pool_id in [811_i64, 812_i64] {
            sqlx::query(
                "INSERT INTO predictions (pool_id, user_address, outcome, amount) \
                 VALUES ($1, 'GCLAIM', 0, 100)",
            )
            .bind(pool_id)
            .execute(&pool)
            .await
            .expect("insert prediction");
        }

        let queue = crate::db::get_claimable_rewards(&pool, "GCLAIM", 10)
            .await
            .expect("claimable rewards");
        assert!(queue.iter().any(|c| c.pool_id == 811 && c.eligible));
        assert!(queue.iter().any(|c| c.pool_id == 812 && !c.eligible));

        // Bounded selection: requesting one row returns at most one.
        let bounded = crate::db::get_claimable_rewards(&pool, "GCLAIM", 1)
            .await
            .expect("bounded claimable rewards");
        assert_eq!(bounded.len(), 1);

        // A partial batch failure must not mark the failed claim complete.
        let batch = crate::db::simulate_claim_batch(&pool, "GCLAIM", &[811, 812])
            .await
            .expect("simulate claim batch");
        assert!(batch.iter().any(|r| r.pool_id == 811 && r.simulated));
        assert!(batch.iter().any(|r| r.pool_id == 812 && !r.simulated));

        let status: String = sqlx::query(
            "SELECT status FROM claim_queue WHERE user_address = 'GCLAIM' AND pool_id = 812",
        )
        .fetch_one(&pool)
        .await
        .expect("failed claim status")
        .get(0);
        assert_eq!(status, "pending", "simulation failure must not complete a claim");

        // Status refresh after confirmation flips the claim to confirmed.
        crate::db::mark_claim_confirmed(&pool, "GCLAIM", 811, "tx-claim-811")
            .await
            .expect("mark claim confirmed");
        let confirmed: String = sqlx::query(
            "SELECT status FROM claim_queue WHERE user_address = 'GCLAIM' AND pool_id = 811",
        )
        .fetch_one(&pool)
        .await
        .expect("confirmed claim status")
        .get(0);
        assert_eq!(confirmed, "confirmed");

        pool.close().await;
        drop(container);
    }

    // ------------------------------------------------------------------
    // #263 — Claim reconciliation
    // ------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "Requires Docker container for Postgres"]
    async fn claim_reconciliation_is_idempotent_under_retries_and_reordering() {
        let (pool, container) = setup().await;

        sqlx::query(
            "INSERT INTO pools (pool_id, name, end_time, state) VALUES \
             (821, 'settled', NOW() - INTERVAL '1 day', 'closed')",
        )
        .execute(&pool)
        .await
        .expect("insert pool");

        sqlx::query(
            "INSERT INTO predictions (pool_id, user_address, outcome, amount) \
             VALUES (821, 'GRECON', 0, 100)",
        )
        .execute(&pool)
        .await
        .expect("insert prediction");

        // Claim event arrives before the settlement event is indexed.
        crate::db::record_claim_event(&pool, "GRECON", 821, 0, "tx-recon-1", 100)
            .await
            .expect("record claim event");

        let pending = crate::db::get_claim_reconciliation(&pool, "GRECON", "tx-recon-1")
            .await
            .expect("reconciliation status");
        assert_eq!(pending.status, "pending", "delayed indexing must be surfaced");

        // Settlement event is indexed afterwards (reordering).
        crate::db::record_settlement_event(&pool, 821, 0, "settle-821")
            .await
            .expect("record settlement event");

        // Retrying the same claim event must not duplicate ledger credits.
        for _ in 0..3 {
            crate::db::record_claim_event(&pool, "GRECON", 821, 0, "tx-recon-1", 100)
                .await
                .expect("retry claim event");
        }

        let reconciled = crate::db::get_claim_reconciliation(&pool, "GRECON", "tx-recon-1")
            .await
            .expect("reconciled status");
        assert_eq!(reconciled.status, "reconciled");
        assert_eq!(reconciled.payout, 100);
        assert_eq!(reconciled.source_event, "settle-821");

        let credits: i64 = sqlx::query(
            "SELECT COUNT(*) FROM claim_ledger WHERE user_address = 'GRECON' AND tx_hash = 'tx-recon-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("count ledger credits")
        .get(0);
        assert_eq!(credits, 1, "retries must not duplicate ledger credits");

        pool.close().await;
        drop(container);
    }

    // ------------------------------------------------------------------
    // #264 — CSV export
    // ------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "Requires Docker container for Postgres"]
    async fn portfolio_csv_export_preserves_decimals_escapes_formulas_and_orders_deterministically(
    ) {
        let (pool, container) = setup().await;

        sqlx::query(
            "INSERT INTO pools (pool_id, name, end_time, state) VALUES \
             (831, 'export', NOW() - INTERVAL '1 day', 'closed')",
        )
        .execute(&pool)
        .await
        .expect("insert pool");

        // Untrusted text that would be interpreted as a formula by spreadsheet apps.
        sqlx::query(
            "INSERT INTO predictions (pool_id, user_address, outcome, amount) \
             VALUES (831, '=cmd|'' /C calc''!A0', 0, 100)",
        )
        .execute(&pool)
        .await
        .expect("insert prediction");

        let export = crate::db::export_portfolio_activity_csv(&pool, "=cmd|'' /C calc''!A0", 100)
            .await
            .expect("csv export");

        assert_eq!(export.schema_version, "1");
        assert_eq!(export.network, "testnet");

        // Exact decimal preservation: no float rounding of the amount column.
        assert!(
            export.csv.contains("100"),
            "amount must be preserved exactly; got:\n{}",
            export.csv
        );

        // Formula injection is neutralised with a leading apostrophe.
        assert!(
            export.csv.contains("'=cmd"),
            "untrusted cell must be escaped; got:\n{}",
            export.csv
        );

        // Deterministic ordering across repeated exports.
        let again = crate::db::export_portfolio_activity_csv(&pool, "=cmd|'' /C calc''!A0", 100)
            .await
            .expect("csv export again");
        assert_eq!(export.csv, again.csv, "export ordering must be deterministic");

        pool.close().await;
        drop(container);
    }

    #[tokio::test]
    #[ignore = "Requires Docker container for Postgres"]
    async fn market_prediction_cursor_is_stable_across_duplicate_times_and_inserts() {
        let (pool, container) = setup().await;
        sqlx::query("INSERT INTO pools (pool_id, name, end_time) VALUES (901, 'cursor', NOW() + INTERVAL '1 day')")
            .execute(&pool).await.expect("insert pool");
        let timestamp = chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap();
        for address in ["G1", "G2", "G3"] {
            sqlx::query("INSERT INTO predictions (pool_id, user_address, outcome, amount, created_at) VALUES (901, $1, 0, 1, $2)")
                .bind(address).bind(timestamp).execute(&pool).await.expect("insert prediction");
        }

        let first = crate::db::get_market_predictions(&pool, 901, None, 2)
            .await
            .expect("first page");
        assert_eq!(first.len(), 3, "fetches one lookahead row");
        let cursor_row = &first[1];
        let cursor = (cursor_row.created_at, cursor_row.id);

        // An insertion newer than the cursor cannot move a later page boundary.
        sqlx::query("INSERT INTO predictions (pool_id, user_address, outcome, amount, created_at) VALUES (901, 'G4', 0, 1, $1)")
            .bind(timestamp).execute(&pool).await.expect("insert concurrent prediction");
        let second = crate::db::get_market_predictions(&pool, 901, Some(cursor), 2)
            .await
            .expect("second page");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].id, first[2].id);
        let exhausted = crate::db::get_market_predictions(
            &pool,
            901,
            Some((second[0].created_at, second[0].id)),
            2,
        )
        .await
        .expect("exhausted page");
        assert!(exhausted.is_empty());

        pool.close().await;
        drop(container);
    }
}
