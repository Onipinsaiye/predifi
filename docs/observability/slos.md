# Service-Level Indicators, Objectives, and Error Budgets

Process uptime says the backend is running. It does not say whether users can
load pools, see fresh on-chain data, or get to a wallet prompt. This document
defines the indicators (SLIs) that do, the targets (SLOs) we hold ourselves to,
and what we do when a target is at risk (budget policy).

Implementation:

| Piece | Location |
|---|---|
| Recording rules and burn-rate alerts | [`backend/grafana/alerts/predifi_slo_rules.yml`](../../backend/grafana/alerts/predifi_slo_rules.yml) |
| Dashboard (`/d/predifi-slo`) | [`backend/grafana/dashboards/predifi_slo.json`](../../backend/grafana/dashboards/predifi_slo.json) |
| Runbooks for every SLO alert | [`docs/runbooks/slo.md`](../runbooks/slo.md) |

Change a target here and in the rules file in the same PR. Reviewers should
reject a PR that changes one without the other.

## Principles

- **User-centred.** Each SLI measures something a user would notice: a failed
  page, a slow page, stale on-chain data, a stake that cannot start.
- **Ratio of good events to valid events.** Every SLI is `good / valid` so the
  error budget is simply `1 − target`.
- **Exclude what users never call.** `/metrics`, `/health*`, `/live*`,
  `/ready*`, `/healthz`, `/readyz` are excluded from API SLIs; probes would
  otherwise inflate availability.
- **28-day rolling window** for every SLO. Long enough to smooth noise, short
  enough to react to within a sprint.

## Critical services and their SLOs

| SLO id | Service | SLI (good / valid) | Target | Budget (28d) |
|---|---|---|---|---|
| `api-availability` | Public API | Non-5xx responses / all responses on user-facing routes | **99.5%** | 0.5% of requests (~3h 22m of full outage) |
| `api-latency` | Public API | Responses served in ≤ 500ms / all responses on user-facing routes | **99.0%** | 1% of requests |
| `tx-preparation` | Transaction preparation | `/v1/fees`, `/v1/prices`, `/v1/pools/<id>` responses that are non-5xx **and** ≤ 1s / all such responses | **99.5%** | 0.5% of requests |
| `indexer-freshness` | Indexing | Minutes in which the indexer backlog is ≤ 50 jobs / all minutes | **99.0%** | ~6h 43m of stale minutes |

### Why these indicators

**API availability.** A 5xx is a request the user made that we failed. 4xx is
excluded because it is usually the client's input (bad address, unknown pool).

**API latency.** 500ms is the point at which the existing `HighP99Latency`
alert considers the app sluggish, and the bucket boundary exists in
`app_http_request_duration_seconds`. Measuring "share of requests under
500ms" rather than "p99 under 500ms" makes the budget additive and
comparable across windows.

**Transaction preparation.** Before a user signs a stake or a claim, the
frontend needs the current fee, prices, and the target pool. If any of those
fails or stalls, the user never reaches the wallet prompt. This is the
backend's share of the transaction path; Soroban RPC simulation and wallet
signing happen client-side and are outside the backend's control. The 1s
threshold is looser than the general API because users tolerate a short wait
right before signing, but not a failure.

**Indexer freshness.** Users judge the indexer by whether a stake they just
made shows up. The direct measure would be *seconds between a ledger closing
and the backend having indexed it*. That gauge does not exist yet, so the SLI
uses the worker backlog as a proxy: a backlog above 50 jobs (the existing
`WorkerQueueBacklog` threshold) means new events wait behind old ones.

**Instrumentation follow-up:** add an
`app_indexer_last_indexed_ledger_close_time_seconds` gauge and switch the
freshness SLI to `time() − gauge ≤ 30s`. Also wire
`Metrics::record_worker_queue_depth` into the worker loop — today it is only
called from tests, so the proxy reads 0 (see
[runbook known gaps](../runbooks/README.md#known-gaps)).
`SloIndicatorDataMissing` pages if the inputs disappear entirely.

## Alerting: multi-window, multi-burn-rate

A burn rate of 1 spends exactly the whole budget in 28 days. Each SLO has two
alerts; each fires only when both a long and a short window agree, so it
triggers quickly on real problems and resets quickly after recovery.

| Alert | Burn rate | Long window | Short window | Budget spent when it fires | Severity |
|---|---|---|---|---|---|
| `*FastBurn` | 14.4× | 1h | 5m | 2% | critical (page) |
| `*FastBurn` | 6× | 6h | 30m | 5% | critical (page) |
| `*SlowBurn` | 3× | 1d | 2h | 10% | warning (ticket) |
| `*SlowBurn` | 1× | 3d | 6h | 10% | warning (ticket) |

Thresholds per SLO (error ratio that triggers the alert):

| SLO | Budget | 14.4× | 6× | 3× | 1× |
|---|---|---|---|---|---|
| `api-availability` | 0.005 | 7.2% | 3.0% | 1.5% | 0.5% |
| `api-latency` | 0.01 | 14.4% | 6.0% | 3.0% | 1.0% |
| `tx-preparation` | 0.005 | 7.2% | 3.0% | 1.5% | 0.5% |
| `indexer-freshness` | 0.01 | 14.4% | 6.0% | 3.0% | 1.0% |

The existing threshold alerts (`HighApiErrorRate`, `HighP99Latency`,
`WorkerQueueBacklog`, …) stay in place as cause-level signals. Each burn-rate
alert's runbook points to the cause-level runbook to use.

## Error budget policy

Remaining budget is recorded as `slo:<sli>:error_budget_remaining` and shown
on `/d/predifi-slo`.

| Remaining budget (28d) | What happens |
|---|---|
| **> 50%** | Normal. Ship features at the usual pace. |
| **25–50%** | The owning team reviews the top budget consumers in the weekly sync and files fixes. |
| **0–25%** | Changes to the affected service need a second reviewer from the owning team and a rollback plan in the PR description. Reliability work for that service takes priority over new features. |
| **Exhausted (≤ 0)** | Feature freeze for the affected service: only fixes, security patches, and changes that reduce risk ship until the 28-day budget recovers above 25%. The owning team writes a short post-incident review for the largest contributor. |

Exceptions (for example a security fix that must ship during a freeze) are
approved by `@Web3Novalabs/maintainers` and noted in the PR.

Budget spent by a dependency outage outside our control (for example a Soroban
RPC provider incident) still counts. The point is to protect users; if a
dependency regularly burns our budget, the fix is redundancy, not an
exception.

## Ownership and review

| SLO | Owner | Review |
|---|---|---|
| `api-availability`, `api-latency`, `tx-preparation`, `indexer-freshness` | `backend-oncall` (`@Web3Novalabs/backend-reviewers`) | Monthly: compare attainment to target, adjust only with data |

## SLO review exercise

Validate the definitions with a representative incident before relying on them:

1. Pick an incident (real or [tabletop](../runbooks/TABLETOP.md) scenario 2 or 3).
2. For each SLO, write down: did the SLI move? Did the right burn-rate alert
   fire, and how soon? How much budget did the incident spend?
3. If an incident users noticed did not move any SLI, the SLIs are missing
   something — add or adjust one. If an SLI moved but users did not notice,
   the target may be too strict.
4. Record the outcome below.

| Date | Scenario | SLIs affected | Alert fired (time to fire) | Budget spent | Changes made |
|---|---|---|---|---|---|
| | | | | | |
