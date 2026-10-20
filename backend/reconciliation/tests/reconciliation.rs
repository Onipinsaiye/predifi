use axum::{extract::State, routing::post, Json, Router};
use chrono::Utc;
use predifi_reconciliation::{
    rpc::{pool_key, SorobanReader},
    run_pass,
    store::PgProjectionStore,
    Category, ContractReader, Error, IndexedPool, Policy, ProjectionStore,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use stellar_xdr::curr::{
    ContractDataEntry, ExtensionPoint, Int128Parts, LedgerEntryData, LedgerKey, Limits, ReadXdr,
    ScMap, ScMapEntry, ScSymbol, ScVal, WriteXdr,
};

fn contract_id() -> String {
    stellar_strkey::Contract([7; 32]).to_string()
}

#[derive(Clone)]
enum Reply {
    Pool {
        stake: i128,
        state: u32,
        outcome: u32,
    },
    Missing,
    BadXdr,
    WrongKey,
    Error,
    Slow,
}

#[derive(Clone)]
struct RpcState {
    replies: HashMap<i64, Reply>,
    calls: Arc<Mutex<Vec<(i64, Instant)>>>,
}

struct MockRpc {
    url: String,
    state: RpcState,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockRpc {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MockRpc {
    async fn start(replies: impl IntoIterator<Item = (i64, Reply)>) -> Self {
        let state = RpcState {
            replies: replies.into_iter().collect(),
            calls: Default::default(),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/", post(rpc_handler))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { url, state, task }
    }

    fn reader(&self, policy: &Policy) -> SorobanReader {
        SorobanReader::new(&self.url, &contract_id(), policy).unwrap()
    }
}

async fn rpc_handler(State(state): State<RpcState>, Json(request): Json<Value>) -> Json<Value> {
    assert_eq!(request["method"], "getLedgerEntries");
    assert_eq!(request["params"]["keys"].as_array().unwrap().len(), 1);
    let encoded = request["params"]["keys"][0].as_str().unwrap();
    let LedgerKey::ContractData(key) = LedgerKey::from_xdr_base64(encoded, Limits::none()).unwrap()
    else {
        panic!("expected ContractData key")
    };
    let ScVal::Vec(Some(values)) = &key.key else {
        panic!("expected enum key")
    };
    assert_eq!(values[0], symbol("Pool"));
    let ScVal::U64(pool_id) = values[1] else {
        panic!("expected pool ID")
    };
    let pool_id = i64::try_from(pool_id).unwrap();
    assert_eq!(
        pool_key(&contract_id(), pool_id).unwrap(),
        LedgerKey::ContractData(key.clone())
    );
    state.calls.lock().unwrap().push((pool_id, Instant::now()));
    let reply = state.replies.get(&pool_id).unwrap_or(&Reply::Missing);
    if matches!(reply, Reply::Error) {
        return Json(
            json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32603, "message": "private diagnostics"}}),
        );
    }
    if matches!(reply, Reply::Slow) {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let entries = match reply {
        Reply::Missing | Reply::Slow => vec![],
        _ => {
            let val = match reply {
                Reply::Pool {
                    stake,
                    state,
                    outcome,
                } => pool_value(*stake, *state, *outcome),
                _ => ScVal::Void,
            };
            let xdr = LedgerEntryData::ContractData(ContractDataEntry {
                ext: ExtensionPoint::V0,
                contract: key.contract,
                key: key.key,
                durability: key.durability,
                val,
            })
            .to_xdr_base64(Limits::none())
            .unwrap();
            vec![json!({
                "key": if matches!(reply, Reply::WrongKey) {
                    pool_key(&contract_id(), pool_id + 1).unwrap().to_xdr_base64(Limits::none()).unwrap()
                } else { encoded.to_owned() },
                "xdr": if matches!(reply, Reply::BadXdr) { "not-xdr" } else { &xdr },
                "lastModifiedLedgerSeq": 120
            })]
        }
    };
    Json(json!({"jsonrpc": "2.0", "id": 1, "result": {"latestLedger": 125, "entries": entries}}))
}

fn symbol(value: &str) -> ScVal {
    ScVal::Symbol(ScSymbol(value.try_into().unwrap()))
}

fn pool_value(stake: i128, state: u32, outcome: u32) -> ScVal {
    let mut fields = vec![
        ScMapEntry {
            key: symbol("total_stake"),
            val: ScVal::I128(Int128Parts {
                hi: (stake >> 64) as i64,
                lo: stake as u64,
            }),
        },
        ScMapEntry {
            key: symbol("state"),
            val: ScVal::U32(state),
        },
        ScMapEntry {
            key: symbol("outcome"),
            val: ScVal::U32(outcome),
        },
    ];
    fields.sort_by(|a, b| a.key.cmp(&b.key));
    ScVal::Map(Some(ScMap(fields.try_into().unwrap())))
}

fn indexed(pool_id: i64, total_stake: &str) -> IndexedPool {
    IndexedPool {
        pool_id,
        total_stake: total_stake.into(),
        state: "active".into(),
        result: None,
        observed_at: Utc::now(),
    }
}

struct MemoryStore(Vec<IndexedPool>);
impl ProjectionStore for MemoryStore {
    async fn sample(&self, after: i64, limit: usize) -> Result<Vec<IndexedPool>, Error> {
        Ok(self
            .0
            .iter()
            .filter(|row| row.pool_id > after)
            .take(limit)
            .cloned()
            .collect())
    }
}

#[tokio::test]
async fn injected_projection_drift_retains_evidence_and_never_repairs() {
    let rpc = MockRpc::start([(
        42,
        Reply::Pool {
            stake: 900,
            state: 1,
            outcome: 2,
        },
    )])
    .await;
    let policy = Policy::default();
    let original = indexed(42, "100.0000000");
    let store = MemoryStore(vec![original.clone()]);
    let mut cursor = 0;
    let report = run_pass(
        &store,
        &mut rpc.reader(&policy),
        &contract_id(),
        &mut cursor,
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(
        report
            .findings
            .iter()
            .map(|f| f.category)
            .collect::<Vec<_>>(),
        [
            Category::TotalStake,
            Category::Lifecycle,
            Category::Resolution
        ]
    );
    assert_eq!((report.sampled, report.matched, cursor), (1, 0, 42));
    let evidence = serde_json::to_value(&report).unwrap();
    assert_eq!(evidence["contract_id"], contract_id());
    assert!(evidence["run_id"].as_str().is_some());
    for finding in &report.findings {
        assert_eq!(finding.indexed, original);
        let contract = finding.contract.as_ref().unwrap();
        assert_eq!(contract.total_stake, "900");
        assert_eq!(contract.result.as_deref(), Some("2"));
        assert_eq!(
            (contract.latest_ledger, contract.last_modified_ledger),
            (125, 120)
        );
    }
    assert_eq!(store.0, vec![original]);
}

#[tokio::test]
async fn sampling_rotates_over_terminal_pools_and_rate_limits_across_passes() {
    let rpc = MockRpc::start((0..=2).map(|id| {
        (
            id,
            Reply::Pool {
                stake: 500,
                state: 2,
                outcome: u32::MAX,
            },
        )
    }))
    .await;
    let policy = Policy {
        sample_size: 2,
        ..Policy::default()
    };
    let store = MemoryStore(
        (0..=2)
            .map(|id| IndexedPool {
                state: "closed".into(),
                ..indexed(id, "500")
            })
            .collect(),
    );
    let mut reader = rpc.reader(&policy);
    let mut cursor = predifi_reconciliation::INITIAL_CURSOR;
    let first = run_pass(&store, &mut reader, &contract_id(), &mut cursor, &policy)
        .await
        .unwrap();
    assert_eq!((first.sampled, first.matched, cursor), (2, 2, 1));
    let next = run_pass(&store, &mut reader, &contract_id(), &mut cursor, &policy)
        .await
        .unwrap();
    assert_eq!((next.sampled, next.matched, cursor), (1, 1, 2));
    let empty = run_pass(&store, &mut reader, &contract_id(), &mut cursor, &policy)
        .await
        .unwrap();
    assert_eq!((empty.sampled, cursor), (0, -1));
    let calls = rpc.state.calls.lock().unwrap();
    assert_eq!(
        calls.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    for pair in calls.windows(2) {
        assert!(pair[1].1.duration_since(pair[0].1) >= Duration::from_millis(450));
    }
}

#[tokio::test]
async fn errors_are_not_zero_stakes_or_missing_contracts_and_do_not_starve_sampling() {
    let rpc = MockRpc::start([
        (1, Reply::BadXdr),
        (2, Reply::Error),
        (3, Reply::Missing),
        (4, Reply::WrongKey),
    ])
    .await;
    let policy = Policy::default();
    let store = MemoryStore((1..=4).map(|id| indexed(id, "0")).collect());
    let mut cursor = 0;
    let report = run_pass(
        &store,
        &mut rpc.reader(&policy),
        &contract_id(),
        &mut cursor,
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(cursor, 4);
    assert_eq!(report.matched, 0);
    assert_eq!(
        report
            .findings
            .iter()
            .map(|f| f.category)
            .collect::<Vec<_>>(),
        [
            Category::ContractReadFailed,
            Category::ContractReadFailed,
            Category::ContractUnavailable,
            Category::ContractReadFailed
        ]
    );
    assert!(report.findings.iter().all(|f| f.contract.is_none()));
    assert_eq!(report.findings[2].latest_ledger, Some(125));
    assert!(!serde_json::to_string(&report)
        .unwrap()
        .contains("private diagnostics"));
}

#[tokio::test]
async fn exact_i128_stakes_and_fractional_projection_validation() {
    let stake = 12_345_678_901_234_567_890_123_456_i128;
    let rpc = MockRpc::start((1..=2).map(|id| {
        (
            id,
            Reply::Pool {
                stake,
                state: 0,
                outcome: u32::MAX,
            },
        )
    }))
    .await;
    let policy = Policy::default();
    let store = MemoryStore(vec![
        indexed(1, &format!("{stake}.0000000")),
        indexed(2, "1.5000000"),
    ]);
    let report = run_pass(
        &store,
        &mut rpc.reader(&policy),
        &contract_id(),
        &mut 0,
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(report.matched, 1);
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].category, Category::InvalidProjection);
}

#[tokio::test]
async fn slow_rpc_times_out_without_retry() {
    let rpc = MockRpc::start([(1, Reply::Slow)]).await;
    let policy = Policy {
        rpc_timeout: Duration::from_millis(200),
        ..Policy::default()
    };
    let report = run_pass(
        &MemoryStore(vec![indexed(1, "0")]),
        &mut rpc.reader(&policy),
        &contract_id(),
        &mut 0,
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(report.findings[0].category, Category::ContractReadFailed);
    assert_eq!(rpc.state.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn unknown_contract_schema_fails_closed() {
    let rpc = MockRpc::start([(
        1,
        Reply::Pool {
            stake: 0,
            state: 99,
            outcome: 0,
        },
    )])
    .await;
    assert!(rpc.reader(&Policy::default()).read_pool(1).await.is_err());
}

#[test]
fn limits_cannot_be_disabled_or_exceeded() {
    for policy in [
        Policy {
            sample_size: 0,
            ..Policy::default()
        },
        Policy {
            sample_size: 101,
            ..Policy::default()
        },
        Policy {
            interval: Duration::from_secs(59),
            ..Policy::default()
        },
        Policy {
            rpc_spacing: Duration::ZERO,
            ..Policy::default()
        },
        Policy {
            rpc_timeout: Duration::ZERO,
            ..Policy::default()
        },
        Policy {
            rpc_timeout: Duration::from_secs(11),
            ..Policy::default()
        },
    ] {
        assert!(policy.validate().is_err());
    }
    assert!(pool_key("not-a-contract", 1).is_err());
    assert!(pool_key(&contract_id(), -1).is_err());
}

#[test]
fn pool_storage_key_matches_contract_abi_fixture() {
    // Persistent CONTRACT_DATA(address=[7;32], [symbol("Pool"), u64(42)]).
    // Literal XDR fixture is independent of the mock server's key builder.
    assert_eq!(
        pool_key(&contract_id(), 42).unwrap().to_xdr_base64(Limits::none()).unwrap(),
        "AAAABgAAAAEHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwAAABAAAAABAAAAAgAAAA8AAAAEUG9vbAAAAAUAAAAAAAAAKgAAAAE="
    );
}

/// Uses the repository's real pool schema, an injected projection bug, and a
/// mocked Soroban RPC returning genuine LedgerEntryData XDR. CI always runs it.
#[tokio::test]
#[ignore = "requires RECONCILE_TEST_DATABASE_URL; run with --include-ignored"]
async fn postgres_projection_discrepancy_preserves_history() {
    use sqlx::postgres::PgPoolOptions;
    let url = std::env::var("RECONCILE_TEST_DATABASE_URL").expect("test database URL required");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    // A connection-local schema avoids touching any existing tables or history.
    sqlx::raw_sql(
        &include_str!("../../migrations/002_create_pools.sql").replace(
            "CREATE TABLE IF NOT EXISTS pools",
            "CREATE TEMP TABLE pools",
        ),
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql("ALTER TABLE pools ALTER COLUMN total_stake TYPE NUMERIC(32,7);
        ALTER TABLE pools ADD COLUMN contract_id TEXT;
        CREATE TEMP TABLE projection_history (event_id TEXT PRIMARY KEY, pool_id BIGINT, payload TEXT);
        INSERT INTO projection_history VALUES ('ledger-120-event-42', 42, 'prediction stake 900');")
        .execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO pools (pool_id, name, total_stake, state, end_time, contract_id)
        VALUES (42, 'injected drift', 900, 'active', NOW(), $1),
               (43, 'healthy', 500, 'active', NOW(), $1),
               (44, 'other contract', 100, 'active', NOW(), 'OTHER'),
               (45, 'unassigned legacy pool', 100, 'active', NOW(), NULL)",
    )
    .bind(contract_id())
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE pools SET total_stake = 100 WHERE pool_id = 42")
        .execute(&pool)
        .await
        .unwrap();
    let before: Vec<(i64, String, String, Option<String>)> = sqlx::query_as(
        "SELECT pool_id, total_stake::text, state, result FROM pools ORDER BY pool_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let history_before: Vec<(String, i64, String)> =
        sqlx::query_as("SELECT * FROM projection_history")
            .fetch_all(&pool)
            .await
            .unwrap();
    let rpc = MockRpc::start([
        (
            42,
            Reply::Pool {
                stake: 900,
                state: 1,
                outcome: 2,
            },
        ),
        (
            43,
            Reply::Pool {
                stake: 500,
                state: 0,
                outcome: u32::MAX,
            },
        ),
    ])
    .await;
    let policy = Policy::default();
    let store = PgProjectionStore::new(pool.clone(), contract_id());
    let report = run_pass(
        &store,
        &mut rpc.reader(&policy),
        &contract_id(),
        &mut 0,
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(
        (report.sampled, report.matched, report.findings.len()),
        (2, 1, 3)
    );
    assert_eq!(report.findings[0].indexed.total_stake, "100.0000000");
    assert_eq!(
        report.findings[0].contract.as_ref().unwrap().total_stake,
        "900"
    );
    let after: Vec<(i64, String, String, Option<String>)> = sqlx::query_as(
        "SELECT pool_id, total_stake::text, state, result FROM pools ORDER BY pool_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let history_after: Vec<(String, i64, String)> =
        sqlx::query_as("SELECT * FROM projection_history")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(after, before);
    assert_eq!(history_after, history_before);
    // Parameterized keyset selection is enforced by the actual SQL store.
    assert_eq!(store.sample(42, 1).await.unwrap()[0].pool_id, 43);
    assert!(store.sample(43, 1).await.unwrap().is_empty());
    assert!(store.sample(0, 101).await.is_err());
    pool.close().await;
}
