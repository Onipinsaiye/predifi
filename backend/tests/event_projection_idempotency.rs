//! Worker integration test for idempotent on-chain event projection (#45).
//!
//! Replays the same ledger range — whole, repeatedly, and as overlapping
//! listener windows — and asserts the database state and notification count
//! are identical to a single projection.
//!
//! Requires Postgres (`DATABASE_URL` / `PREDIFI_DATABASE_URL`); skipped when
//! no database is reachable.

use predifi_backend::db::{
    project_events, ContractEvent, EventKey, PoolCreatedEvent, PredictionPlacedEvent,
    ProjectedEvent, ProjectionSummary,
};
use sqlx::{postgres::PgPoolOptions, PgPool};

const NETWORK: &str = "event-projection-test";
const CONTRACT: &str = "CPREDIFIEVENTPROJECTIONTEST";
const POOL_ID: u64 = 4_500_001;
const CREATOR: &str = "GCREATOREVENTPROJECTION";

const BASE_SCHEMA: &str = r#"
        CREATE TABLE IF NOT EXISTS pools (
            pool_id BIGINT PRIMARY KEY,
            name TEXT NOT NULL,
            category TEXT NOT NULL DEFAULT '',
            total_stake BIGINT NOT NULL DEFAULT 0,
            end_time TIMESTAMPTZ NOT NULL,
            state VARCHAR(20) NOT NULL DEFAULT 'active',
            creator TEXT NOT NULL DEFAULT '',
            token TEXT NOT NULL DEFAULT '',
            result TEXT,
            resolved_at TIMESTAMPTZ,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
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
        CREATE TABLE IF NOT EXISTS notification_outbox (
            id BIGSERIAL PRIMARY KEY,
            user_address TEXT NOT NULL,
            notif_type TEXT NOT NULL,
            title TEXT NOT NULL,
            message TEXT NOT NULL,
            pool_id BIGINT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            attempts INTEGER NOT NULL DEFAULT 0,
            max_attempts INTEGER NOT NULL DEFAULT 5,
            next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            last_error TEXT,
            claimed_at TIMESTAMPTZ,
            claimed_by TEXT,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        );
"#;

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

/// Create the tables the projector writes to, then apply migration 016.
/// Runs as one simple-query batch (one implicit transaction) under an advisory
/// lock so parallel tests do not race on `CREATE ... IF NOT EXISTS`.
async fn setup_schema(pool: &PgPool) {
    let script = format!(
        "SELECT pg_advisory_xact_lock(45045);\n{BASE_SCHEMA}\n{}",
        include_str!("../migrations/016_create_projected_contract_events.sql")
    );
    sqlx::raw_sql(&script)
        .execute(pool)
        .await
        .expect("create schema and apply migration 016");
}

/// Remove rows left by a previous run for this test's pool, creator and network.
async fn reset(pool: &PgPool, pool_id: u64, creator: &str, network: &str) {
    for sql in [
        "DELETE FROM notification_outbox WHERE pool_id = $1",
        "DELETE FROM predictions WHERE pool_id = $1",
        "DELETE FROM pools WHERE pool_id = $1",
    ] {
        sqlx::query(sql)
            .bind(pool_id as i64)
            .execute(pool)
            .await
            .unwrap();
    }
    sqlx::query("DELETE FROM creator_stats WHERE creator = $1")
        .bind(creator)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM projected_contract_events WHERE network = $1")
        .bind(network)
        .execute(pool)
        .await
        .unwrap();
}

fn key(ledger: u64, tx: &str, event_index: u32) -> EventKey {
    EventKey {
        network: NETWORK.into(),
        contract_id: CONTRACT.into(),
        ledger,
        tx_hash: tx.into(),
        event_index,
    }
}

fn prediction(user: &str, outcome: i32, amount: i64) -> ProjectedEvent {
    ProjectedEvent::PredictionPlaced(PredictionPlacedEvent {
        pool_id: POOL_ID,
        user_address: user.into(),
        outcome,
        amount,
    })
}

/// Contract events for ledgers 100..=103, in on-chain order.
fn ledger_range() -> Vec<ContractEvent> {
    let created = ProjectedEvent::PoolCreated(PoolCreatedEvent {
        pool_id: POOL_ID,
        creator: CREATOR.into(),
        end_time: 4_102_444_800,
        token: "CTOKEN".into(),
        category: "sports".into(),
        description: "Idempotency test pool".into(),
    });
    vec![
        ContractEvent {
            key: key(100, "tx-a", 0),
            event: created,
        },
        ContractEvent {
            key: key(101, "tx-b", 0),
            event: prediction("GALICE", 0, 400),
        },
        // Identical payload in another transaction is a distinct event (a top-up).
        ContractEvent {
            key: key(101, "tx-c", 0),
            event: prediction("GALICE", 0, 400),
        },
        // Two events in the same transaction differ only by position.
        ContractEvent {
            key: key(102, "tx-d", 0),
            event: prediction("GBOB", 1, 250),
        },
        ContractEvent {
            key: key(102, "tx-d", 1),
            event: prediction("GBOB", 1, 250),
        },
        ContractEvent {
            key: key(103, "tx-e", 0),
            event: ProjectedEvent::PoolResolved {
                pool_id: POOL_ID,
                winning_outcome: 0,
            },
        },
    ]
}

fn window(events: &[ContractEvent], from: u64, to: u64) -> Vec<ContractEvent> {
    events
        .iter()
        .filter(|e| (from..=to).contains(&e.key.ledger))
        .cloned()
        .collect()
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    pool: (i64, String, Option<String>),
    predictions: Vec<(String, i32, i64)>,
    pools_created_by_creator: i32,
    notifications: Vec<(String, String)>,
    projected_keys: i64,
}

async fn snapshot(pool: &PgPool) -> Snapshot {
    let pool_id = POOL_ID as i64;
    Snapshot {
        pool: sqlx::query_as(
            "SELECT total_stake, state::text, result FROM pools WHERE pool_id = $1",
        )
        .bind(pool_id)
        .fetch_one(pool)
        .await
        .unwrap(),
        predictions: sqlx::query_as(
            "SELECT user_address::text, outcome, amount FROM predictions
             WHERE pool_id = $1 ORDER BY user_address, outcome, amount, id",
        )
        .bind(pool_id)
        .fetch_all(pool)
        .await
        .unwrap(),
        pools_created_by_creator: sqlx::query_scalar(
            "SELECT pools_created FROM creator_stats WHERE creator = $1",
        )
        .bind(CREATOR)
        .fetch_one(pool)
        .await
        .unwrap(),
        notifications: sqlx::query_as(
            "SELECT user_address, notif_type FROM notification_outbox
             WHERE pool_id = $1 ORDER BY user_address, notif_type",
        )
        .bind(pool_id)
        .fetch_all(pool)
        .await
        .unwrap(),
        projected_keys: sqlx::query_scalar(
            "SELECT COUNT(*) FROM projected_contract_events WHERE network = $1",
        )
        .bind(NETWORK)
        .fetch_one(pool)
        .await
        .unwrap(),
    }
}

#[tokio::test]
async fn replaying_ledger_range_yields_identical_state() {
    let Some(pool) = get_test_pool().await else {
        println!("Skipping Postgres test: database unavailable");
        return;
    };
    setup_schema(&pool).await;
    reset(&pool, POOL_ID, CREATOR, NETWORK).await;
    let events = ledger_range();

    let first = project_events(&pool, &events).await.unwrap();
    assert_eq!(
        first,
        ProjectionSummary {
            applied: 6,
            duplicates: 0
        }
    );
    let expected = snapshot(&pool).await;
    assert_eq!(expected.pool, (1_300, "settled".into(), Some("0".into())));
    assert_eq!(expected.predictions.len(), 4);
    assert_eq!(expected.pools_created_by_creator, 1);
    assert_eq!(expected.notifications.len(), 2, "one per predictor");
    assert_eq!(expected.projected_keys, 6);

    // Full retries of the same range.
    for _ in 0..3 {
        let replay = project_events(&pool, &events).await.unwrap();
        assert_eq!(
            replay,
            ProjectionSummary {
                applied: 0,
                duplicates: 6
            }
        );
        assert_eq!(snapshot(&pool).await, expected);
    }

    // Overlapping listener windows over the same range.
    for (from, to) in [(100, 101), (101, 102), (102, 103), (100, 103)] {
        let replay = project_events(&pool, &window(&events, from, to))
            .await
            .unwrap();
        assert_eq!(
            replay.applied, 0,
            "window {from}..={to} re-applied an event"
        );
        assert_eq!(snapshot(&pool).await, expected);
    }
}

#[tokio::test]
async fn duplicate_events_within_one_batch_are_projected_once() {
    let Some(pool) = get_test_pool().await else {
        println!("Skipping Postgres test: database unavailable");
        return;
    };
    // Uses its own pool id so it can run in parallel with the replay test.
    let pool_id = POOL_ID + 1;
    let network = format!("{NETWORK}-batch");
    setup_schema(&pool).await;
    reset(&pool, pool_id, "GBATCHCREATOR", &network).await;

    let k = |ledger, index| EventKey {
        network: network.clone(),
        ..key(ledger, "tx-z", index)
    };
    let created = ContractEvent {
        key: k(200, 0),
        event: ProjectedEvent::PoolCreated(PoolCreatedEvent {
            pool_id,
            creator: "GBATCHCREATOR".into(),
            end_time: 4_102_444_800,
            token: "CTOKEN".into(),
            category: "sports".into(),
            description: "Batch duplicate pool".into(),
        }),
    };
    let predicted = ContractEvent {
        key: k(200, 1),
        event: ProjectedEvent::PredictionPlaced(PredictionPlacedEvent {
            pool_id,
            user_address: "GCAROL".into(),
            outcome: 1,
            amount: 75,
        }),
    };
    let batch = vec![
        created.clone(),
        predicted.clone(),
        predicted.clone(),
        created,
    ];

    let summary = project_events(&pool, &batch).await.unwrap();
    assert_eq!(
        summary,
        ProjectionSummary {
            applied: 2,
            duplicates: 2
        }
    );

    let (stake, rows): (i64, i64) = sqlx::query_as(
        "SELECT p.total_stake, (SELECT COUNT(*) FROM predictions WHERE pool_id = p.pool_id)
         FROM pools p WHERE p.pool_id = $1",
    )
    .bind(pool_id as i64)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((stake, rows), (75, 1));
}
