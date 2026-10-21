# API Runbooks

Shared context for API alerts:

- **Rolling back a release:** redeploy the previous backend image/commit through
  the normal deploy pipeline. Check [`docs/deploy/canary.md`](../deploy/canary.md)
  and confirm the previous release is schema-compatible before rolling back
  (see [`docs/migrations/expand-migrate-contract.md`](../migrations/expand-migrate-contract.md)).
- **Raising log detail without a restart:**
  `PATCH /api/v1/admin/log-level` (see [`docs/admin-log-level.md`](../admin-log-level.md)).
  Set it back to `info` when you are done.

---

## HighApiErrorRate

**Owner:** backend-oncall · **Severity:** critical

**User impact:** More than 1% of API requests return 5xx. Users see failed page
loads, empty pool lists, and cannot view predictions or claim status. If this
coincides with `TxPreparationFastBurn`, users also cannot start stakes.

**Dashboards & queries:**
- Grafana `/d/predifi-backend-prod` → *HTTP error rate* and *Requests by path* panels.
- Which routes are failing:
  ```promql
  topk(10, sum by (path, status) (rate(app_http_requests_total{status=~"5.."}[5m])))
  ```
- Error logs (Loki): `{job="predifi", service="backend", level="ERROR"}`
- Panics (Loki): `{job="predifi", service="backend"} |~ "(?i)panicked at"`

**Diagnosis:**
1. Check whether a deploy happened in the last hour. A deploy followed by the
   alert is the most likely cause.
2. Group errors by path with the query above. One path failing points at a code
   or data bug; every path failing points at a dependency.
3. If every path fails, check `GET /ready` on a backend instance and the
   `DatabaseQueryFailures`, `DatabaseConnectionPoolCritical`, and
   `HighRedisErrors` alerts. Postgres or Redis being down shows up here first.
4. Check Soroban RPC: `GET /api/v1/health` reports the RPC check.
5. Read the first error log lines after the alert started — the first error is
   usually the cause; later ones are fallout.

**Safe mitigation:**
1. **Recent deploy:** roll back to the previous release (see top of page).
2. **Database down/saturated:** follow [database.md](database.md#databaseconnectionpoolcritical).
3. **Redis down:** the backend runs degraded without Redis when
   `PREDIFI_REDIS_REQUIRED=false`; follow [cache.md](cache.md#highrediserrors).
4. **Soroban RPC down:** switch `PREDIFI_STELLAR_RPC_URL` to the backup RPC
   provider and restart instances one at a time.
5. **One bad instance:** take it out of the load balancer target group and let
   the auto scaling group replace it.

**Verification:** The 5xx ratio drops below 1% and stays there for 10 minutes:
```promql
(sum(rate(app_http_requests_total{status=~"5.."}[5m])) / sum(rate(app_http_requests_total[5m]))) * 100
```
The alert resolves on its own.

**Rollback:** If a rollback made errors worse (for example a schema mismatch),
redeploy the release you rolled back from. If you changed the RPC URL, restore
the original value and restart. Instances removed from the target group can be
re-registered.

**Escalation:** Not mitigated after 30 minutes → secondary backend-oncall. If
the cause is the database host, Redis, or networking → also page infra-oncall.
Contract errors → contract-oncall.

---

## HighP99Latency

**Owner:** backend-oncall · **Severity:** warning

**User impact:** The slowest 1% of requests take over 500ms. Pages feel slow and
users may see odds or pool totals that are a few seconds stale.

**Dashboards & queries:**
- Grafana `/d/predifi-backend-prod` → *Latency percentiles* panel.
- Slowest routes:
  ```promql
  topk(5, sum by (path) (rate(app_http_request_duration_seconds_sum[5m])) / sum by (path) (rate(app_http_request_duration_seconds_count[5m])))
  ```
- Slowest queries:
  ```promql
  topk(5, histogram_quantile(0.99, sum by (le, query_type) (rate(app_db_query_duration_seconds_bucket[5m]))))
  ```
- Redis latency: `histogram_quantile(0.99, sum by (le) (rate(app_redis_operation_duration_seconds_bucket[5m])))`

**Diagnosis:**
1. Find the slow routes with the first query. Is it one route or all of them?
2. One route → check its database query with the second query. A slow
   `query_type` after a deploy or migration usually means a missing index.
3. All routes → check database pool utilization and CPU/memory on the backend
   instances. A pool near capacity makes every request wait.
4. Check the Redis cache hit rate (`app_redis_cache_lookups_total` by
   `outcome`). A cache that stopped hitting sends all reads to Postgres.
5. Check request volume. A traffic spike with flat error rate is a capacity
   problem, not a bug.

**Safe mitigation:**
1. **Recent deploy or migration:** roll back the application release.
2. **Missing index:** create it with `CREATE INDEX CONCURRENTLY` (does not block
   writes), then add the matching migration in a follow-up PR.
3. **Traffic spike:** raise the auto scaling group's desired capacity by one or
   two instances.
4. **Long-running query:** cancel it with `SELECT pg_cancel_backend(<pid>);`
   (never `pg_terminate_backend` on the indexer's connection unless asked to by
   the indexer owner).

**Verification:** p99 drops below 500ms for 10 minutes:
`histogram_quantile(0.99, sum(rate(app_http_request_duration_seconds_bucket[5m])) by (le))`

**Rollback:** Lower the desired capacity back to the previous value. Drop an
index you added with `DROP INDEX CONCURRENTLY <name>;` if it did not help.

**Escalation:** Still above threshold after one business hour, or rising toward
`ApiLatencyFastBurn` → secondary backend-oncall. Capacity changes beyond two
extra instances → infra-oncall.
