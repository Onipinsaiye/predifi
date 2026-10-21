# SLO Burn-Rate Runbooks

Burn-rate alerts fire when an SLO is spending its error budget faster than the
28-day window allows. The indicators, targets, and budget policy are defined in
[`docs/observability/slos.md`](../observability/slos.md); the rules are in
`backend/grafana/alerts/predifi_slo_rules.yml`.

- **Fast burn (critical):** at this rate the whole 28-day budget is gone in
  under 5 days. Something is broken now.
- **Slow burn (warning):** a low, steady rate that will still miss the target
  by the end of the window. Something has degraded and nobody noticed.

Both are symptoms. Each section below says which cause-level runbook to use.

Remaining budget per SLO: `slo:<sli>:error_budget_remaining` (1 = untouched,
0 = spent), shown on Grafana `/d/predifi-slo`.

---

## ApiAvailabilityFastBurn

**Owner:** backend-oncall · **Severity:** critical

**User impact:** More than ~3% of user requests are failing (the exact
threshold is 14.4× or 6× the 0.5% budget). Users see errors across the app.

**Dashboards & queries:**
- `/d/predifi-slo` → *API availability*.
- `slo:api_availability:error_ratio_rate5m`, `slo:api_availability:error_ratio_rate1h`
- Failing routes: `topk(10, sum by (path, status) (rate(app_http_requests_total{status=~"5.."}[5m])))`

**Diagnosis:** Follow [HighApiErrorRate](api.md#highapierrorrate). This alert
excludes health and metrics endpoints, so it can fire when `HighApiErrorRate`
is borderline.

**Safe mitigation:** As in [HighApiErrorRate](api.md#highapierrorrate). Roll
back first if a deploy happened in the last hour.

**Verification:** `slo:api_availability:error_ratio_rate5m` below 0.005 and
the 1h ratio falling.

**Rollback:** As in the cause-level runbook you followed.

**Escalation:** Critical. Not mitigated in 30 minutes → secondary
backend-oncall and an incident commander.

---

## ApiAvailabilitySlowBurn

**Owner:** backend-oncall · **Severity:** warning

**User impact:** A small, steady share of requests fail — not enough to page
on, enough to miss the 99.5% monthly target.

**Dashboards & queries:**
- `/d/predifi-slo` → *API availability* (look at the 3-day trend).
- Errors by route over a day: `topk(10, sum by (path, status) (increase(app_http_requests_total{status=~"5.."}[1d])))`

**Diagnosis:**
1. Find which route contributes most errors over the last day.
2. Look for a recurring cause: a slow dependency timing out, a specific bad
   input, one unhealthy instance.

**Safe mitigation:** File a bug for the top contributor and fix it in normal
working hours. Remove an unhealthy instance if one stands out.

**Verification:** `slo:api_availability:error_ratio_rate6h` below 0.005.

**Rollback:** Re-add any instance you removed once healthy.

**Escalation:** If remaining budget drops below 25%, apply the budget policy in
[`slos.md`](../observability/slos.md#error-budget-policy) and tell the backend team.

---

## ApiLatencyFastBurn

**Owner:** backend-oncall · **Severity:** critical

**User impact:** More than ~6% of user requests take over 500ms. The app feels
slow and odds load late.

**Dashboards & queries:**
- `/d/predifi-slo` → *API latency*.
- `slo:api_latency:error_ratio_rate5m`, `slo:api_latency:error_ratio_rate1h`

**Diagnosis:** Follow [HighP99Latency](api.md#highp99latency).

**Safe mitigation:** As in [HighP99Latency](api.md#highp99latency).

**Verification:** `slo:api_latency:error_ratio_rate5m` below 0.01.

**Rollback:** As in the cause-level runbook you followed.

**Escalation:** Not mitigated in 30 minutes → secondary backend-oncall;
capacity problems → infra-oncall.

---

## ApiLatencySlowBurn

**Owner:** backend-oncall · **Severity:** warning

**User impact:** A steady share of requests are slow.

**Dashboards & queries:**
- Slowest routes over a day:
  `topk(5, sum by (path) (increase(app_http_request_duration_seconds_sum[1d])) / sum by (path) (increase(app_http_request_duration_seconds_count[1d])))`

**Diagnosis:** Find the route that contributes most slow requests and its
database `query_type`. Slow-burn latency is usually a query that got slower as
a table grew.

**Safe mitigation:** Add the missing index (`CREATE INDEX CONCURRENTLY`) or fix
the query in a normal PR.

**Verification:** `slo:api_latency:error_ratio_rate6h` below 0.01.

**Rollback:** `DROP INDEX CONCURRENTLY` if a new index did not help.

**Escalation:** Remaining budget under 25% → apply the budget policy.

---

## TxPreparationFastBurn

**Owner:** backend-oncall · **Severity:** critical

**User impact:** The reads the app needs before a wallet prompt (`/v1/fees`,
`/v1/prices`, `/v1/pools/<id>`) are failing or slower than 1s. Users cannot
reliably start a stake or a claim.

**Dashboards & queries:**
- `/d/predifi-slo` → *Transaction preparation*.
- By route:
  `sum by (path, status) (rate(app_http_requests_total{path=~".*/v1/(fees|prices|pools/[^/]+)"}[5m]))`
- Price cache health: `sum by (result) (rate(app_price_cache_fetch_total[5m]))`

**Diagnosis:**
1. `/v1/prices` failing → the upstream price source or the price cache is
   failing; check `app_price_cache_fetch_total` and logs for the provider error.
2. `/v1/pools/<id>` failing → database or cache problem; follow
   [api.md](api.md#highapierrorrate).
3. `/v1/fees` failing → configuration or contract read problem; follow
   [contract.md](contract.md#contractinteractionfailures).

**Safe mitigation:** As in the cause-level runbook. The price cache keeps
serving its last good value; do not clear it during an upstream outage.

**Verification:** `slo:tx_preparation:error_ratio_rate5m` below 0.005.

**Rollback:** As in the cause-level runbook you followed.

**Escalation:** Not mitigated in 30 minutes → secondary backend-oncall. Any
doubt about amounts shown in the wallet prompt → contract-oncall and
`@Web3Novalabs/maintainers`.

---

## TxPreparationSlowBurn

**Owner:** backend-oncall · **Severity:** warning

**User impact:** Some stake and claim attempts fail or stall before the wallet
prompt.

**Dashboards & queries:** As in
[TxPreparationFastBurn](#txpreparationfastburn), over a 1-day window.

**Diagnosis:** Find which of the three routes contributes most bad requests
over the last day, then its cause (slow pool query, flaky price provider).

**Safe mitigation:** Fix in a normal PR; tune price-cache refresh or add an
index as needed.

**Verification:** `slo:tx_preparation:error_ratio_rate6h` below 0.005.

**Rollback:** Revert the tuning change if it does not help.

**Escalation:** Remaining budget under 25% → apply the budget policy.

---

## IndexerFreshnessFastBurn

**Owner:** backend-oncall · **Severity:** critical

**User impact:** The indexer backlog has been above 50 jobs for a large part of
the last hour. New stakes, pools, and resolutions show up late or not at all.

**Dashboards & queries:**
- `/d/predifi-slo` → *Indexer freshness*.
- `app_worker_queue_depth`, `app_worker_dlq_depth`

**Diagnosis:** Follow [WorkerQueueBacklog](worker.md#workerqueuebacklog) and
check for [WorkerDLQBacklog](worker.md#workerdlqbacklog).

**Safe mitigation:** As in the worker runbooks.

**Verification:** `slo:indexer_freshness:error_ratio_rate5m` is 0 and the
backlog is draining.

**Rollback:** As in the worker runbooks.

**Escalation:** Not mitigated in 30 minutes → secondary backend-oncall;
contract event problems → contract-oncall.

---

## IndexerFreshnessSlowBurn

**Owner:** backend-oncall · **Severity:** warning

**User impact:** Users intermittently see stale data.

**Dashboards & queries:** `avg_over_time(slo:indexer_freshness:stale[1d])` and
`max_over_time(app_worker_queue_depth[1d])`.

**Diagnosis:** Look for a daily pattern (traffic peaks, RPC provider limits).
Recurring short backlogs usually mean the worker's throughput is close to
peak event volume.

**Safe mitigation:** Increase worker throughput or RPC quota in a normal change.

**Verification:** `slo:indexer_freshness:error_ratio_rate6h` below 0.01.

**Rollback:** Revert the throughput/quota change.

**Escalation:** Remaining budget under 25% → apply the budget policy.

---

## SloIndicatorDataMissing

**Owner:** infra-oncall · **Severity:** warning

**User impact:** None directly. SLO dashboards and burn-rate alerts are blind,
so a real outage would not page.

**Dashboards & queries:**
- `absent(app_http_requests_total)`, `absent(app_worker_queue_depth)`
- Prometheus → *Status → Targets* for the `predifi` job.

**Diagnosis:**
1. Is the `predifi` scrape target up? If down, is the backend down (check
   `GET /live`) or is Prometheus unable to reach it (security group, DNS)?
2. Did a release rename or remove a metric? Compare `/metrics` output with the
   names in `predifi_slo_rules.yml`.

**Safe mitigation:**
1. Backend down → page backend-oncall and follow [api.md](api.md#highapierrorrate).
2. Scrape broken → fix the target in `prometheus.yml` (rendered by
   `terraform/modules/monitoring`) or the security group.
3. Metric renamed → restore the old name, or update the rules and this
   runbook in the same PR.

**Verification:** Both `absent()` queries return empty and the SLO panels show data.

**Rollback:** Revert the scrape configuration change.

**Escalation:** Not fixed within one business hour → secondary infra-oncall.
