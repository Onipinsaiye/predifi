# Alert Runbooks

Every production alert links here through its `runbook_url` annotation. A
runbook answers five questions for the person who was just paged:

1. **What are users experiencing?** (user impact)
2. **How do I confirm it and find the cause?** (dashboards, queries, diagnosis)
3. **What can I safely do right now?** (mitigation that cannot make things worse)
4. **How do I know it worked, and how do I undo it?** (verification and rollback)
5. **Who do I call if I'm stuck?** (escalation)

An operator should be able to resolve a simulated incident using only the
linked runbook. If you needed anything else during an incident, add it here
afterwards.

## Alert inventory

Source files:

| File | Evaluated by |
|---|---|
| [`backend/grafana/alerts/predifi_alerts.yml`](../../backend/grafana/alerts/predifi_alerts.yml) | Prometheus (deployed by `terraform/modules/monitoring`) |
| [`backend/grafana/alerts/predifi_slo_rules.yml`](../../backend/grafana/alerts/predifi_slo_rules.yml) | Prometheus (SLO recording rules and burn-rate alerts) |
| [`docker/grafana/provisioning/alerting/rules.yaml`](../../docker/grafana/provisioning/alerting/rules.yaml) | Grafana unified alerting (local / compose stack) |
| [`docker/loki/rules/fake/predifi-log-alerts.yml`](../../docker/loki/rules/fake/predifi-log-alerts.yml) | Loki ruler |

| Alert | Severity | Owner | Runbook |
|---|---|---|---|
| `HighApiErrorRate` | critical | backend-oncall | [api.md](api.md#highapierrorrate) |
| `HighP99Latency` | warning | backend-oncall | [api.md](api.md#highp99latency) |
| `WorkerQueueBacklog` | warning | backend-oncall | [worker.md](worker.md#workerqueuebacklog) |
| `WorkerDLQBacklog` | critical | backend-oncall | [worker.md](worker.md#workerdlqbacklog) |
| `DatabaseConnectionPoolSaturated` | warning | backend-oncall | [database.md](database.md#databaseconnectionpoolsaturated) |
| `DatabaseConnectionPoolCritical` | critical | backend-oncall | [database.md](database.md#databaseconnectionpoolcritical) |
| `DatabaseQueryFailures` | critical | backend-oncall | [database.md](database.md#databasequeryfailures) |
| `HighRedisErrors` | warning | infra-oncall | [cache.md](cache.md#highrediserrors) |
| `ContractInteractionFailures` | critical | contract-oncall | [contract.md](contract.md#contractinteractionfailures) |
| `BackendMemoryHigh` | critical | infra-oncall | [host.md](host.md#backendmemoryhigh) |
| `BackendErrorLogSpike` | critical | backend-oncall | [logs.md](logs.md#backenderrorlogspike) |
| `BackendPanic` | critical | backend-oncall | [logs.md](logs.md#backendpanic) |
| `DatabaseErrorLogs` | warning | backend-oncall | [logs.md](logs.md#databaseerrorlogs) |
| `NoBackendLogs` | warning | infra-oncall | [logs.md](logs.md#nobackendlogs) |
| `ApiAvailabilityFastBurn` | critical | backend-oncall | [slo.md](slo.md#apiavailabilityfastburn) |
| `ApiAvailabilitySlowBurn` | warning | backend-oncall | [slo.md](slo.md#apiavailabilityslowburn) |
| `ApiLatencyFastBurn` | critical | backend-oncall | [slo.md](slo.md#apilatencyfastburn) |
| `ApiLatencySlowBurn` | warning | backend-oncall | [slo.md](slo.md#apilatencyslowburn) |
| `TxPreparationFastBurn` | critical | backend-oncall | [slo.md](slo.md#txpreparationfastburn) |
| `TxPreparationSlowBurn` | warning | backend-oncall | [slo.md](slo.md#txpreparationslowburn) |
| `IndexerFreshnessFastBurn` | critical | backend-oncall | [slo.md](slo.md#indexerfreshnessfastburn) |
| `IndexerFreshnessSlowBurn` | warning | backend-oncall | [slo.md](slo.md#indexerfreshnessslowburn) |
| `SloIndicatorDataMissing` | warning | infra-oncall | [slo.md](slo.md#sloindicatordatamissing) |

CI (`.github/workflows/alert-runbooks.yml`) fails if any rule in the files above
is missing an allowed `owner`, a valid `severity`, an `impact`, a
`dashboard_url`, or a `runbook_url` that resolves to a heading in this
directory containing every required runbook field. There is no such thing as an
unowned alert.

## Owners

The `owner` label routes the page. Each owner maps to a CODEOWNERS team that is
accountable for the runbook staying correct.

| `owner` label | Covers | Accountable team | Escalate to |
|---|---|---|---|
| `backend-oncall` | API, indexer/worker, database usage, application logs | `@Web3Novalabs/backend-reviewers` | `@Web3Novalabs/maintainers` |
| `infra-oncall` | Hosts, Redis, monitoring pipeline, Terraform-managed resources | `@Web3Novalabs/infra-reviewers` | `@Web3Novalabs/maintainers` |
| `contract-oncall` | Soroban contract interaction and on-chain state | `@Web3Novalabs/contract-reviewers` | `@Web3Novalabs/maintainers` |

To add an owner, add it to this table **and** to `ALLOWED_OWNERS` in
`.github/scripts/validate_alert_runbooks.py` in the same pull request.

## Severity

| Severity | Meaning | Response |
|---|---|---|
| `critical` | Users are affected now, or will be within minutes. | Page the owner. Acknowledge within 5 minutes; start mitigation within 15. |
| `warning` | Risk is building, or a small share of users are affected. | Notify the owner's channel. Look at it within one business hour. |

## Escalation

1. **Primary on-call** for the alert's `owner` acknowledges and follows the runbook.
2. If not mitigated within **30 minutes** (critical) or **1 business day**
   (warning), page the **secondary on-call** for the same owner.
3. If two owners are involved, or a critical alert is still firing after
   **60 minutes**, the secondary declares an incident and asks the escalation
   team in the table above to name an incident commander.
4. Anything touching funds, contract state, or user keys goes straight to
   `contract-oncall` and `@Web3Novalabs/maintainers`, regardless of the alert's owner.

Record every page in the incident log: alert, start/ack/mitigate/resolve times,
what you did, and anything missing from the runbook.

## Dashboards

`dashboard_url` annotations are Grafana paths. Prefix them with your Grafana
host (for example `https://grafana.example.com/d/predifi-backend-prod`).

| Path | Dashboard | Source |
|---|---|---|
| `/d/predifi-backend-prod` | PrediFi Backend — Production Observability | `backend/grafana/dashboards/predifi_backend.json` |
| `/d/predifi-slo` | PrediFi SLOs & Error Budgets | `backend/grafana/dashboards/predifi_slo.json` |
| `/d/predifi-logs` | PrediFi logs (Loki) | `docker/grafana/dashboards/predifi-logs.json` |

## Known gaps

These were found while writing the runbooks. They do not block using the
runbooks, but the affected alerts cannot fire until fixed:

- `db_pool_utilization_ratio` is documented in `backend/src/db/README.md` but is
  not registered in `backend/src/metrics.rs`, so the two connection-pool alerts
  currently have no data.
- `Metrics::record_worker_queue_depth` is only called from tests, so
  `app_worker_queue_depth` / `app_worker_dlq_depth` stay at 0 in production.
  The worker alerts and the indexer-freshness SLO depend on it being wired into
  the worker loop.
- There is no HTTP endpoint for replaying dead-lettered jobs; replay requires
  `JobQueue::requeue_from_dlq` from an operator binary or a restart-time hook
  (see [worker.md](worker.md#workerdlqbacklog)).

## Runbook format

Each alert gets a `## <AlertName>` heading (GitHub turns it into the
lowercase anchor used by `runbook_url`) followed by these fields. The CI check
looks for each bold label:

```markdown
## AlertName

**Owner:** backend-oncall · **Severity:** critical

**User impact:** what users see.

**Dashboards & queries:** panel links and copy-pasteable PromQL/LogQL.

**Diagnosis:**
1. ...

**Safe mitigation:**
1. ...

**Verification:** how to confirm the mitigation worked.

**Rollback:** how to undo the mitigation if it made things worse.

**Escalation:** who to call next, and when.
```

Practise with [TABLETOP.md](TABLETOP.md).
