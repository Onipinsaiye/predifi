# Worker & Indexer Runbooks

The background worker ingests Soroban contract events, keeps pool and
prediction tables in sync with the chain, and runs follow-up jobs. Jobs retry
with backoff; a job that exhausts its retries moves to the dead-letter queue
(DLQ). See `backend/src/worker/queue.rs`.

> **Known gap:** `app_worker_queue_depth` and `app_worker_dlq_depth` are only
> updated if `Metrics::record_worker_queue_depth` is called from the worker
> loop. Until that is wired in, these alerts read 0. Check the worker's
> `/health` output (`WorkerHealth`) directly when in doubt.

---

## WorkerQueueBacklog

**Owner:** backend-oncall · **Severity:** warning

**User impact:** More than 50 jobs are waiting. New pools, stakes, and
resolutions appear in the app late. The chain is still correct; only the app's
view of it is behind.

**Dashboards & queries:**
- Grafana `/d/predifi-backend-prod` → *Worker queue* panel.
- Backlog trend: `app_worker_queue_depth`
- SLO view: Grafana `/d/predifi-slo` → *Indexer freshness*.
- Worker logs (Loki): `{job="predifi", service="backend"} |~ "(?i)(worker|retry|backoff)"`

**Diagnosis:**
1. Is the backlog growing or draining? Draining after a spike needs no action.
2. Look for retry/backoff messages in the logs. The retry reason is usually an
   RPC error (rate limit, timeout) or a database error.
3. Check Soroban RPC health via `GET /api/v1/health` and the provider's status
   page. HTTP 429 from the RPC means we are being throttled.
4. Check database health: `DatabaseQueryFailures` or pool saturation slows every
   job.

**Safe mitigation:**
1. **RPC throttling or outage:** point `PREDIFI_STELLAR_RPC_URL` at the backup
   RPC provider and restart instances one at a time.
2. **Large catch-up after downtime:** lower `PREDIFI_INDEXER_MAX_BATCH_SIZE`
   (default 500) if batches are timing out, so each cycle completes.
3. **Database slow:** follow [database.md](database.md).
4. Do **not** clear the queue. Dropped jobs mean missing on-chain data.

**Verification:** `app_worker_queue_depth` falls below 50 and keeps falling.
Spot-check one recent on-chain pool: it appears in `GET /api/v1/pools/<id>`.

**Rollback:** Restore the original `PREDIFI_STELLAR_RPC_URL` /
`PREDIFI_INDEXER_MAX_BATCH_SIZE` and restart one instance at a time.

**Escalation:** Backlog still growing after one business hour, or
`IndexerFreshnessFastBurn` fires → secondary backend-oncall. Suspected contract
event format change → contract-oncall.

---

## WorkerDLQBacklog

**Owner:** backend-oncall · **Severity:** critical

**User impact:** More than 10 jobs failed permanently. The events they carried
are not reflected in the app: a pool may show the wrong total, a prediction may
be missing, or a resolution may not appear. Funds on chain are not affected.

**Dashboards & queries:**
- `app_worker_dlq_depth`
- Worker health: `GET /health` on a backend instance (the `WorkerHealth` block
  lists DLQ size and last error).
- Final-failure logs (Loki): `{job="predifi", service="backend", level="ERROR"} |~ "(?i)(dead.?letter|dlq)"`

**Diagnosis:**
1. Read the last error for the dead-lettered jobs. Are they all the same error?
2. Same error, same event type → a code bug or an event schema the backend
   does not understand (for example after a contract upgrade).
3. Mixed errors around one time → a dependency outage (RPC, database) that
   outlasted the retry budget.
4. Note the affected pool IDs so you can verify them after replay.

**Safe mitigation:**
1. **Dependency outage (now recovered):** replay the dead-lettered jobs with
   `JobQueue::requeue_from_dlq(job_id)` from an operator shell or a one-off
   binary. There is no HTTP endpoint for this yet.
2. **Code or schema bug:** do not replay — they will fail again. Ship the fix
   first, then replay.
3. If a pool's totals are wrong and replay is blocked, the reconciliation job
   (`backend/reconciliation`, `.github/workflows/reconciliation.yml`) and
   `run_full_sync` in `backend/src/worker/sync.rs` re-read state from the chain.

**Verification:** `app_worker_dlq_depth` returns to 0 (or below 10 and falling).
For each affected pool ID, `GET /api/v1/pools/<id>` matches the on-chain totals.

**Rollback:** Replay is idempotent for indexed events (writes are upserts keyed
on chain IDs). If a replayed job fails again, it returns to the DLQ — stop
replaying and escalate.

**Escalation:** Not understood within 30 minutes → secondary backend-oncall.
Any sign the contract emitted unexpected events → contract-oncall and
`@Web3Novalabs/maintainers` immediately.

---

## Abuse quotas (market creation & wallet actions)

Per-wallet and per-network quotas protect backend capacity from automated
market spam, account cycling, and repeated expensive wallet-triggering
requests. Quotas are keyed on the authenticated wallet, **not** on client IP:
shared IPs (NAT, mobile carriers, corporate proxies) are never treated as a
single identity, so legitimate multi-wallet users behind one IP are unaffected.

**Tuning knobs (environment):**
- `PREDIFI_ABUSE_WALLET_QUOTA_<ACTION>` — per-wallet limit per window for
  `MARKET_CREATE`, `WALLET_SIGN`, `WALLET_SUBMIT` (e.g. `10/1h`).
- `PREDIFI_ABUSE_NETWORK_QUOTA_<NETWORK>` — aggregate per-network ceiling
  across all wallets (e.g. `testnet: 500/1m`, `mainnet: 200/1m`).
- `PREDIFI_ABUSE_MIN_VALIDATION_COST` — minimum validation cost (in stroops)
  a request must justify before it is admitted; cheap, low-value requests are
  rejected early so expensive validation is not spent on spam.
- `PREDIFI_ABUSE_WINDOW_SECS` — sliding-window length for quota accounting.

**Rejection behavior:** over-quota requests return HTTP `429` with a
transparent body (`error: "quota_exceeded"`, `scope: "wallet" | "network"`,
`retry_after_secs`, and the published limit). Rejections are counted, never
silently dropped, and never consume validation budget.

**Metrics:** `app_abuse_rejections_total{scope,action,network}`,
`app_abuse_quota_utilization{scope,action,network}` (0–1),
`app_abuse_validation_cost_rejected_total`. Alert when network-scope
utilization stays above 0.8 for 10 minutes (raise the network ceiling or
investigate a spam source) or when wallet-scope rejections spike for many
distinct wallets (likely a client bug or a coordinated campaign).

**Validation:** run the backend abuse simulation
(`backend/tests/abuse_simulation`) to confirm bounded resource use, and
confirm legitimate multi-wallet users stay within the published limits before
tightening any quota.

---

## RPC provider quotas & failover

The worker and transaction-preparation paths share a per-network RPC budget so
public rate limits or a provider outage cannot stall indexing or user
transaction preparation.

**Policy:**
- **Per-network request budgets:** `PREDIFI_RPC_BUDGET_<NETWORK>` (requests
  per window) split across priority classes.
- **Priority classes:** `INDEXER` (background ingestion), `TX_PREP` (user
  transaction preparation), `CRITICAL` (health/reconciliation). Under
  pressure, `INDEXER` is shed first; `TX_PREP` and `CRITICAL` are preserved.
- **Provider health:** each provider is probed; unhealthy providers are
  removed from rotation until they recover.
- **Fallback ordering:** `PREDIFI_RPC_FALLBACK_ORDER_<NETWORK>` (comma-separated
  provider list). Requests fail over in order, with bounded retries and
  jittered backoff; retries never re-submit a transaction that may already
  have been accepted (submission is idempotent and keyed on the transaction
  hash), so there are no duplicate submissions.
- **Provider disagreement:** when providers return conflicting ledger/state,
  the request is retried against the next provider; if disagreement persists,
  the worker pauses that stream and raises an alert rather than writing
  ambiguous data.

**Metrics:** `app_rpc_requests_total{network,provider,priority,outcome}`,
`app_rpc_budget_utilization{network,priority}`,
`app_rpc_provider_health{network,provider}` (1 healthy / 0 unhealthy),
`app_rpc_failovers_total{network,provider}`, and
`app_rpc_disagreements_total{network}`. Review these on the operator
dashboard; raise a network budget only after confirming provider headroom,
and reorder fallbacks when a provider's health flaps.

**Validation:** run the RPC worker fault-injection tests
(`backend/tests/rpc_fault_injection`) — simulated throttling and provider
outage must produce bounded retries and no duplicate transaction submission.
