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
