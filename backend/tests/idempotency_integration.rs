//! Postgres integration tests for idempotency keys on mutations (#48).
//!
//! Requires `DATABASE_URL`; each test is skipped when it is unset. Requests go
//! through a real Axum router guarded by the idempotency middleware with the
//! Postgres store, so concurrent duplicates race on the actual
//! `idempotency_keys` table. Every test uses fresh keys.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::post;
use axum::Router;
use predifi_backend::idempotency::{
    codes, Idempotency, IdempotencyPolicy, IdempotencyStore, IDEMPOTENCY_KEY_HEADER,
    IDEMPOTENT_REPLAYED_HEADER,
};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower::ServiceExt;

const SECRET: &str = "integration-secret-key-that-is-long-enough-for-hs256-42";

async fn pool() -> Option<PgPool> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL not set; skipping idempotency integration test");
        return None;
    };
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    // Tests start in parallel; serialise the DDL so concurrent
    // `CREATE TABLE IF NOT EXISTS` calls do not race.
    let migration = format!(
        "BEGIN; SELECT pg_advisory_xact_lock(4812); {} COMMIT;",
        include_str!("../migrations/012_create_idempotency_keys.sql")
    );
    sqlx::raw_sql(&migration)
        .execute(&pool)
        .await
        .expect("apply idempotency_keys migration");
    Some(pool)
}

fn bearer(wallet: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let token = predifi_backend::jwt::sign_jwt(wallet, SECRET, now).expect("sign jwt");
    format!("Bearer {token}")
}

/// A "create prediction" endpoint that counts executions and takes long
/// enough for concurrent duplicates to overlap.
fn app(pool: PgPool, policy: IdempotencyPolicy) -> (Router, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let handler = post(move |body: String| {
        let counter = counter.clone();
        async move {
            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
            tokio::time::sleep(Duration::from_millis(100)).await;
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                format!(r#"{{"status":"success","data":{{"execution":{n},"echo":{body}}}}}"#),
            )
        }
    });
    let idem = Idempotency::new(IdempotencyStore::postgres(pool), "predictions.create", SECRET, 0)
        .with_policy(policy);
    (Router::new().route("/predictions", idem.guard(handler)), calls)
}

#[derive(Debug)]
struct Reply {
    status: StatusCode,
    replayed: bool,
    body: String,
}

async fn send(app: &Router, key: &str, wallet: &str, body: &str) -> Reply {
    let request = Request::builder()
        .method("POST")
        .uri("/predictions")
        .header(IDEMPOTENCY_KEY_HEADER, key)
        .header(header::AUTHORIZATION, bearer(wallet))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let replayed = response.headers().contains_key(IDEMPOTENT_REPLAYED_HEADER);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    Reply {
        status,
        replayed,
        body: String::from_utf8(bytes.to_vec()).unwrap(),
    }
}

fn key() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[tokio::test]
async fn exact_retry_replays_the_original_response() {
    let Some(pool) = pool().await else { return };
    let (app, calls) = app(pool, IdempotencyPolicy::default());
    let key = key();
    let first = send(&app, &key, "GALICE", r#"{"pool_id":1,"amount":10}"#).await;
    let retry = send(&app, &key, "GALICE", r#"{"amount":10,"pool_id":1}"#).await;
    assert_eq!(first.status, StatusCode::OK);
    assert!(!first.replayed);
    assert_eq!(retry.status, StatusCode::OK);
    assert!(retry.replayed);
    assert_eq!(retry.body, first.body);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn same_key_different_payload_is_rejected() {
    let Some(pool) = pool().await else { return };
    let (app, calls) = app(pool, IdempotencyPolicy::default());
    let key = key();
    send(&app, &key, "GALICE", r#"{"pool_id":1,"amount":10}"#).await;
    let conflict = send(&app, &key, "GALICE", r#"{"pool_id":1,"amount":99}"#).await;
    assert_eq!(conflict.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(conflict.body.contains(codes::KEY_REUSED), "{}", conflict.body);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn key_is_scoped_to_the_authenticated_wallet() {
    let Some(pool) = pool().await else { return };
    let (app, calls) = app(pool, IdempotencyPolicy::default());
    let key = key();
    send(&app, &key, "GALICE", r#"{"pool_id":1}"#).await;
    let bob = send(&app, &key, "GBOB", r#"{"pool_id":1}"#).await;
    assert_eq!(bob.status, StatusCode::OK);
    assert!(!bob.replayed, "a wallet must never receive another wallet's response");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn concurrent_duplicates_execute_exactly_once() {
    let Some(pool) = pool().await else { return };
    let (app, calls) = app(pool, IdempotencyPolicy::default());
    let key = key();

    let tasks: Vec<_> = (0..20)
        .map(|_| {
            let app = app.clone();
            let key = key.clone();
            tokio::spawn(async move { send(&app, &key, "GALICE", r#"{"pool_id":7}"#).await })
        })
        .collect();
    let mut replies = Vec::new();
    for task in tasks {
        replies.push(task.await.unwrap());
    }

    assert_eq!(calls.load(Ordering::SeqCst), 1, "{replies:?}");
    let executed: Vec<_> = replies
        .iter()
        .filter(|r| r.status == StatusCode::OK && !r.replayed)
        .collect();
    assert_eq!(executed.len(), 1, "{replies:?}");
    for reply in &replies {
        let ok_replay = reply.status == StatusCode::OK && reply.body == executed[0].body;
        let in_progress =
            reply.status == StatusCode::CONFLICT && reply.body.contains(codes::IN_PROGRESS);
        assert!(ok_replay || in_progress, "unexpected reply {reply:?}");
    }

    // After the winner finishes, a retry replays its response.
    let retry = send(&app, &key, "GALICE", r#"{"pool_id":7}"#).await;
    assert!(retry.replayed);
    assert_eq!(retry.body, executed[0].body);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn expired_keys_are_reusable_and_purged() {
    let Some(pool) = pool().await else { return };
    let policy = IdempotencyPolicy {
        retention: Duration::from_millis(500),
        lease: Duration::from_secs(5),
    };
    let store = IdempotencyStore::postgres(pool.clone());
    let (app, calls) = app(pool, policy);
    let key = key();

    send(&app, &key, "GALICE", r#"{"pool_id":1}"#).await;
    let within = send(&app, &key, "GALICE", r#"{"pool_id":1}"#).await;
    assert!(within.replayed);

    tokio::time::sleep(Duration::from_millis(700)).await;
    // Past retention the key is unused: a different payload executes.
    let after = send(&app, &key, "GALICE", r#"{"pool_id":2}"#).await;
    assert_eq!(after.status, StatusCode::OK);
    assert!(!after.replayed);
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(store.purge_expired().await.unwrap() >= 1);
}
