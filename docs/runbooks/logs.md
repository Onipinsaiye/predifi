# Log-based Alert Runbooks

These alerts are evaluated by the Loki ruler
(`docker/loki/rules/fake/predifi-log-alerts.yml`). Explore logs in Grafana
`/d/predifi-logs` or with LogQL in *Explore*.

---

## BackendErrorLogSpike

**Owner:** backend-oncall · **Severity:** critical

**User impact:** The backend is logging more than 0.2 errors per second. Most
errors correspond to failed user requests or failed indexer jobs.

**Dashboards & queries:**
- `{job="predifi", service="backend", level="ERROR"}`
- Top messages: `sum by (message) (count_over_time({job="predifi", service="backend", level="ERROR"} | json [5m]))`
- Compare with the 5xx rate in `/d/predifi-backend-prod`.

**Diagnosis:**
1. Group by message. One message dominating points at one cause.
2. If errors match 5xx responses → follow [api.md](api.md#highapierrorrate).
3. If errors are from the worker → follow [worker.md](worker.md#workerqueuebacklog).
4. If errors started at a deploy → that release is the likely cause.

**Safe mitigation:**
1. Roll back the most recent release if the errors started with it.
2. Otherwise follow the runbook for the subsystem the errors come from.

**Verification:** Error log rate drops below 0.2/s for 10 minutes.

**Rollback:** Redeploy the newer release once the fix is in.

**Escalation:** Not explained in 30 minutes → secondary backend-oncall.

---

## BackendPanic

**Owner:** backend-oncall · **Severity:** critical

**User impact:** A Rust panic aborted a request or a background task. The
request that triggered it failed; if a background task panicked, the work it
was doing (indexing, notifications) may have stopped.

**Dashboards & queries:**
- `{job="predifi", service="backend"} |~ "(?i)panicked at"`
- Sentry (if `PREDIFI_SENTRY_DSN` is set) groups panics with stack traces.

**Diagnosis:**
1. Read the panic message and location (`panicked at src/...:line`).
2. Check whether the panic is repeating. A one-off on bad input is a bug to
   file; a repeating panic in a background task means that task is down.
3. Check `GET /health` → worker status to see whether background work stopped.

**Safe mitigation:**
1. **Repeating panic after a deploy:** roll back.
2. **Background task stopped:** restart instances one at a time to restart it.
3. **Triggered by specific input:** if you can identify the route, block the
   specific request pattern at the load balancer until a fix ships.

**Verification:** No new panic lines for 15 minutes, and `GET /health` shows
the worker running.

**Rollback:** Remove any load balancer rule you added once the fix ships.

**Escalation:** Every panic gets a bug ticket for backend-oncall. Panics in
code that handles payouts or contract data → `@Web3Novalabs/maintainers`.

---

## DatabaseErrorLogs

**Owner:** backend-oncall · **Severity:** warning

**User impact:** Intermittent database errors. Usually a few failed requests or
retried jobs; often an early sign of pool saturation or a failing query.

**Dashboards & queries:**
- `{job="predifi", service="backend", level="ERROR"} |~ "(?i)(database|connection pool|sqlx)"`
- `sum by (query_type) (rate(app_db_queries_total{result="error"}[5m]))`

**Diagnosis:** Follow [database.md](database.md#databasequeryfailures). Pool
timeout messages (`pool timed out`) point at
[pool saturation](database.md#databaseconnectionpoolcritical).

**Safe mitigation:** Follow the database runbook for the cause you find.

**Verification:** Fewer than 5 database error lines per 10 minutes.

**Rollback:** As described in the database runbook you followed.

**Escalation:** Rising error count, or `DatabaseQueryFailures` fires →
secondary backend-oncall.

---

## NoBackendLogs

**Owner:** infra-oncall · **Severity:** warning

**User impact:** Either the backend is down (users affected — check the site)
or log shipping stopped (users fine, operators blind).

**Dashboards & queries:**
- `sum(count_over_time({job="predifi", service="backend"}[10m]))`
- Promtail targets: `http://<promtail-host>:9080/targets`
- Backend liveness: `GET /live` on an instance; `app_up` in Prometheus.

**Diagnosis:**
1. Hit `GET /live`. No answer → the backend is down; treat as
   [HighApiErrorRate](api.md#highapierrorrate).
2. Backend answers → check Promtail is running and its targets are up.
3. Check Loki is up and has disk space (`docker/loki/loki-config.yml`).

**Safe mitigation:**
1. Restart Promtail, then Loki if needed. Neither restart affects users.
2. If Loki's disk is full, raise retention cleanup or expand the volume.

**Verification:** Backend log lines appear in Loki again.

**Rollback:** None needed; restarts are safe.

**Escalation:** Backend down → page backend-oncall immediately. Log pipeline
still broken after one business hour → secondary infra-oncall.
