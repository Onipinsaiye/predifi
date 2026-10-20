//! Integration tests for atomic listener checkpoint persistence and restart recovery.
//!
//! Acceptance criteria:
//! - Commit event rows and the next checkpoint in one database transaction.
//! - Define startup behavior when a checkpoint is missing or malformed.
//! - Inject failures before and after commit and prove restart resumes without event loss or duplicates.

use predifi_backend::db::{
    load_checkpoint, resolve_startup_checkpoint, save_checkpoint, CheckpointStatus,
};
use predifi_backend::redis_cache::RedisCache;
use predifi_backend::worker::stellar_listener::{
    commit_event_batch_atomic, FailureInjector, StellarEvent,
};
use predifi_backend::ws::EventBus;
use sqlx::{postgres::PgPoolOptions, PgPool};

async fn get_test_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL")
        .or_else(|_| std::env::var("PREDIFI_DATABASE_URL"))
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/predifi".into());

    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_millis(500))
        .connect(&url)
        .await
        .ok()
}

async fn setup_clean_tables(pool: &PgPool) {
    let _ = sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS listener_checkpoints (
            id VARCHAR(128) PRIMARY KEY,
            last_ledger BIGINT NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        );
        CREATE TABLE IF NOT EXISTS app_state (
            key VARCHAR(128) PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        );
        CREATE TABLE IF NOT EXISTS pools (
            pool_id BIGINT PRIMARY KEY,
            name TEXT NOT NULL,
            category TEXT NOT NULL,
            total_stake BIGINT NOT NULL DEFAULT 0,
            end_time TIMESTAMPTZ NOT NULL,
            state TEXT NOT NULL,
            creator VARCHAR(56) NOT NULL,
            token VARCHAR(56) NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            result TEXT,
            resolved_at TIMESTAMPTZ
        );
        CREATE TABLE IF NOT EXISTS predictions (
            id BIGSERIAL PRIMARY KEY,
            pool_id BIGINT NOT NULL REFERENCES pools (pool_id),
            user_address VARCHAR(56) NOT NULL,
            outcome INTEGER NOT NULL,
            amount BIGINT NOT NULL DEFAULT 0,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        );
        CREATE TABLE IF NOT EXISTS creator_stats (
            creator VARCHAR(56) PRIMARY KEY,
            pools_created INTEGER NOT NULL DEFAULT 0,
            pools_reward_eligible INTEGER NOT NULL DEFAULT 0,
            total_volume BIGINT NOT NULL DEFAULT 0,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        );
        CREATE TABLE IF NOT EXISTS referrals (
            id BIGSERIAL PRIMARY KEY,
            referrer VARCHAR(56) NOT NULL,
            user_address VARCHAR(56) NOT NULL,
            pool_id BIGINT NOT NULL REFERENCES pools (pool_id),
            amount BIGINT NOT NULL DEFAULT 0,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        );
        "#,
    )
    .execute(pool)
    .await;
}

#[tokio::test]
async fn test_startup_behavior_missing_checkpoint() {
    let Some(pool) = get_test_pool().await else {
        println!("Skipping Postgres test: database unavailable");
        return;
    };
    setup_clean_tables(&pool).await;

    let test_listener_id = "test_listener_missing_checkpoint";
    let _ = sqlx::query("DELETE FROM listener_checkpoints WHERE id = $1")
        .bind(test_listener_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM app_state WHERE key = $1")
        .bind(test_listener_id)
        .execute(&pool)
        .await;

    let status = load_checkpoint(&pool, test_listener_id).await;
    assert_eq!(status, CheckpointStatus::Missing);

    let resolved = resolve_startup_checkpoint(&status, test_listener_id, 1);
    assert_eq!(resolved, 1, "missing checkpoint must safely default to initial ledger");
}

#[tokio::test]
async fn test_startup_behavior_malformed_checkpoint() {
    let Some(pool) = get_test_pool().await else {
        println!("Skipping Postgres test: database unavailable");
        return;
    };
    setup_clean_tables(&pool).await;

    let test_listener_id = "test_listener_malformed_checkpoint";
    let _ = sqlx::query("DELETE FROM listener_checkpoints WHERE id = $1")
        .bind(test_listener_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("INSERT INTO app_state (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value")
        .bind(test_listener_id)
        .bind("not-a-number")
        .execute(&pool)
        .await;

    let status = load_checkpoint(&pool, test_listener_id).await;
    assert!(matches!(status, CheckpointStatus::Malformed(_)));

    let resolved = resolve_startup_checkpoint(&status, test_listener_id, 1);
    assert_eq!(
        resolved, 1,
        "malformed checkpoint must be recovered by falling back to default ledger"
    );
}

#[tokio::test]
async fn test_failure_injection_before_commit_rolls_back_and_restart_prevents_loss() {
    let Some(pool) = get_test_pool().await else {
        println!("Skipping Postgres test: database unavailable");
        return;
    };
    setup_clean_tables(&pool).await;

    let test_listener_id = "stellar_listener_latest_ledger";
    save_checkpoint(&pool, test_listener_id, 100)
        .await
        .expect("save baseline checkpoint");

    let pool_id = 999_001u64;
    let pool_created_event = StellarEvent {
        event_type: "contract".into(),
        ledger: 101,
        contract_id: Some("C_PREDIFI".into()),
        id: "evt-pool-999001".into(),
        topics: Some(vec!["pool_created".into()]),
        data: Some(serde_json::json!({
            "pool_id": pool_id,
            "creator": "GCREATOR_INJECT_1",
            "end_time": 1_800_000_000u64,
            "token": "GTOKEN",
            "category": "Tech",
            "metadata_url": "https://predifi.example/pool-1"
        })),
    };

    let redis = RedisCache::disabled();
    let bus = EventBus::new();

    // 1. Inject failure BEFORE commit: transaction must roll back
    let res = commit_event_batch_atomic(
        &pool,
        &redis,
        &bus,
        &[pool_created_event.clone()],
        102,
        10,
        &FailureInjector::fail_before_commit(),
    )
    .await;
    assert!(res.is_err(), "injected failure before commit must return an error");

    // 2. Verify neither the pool nor the checkpoint were committed
    let pool_row = sqlx::query("SELECT pool_id FROM pools WHERE pool_id = $1")
        .bind(pool_id as i64)
        .fetch_optional(&pool)
        .await
        .unwrap();
    assert!(pool_row.is_none(), "rolled back event must not exist in pools table");

    let status = load_checkpoint(&pool, test_listener_id).await;
    assert_eq!(status, CheckpointStatus::Valid(100), "checkpoint must remain at baseline ledger");

    // 3. Simulate process restart and retry without injected failure
    let restart_cursor = resolve_startup_checkpoint(&status, test_listener_id, 1);
    assert_eq!(restart_cursor, 100);

    let retry_res = commit_event_batch_atomic(
        &pool,
        &redis,
        &bus,
        &[pool_created_event],
        102,
        10,
        &FailureInjector::none(),
    )
    .await;
    assert!(retry_res.is_ok(), "retry after crash must succeed");

    // 4. Verify durable persistence
    let committed_pool = sqlx::query("SELECT pool_id FROM pools WHERE pool_id = $1")
        .bind(pool_id as i64)
        .fetch_optional(&pool)
        .await
        .unwrap();
    assert!(committed_pool.is_some(), "replayed event must be durably committed");

    let new_status = load_checkpoint(&pool, test_listener_id).await;
    assert_eq!(new_status, CheckpointStatus::Valid(102), "checkpoint must advance to 102");
}

#[tokio::test]
async fn test_failure_injection_after_commit_restart_prevents_duplicate_processing() {
    let Some(pool) = get_test_pool().await else {
        println!("Skipping Postgres test: database unavailable");
        return;
    };
    setup_clean_tables(&pool).await;

    let test_listener_id = "stellar_listener_latest_ledger";
    save_checkpoint(&pool, test_listener_id, 200)
        .await
        .expect("save baseline checkpoint");

    let pool_id = 999_002u64;
    let pool_created_event = StellarEvent {
        event_type: "contract".into(),
        ledger: 201,
        contract_id: Some("C_PREDIFI".into()),
        id: "evt-pool-999002".into(),
        topics: Some(vec!["pool_created".into()]),
        data: Some(serde_json::json!({
            "pool_id": pool_id,
            "creator": "GCREATOR_INJECT_2",
            "end_time": 1_800_000_000u64,
            "token": "GTOKEN",
            "category": "Crypto",
            "metadata_url": "https://predifi.example/pool-2"
        })),
    };

    let redis = RedisCache::disabled();
    let bus = EventBus::new();

    // 1. Inject failure AFTER commit: database transaction committed, but crash simulated immediately after
    let res = commit_event_batch_atomic(
        &pool,
        &redis,
        &bus,
        &[pool_created_event.clone()],
        202,
        10,
        &FailureInjector::fail_after_commit(),
    )
    .await;
    assert!(res.is_err(), "injected failure after commit must report error");

    // 2. Restart inspection: checkpoint was already committed along with the events
    let status = load_checkpoint(&pool, test_listener_id).await;
    assert_eq!(
        status,
        CheckpointStatus::Valid(202),
        "post-commit crash must still have durable checkpoint"
    );

    let resume_cursor = resolve_startup_checkpoint(&status, test_listener_id, 1);
    assert_eq!(resume_cursor, 202, "restart resumes past ledger 201, avoiding duplicate execution");

    // 3. Confirm exact single row in pools table (no duplicate)
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pools WHERE pool_id = $1")
        .bind(pool_id as i64)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 1, "exactly one pool record must exist");
}
