# Indexed market reconciliation (#46)

This standalone backend worker compares selected PostgreSQL projections against
authoritative Soroban storage. It can run beside the server without changing the
server, event listener, existing sync helper, schema, or deployment configuration.
It has its own Cargo workspace so it can be reviewed and validated independently.
Starting the server does not start this worker; an operator must opt in.

## Run

Use the same network endpoint as the indexer, a **SELECT-only database role**, and
the exact deployed contract's C-address:

```bash
export PREDIFI_DATABASE_URL='postgres://reconciliation_reader:password@localhost/predifi'
export PREDIFI_STELLAR_RPC_URL='https://soroban-testnet.stellar.org'
export RECONCILE_CONTRACT_ID='C...'
cargo run --locked --manifest-path backend/reconciliation/Cargo.toml
# One bounded pass for diagnosis (nonzero exit if any finding or pass failure):
cargo run --locked --manifest-path backend/reconciliation/Cargo.toml -- --once
```

The worker emits one JSON report per pass on stdout. Forward stdout to the
deployment's retained log store and alert when `findings` is nonempty. Pass-level
database failures emit `reconciliation_pass_failed` on stderr. Retain reports
before any replay or repair. Ctrl-C cancels the current pass. No signing keys,
transaction submissions, or contract writes are involved.

The database must have the existing `pools.contract_id` column from migration
`004_enhance_schema_precision.sql`. Only rows with an exact contract ID match are
eligible. NULL legacy assignments are excluded: operators must establish their
provenance through the normal indexing/backfill process, never guess a network.

## Bounded sampling and RPC budget

| Setting | Default | Enforced range |
| --- | --- | --- |
| `RECONCILE_SAMPLE_SIZE` | 25 pools | 1–100 |
| `RECONCILE_INTERVAL_SECS` | 300 seconds after pass completion | 60–86400 |
| `RECONCILE_RPC_SPACING_MS` | 500 ms between request starts | 500–10000 |
| `RECONCILE_RPC_TIMEOUT_SECS` | 10 seconds per request, including body | 1–10 |

- Sample all lifecycle states using ascending `pool_id` keyset pages with a SQL
  `LIMIT`; there is no random full-table scan or unbounded fetch. Pool zero is
  included. A cursor advances through the sample even on RPC failures. An empty
  page resets it to -1, and the following pass starts a new sweep. Each eligible
  row in a stable dataset is visited once per sweep, including terminal pools.
- The cursor is in memory. A restart begins at pool zero. Run one long-lived
  instance per network/contract and avoid frequent restarts that could starve
  later IDs. There is no promise of detecting drift in every pool every interval.
  A stable set of N rows takes `ceil(N / sample_size) + 1` passes per sweep.
- Each sampled pool costs exactly one `getLedgerEntries` call for its persistent
  `DataKey::Pool(u64)` key, using real base64 XDR. At most 100 RPC calls per pass,
  concurrency one, at most two request starts per second, no redirects or retries.
  Spacing survives pass boundaries; errors consume the same budget as successes.
  Per-instance limits multiply if operators run replicas.
- Passes never overlap or catch up in bursts. Each request has a total deadline
  of at most 10 seconds, response size at most 1 MiB, and bounded XDR decoding.
  SQL statement timeout and connection acquisition timeout are each five seconds.
  No migrations, indexes, or database writes are performed by the worker.

The RPC adapter uses the current contract's storage layout in
`contract/contracts/predifi-contract/src/{lib,pool}.rs`. `Pool(u64)` is a symbol
plus u64 SCVec key; the stored Pool is an SCMap. Integer `MarketState` values map
0 → `active`, 1 → `settled`, 2 → `closed`, 3 → `disputed`. The current database
constraint cannot represent disputed pools, so those produce lifecycle alerts.
RPC envelope, returned key, contract address, durability, and ledger ordering are
validated. Unknown or malformed field encodings fail the read; they never become
zero stakes. See the official
[getLedgerEntries reference](https://developers.stellar.org/docs/data/apis/rpc/api-reference/methods/getLedgerEntries).

## Mismatch categories and repair policy

| Category | Meaning | Action |
| --- | --- | --- |
| `total_stake` | Indexed raw base-unit stake differs from contract i128 | Alert; inspect missed stake/liquidity events and aggregation |
| `lifecycle` | Indexed state differs from mapped contract state | Alert; inspect resolution/cancellation/dispute events |
| `resolution` | Indexed result differs from the resolved outcome (or should be NULL) | Alert; inspect resolution projection and event ordering |
| `invalid_projection` | Indexed stake is fractional, negative, or outside i128 | Alert; investigate unit/precision conversion |
| `contract_unavailable` | No live Pool entry returned | Alert; verify network, contract, archival/TTL; absence does not prove deletion |
| `contract_read_failed` | Transport, HTTP, JSON-RPC, identity, XDR, or schema failure | Alert as inconclusive; verify provider/schema, do not classify as drift |

All categories are **alert only**. There is no automatic repair path. The contract
is authoritative; the checker never edits pool rows, predictions, event history,
or ingestion cursors. It does not call the older `run_full_sync` helper.

Stake comparison uses raw token base units, matching integer amounts indexed by
the backend. PostgreSQL NUMERIC is read as text: trailing decimal zeros are
normalized exactly, without a float or i64 conversion. Nonzero fractional units
are invalid projections, not rounded values. No implicit token-decimal scaling
is assumed. Winning outcome comparisons use the backend's decimal text result;
nonresolved contract pools expect NULL.

Each finding includes the contract ID and pass UUID via its enclosing report,
pool ID, database observation time, both stake/state/result snapshots, latest RPC
ledger, and entry modification ledger. These observations are taken at different
times: an active indexer may catch up after the database sample, so one finding
is evidence of observed drift, not proof of its cause.

Before repair, retain the report, verify the network/contract and indexer health,
allow ingestion to catch up, and confirm the mismatch in a subsequent sweep.
Inspect source events around the entry's modification ledger and the indexer's
checkpoint/DLQ. Repair through an operator-reviewed, idempotent replay/backfill of
verified events or a reviewed projection rebuild with a backup. Keep original
events and the before/after evidence, then rerun reconciliation. RPC failures or
missing live storage never justify overwriting values or deleting history.

This sample detects drift in **already indexed, assigned pools**. Discovering
contract-only pools after a missed creation event, checking all prediction/user
records, durable sweep checkpoints, alert deduplication, and automatic replay are
outside this worker's scope.

## Validation

```bash
cargo fmt --manifest-path backend/reconciliation/Cargo.toml -- --check
cargo clippy --locked --manifest-path backend/reconciliation/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path backend/reconciliation/Cargo.toml
# Dedicated test database; all test tables are connection-local TEMP tables:
RECONCILE_TEST_DATABASE_URL='postgres://postgres:postgres@localhost/predifi_test' \
  cargo test --locked --manifest-path backend/reconciliation/Cargo.toml -- --include-ignored
```

The integration suite serves mocked Soroban discrepancies over HTTP with genuine
LedgerEntryData XDR. The PostgreSQL case uses the existing pool schema, injects
stake drift, verifies all three mismatch categories and contract filtering, and
asserts every projection row and history fixture remains unchanged. The dedicated
CI workflow always runs this database case, plus formatting and strict Clippy.
