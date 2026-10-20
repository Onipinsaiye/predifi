# Health Check Endpoints

This document describes the health check endpoints implemented in
`predifi-backend`. The service exposes two distinct contracts:

- **Liveness** -- is the process alive?
- **Readiness** -- can this instance currently serve traffic?

Keeping these separate prevents restart loops when a downstream dependency
(Postgres, Redis, Soroban RPC) is temporarily unavailable: the process is
still healthy, so it is removed from service rotation but not restarted.

---

## Endpoints

### Liveness

```
GET /health/live
```

Answers whether the process is running and able to handle requests. It must
not contact any external dependency, so its latency is bounded by the handler
only.

Response (HTTP 200):

```json
{
  "status": "ok",
  "service": "predifi-backend",
  "version": "0.1.0"
}
```

- `status` -- always `"ok"` when the process is running
- `service` -- name of the service
- `version` -- current version from Cargo.toml

### Readiness

```
GET /health/ready
```

Reports whether this instance can serve traffic by probing each required
dependency. Response body:

```json
{
  "status": "ready",
  "service": "predifi-backend",
  "version": "0.1.0",
  "dependencies": {
    "postgres": { "status": "up", "latency_ms": 2 },
    "redis": { "status": "up", "latency_ms": 1 },
    "soroban_rpc": { "status": "up", "latency_ms": 12 }
  }
}
```

- `status` -- `"ready"` when all dependencies are up, otherwise `"not_ready"`
- `dependencies.*.status` -- `up` or `down`
- `dependencies/*.latency_ms` -- measured probe latency in milliseconds

HTTP status codes:

| Condition                            | Status Code |
|--------------------------------------|-------------|
| All dependencies up                    | 200           |
| Any dependency down                     | 503           |
| Startup grace period not yet elapsed    | 503           |

The readiness handler always returns a body and a status code; it never
panics or blocks indefinitely.

---

## Contracts

### Liveness contract

- Must return 200 whenever the process is running.
- Must not contact Postgres, Redis, or Soroban RPC.
- Must respond within a bounded time (the handler is purely in-memory).
- Failure to respond is the only signal to restart the process.

### Readiness contract

- Must probe Postgres, Redis, and Soroban RPC independently.
- Must return 200 only when all dependencies are up.
- Must return 503 when any dependency is down or the startup grace hasn't elapsed.
- Must never restart the process; a not-ready signal only removes the
instance from service rotation.
- Must respond within the dependency timeout budget described below.

---

## Dependency Timeout Budget

Each dependency probe is bounded by a per-dependency timeout. Probes run
in parallel, so the total readiness latency is bounded by the largest budget
rather than the sum.

| Dependency     | Probe                             | Timeout budget |
|----------------|-------------------------------------|----------------|
| Postgres       | `SELECT 1`                            | 500 ms         |
| Redis          | `PING`                                | 500 ms         |
| Soroban RPC    | `getHealth`                          | 1000 ms        |

Total readiness latency is therefore bounded at approximately 1000 ms even when
all dependencies are down. Timeouts are reported as `down` with the measured
latency capped at the budget.

---

## Startup Grace

During the startup grace period (default 10 seconds), the readiness endpoint
returns 503 with `status: "not_ready"` regardless of dependency state. This
allows the process to complete initialization and connection pool warmup
before receiving traffic. Liveness is unaffected by the grace period.

---

## Implementation

**File:** `src/main.rs`

### `liveness()` handler
An async Axum handler that returns HTTP 200 with basic system info.
The version is read at compile time from `Cargo.toml` using
denv!("CARGO_PKG_VERSION")` -- no runtime overhead.

### `readiness()` handler
An async Axum handler that reuns the Postgres, Redis, and Soroban RPC probes
in parallel, with per-dependency timeouts as described above. It aggregates
the results into the response body and selects 200 or 503 accordingly.

---

## Test Coverage

**File:** `src/tests.rs`

| Test                                         | What it verifies                                  |
|------------------------------------------------|------------------------------------------------|
| `liveness_returns_200_with_ok_body`               | Liveness returns HTTP 200 with status field           |
| `liveness_returns_system_info`                  | Liveness returns service name and version           |
| `middleware_does_not_alter_200_status`         | Middleware doesn't change 200 response              |
| `readiness_returns_200_when_all_deps_up`       | Readiness returns 200 when all dependencies are up     |
| `readiness_returns_503_when_postgres_down`       | Postgres failure alone returns 503                     |
| `readiness_returns_503_when_redis_down`           | Redis failure alone returns 503                         |
| `readiness_returns_503_when_soroban_rpc_down`    | Soroban RPC failure alone returns 503                  |
| `readiness_response_latency_is_bounded`        | Readiness responds within the dependency budget      |
| `readiness_returns_503_during_startup_grace`     | Readiness returns 503 during the startup grace         |

---

## Example Usage

```bash
curl -i http://localhost:3000/health/live
curl -i http://localhost:3000/health/ready
```

Expected liveness response:
```json
{"status":"ok","service":"predifi-backend","version":"0.1.0"}
```

Expected readiness response when all dependencies are up:
```json
{"status":"ready","service":"predifi-backend","version":"0.1.0","dependencies":{"postgres":{"status":"up","latency_ms":2},"redis":{"status":"up","latency_ms":1},"soroban_rpc":{"status":"up","latency_ms":12}}}
```

---

## Deployment Probe Configuration

Liveness probes must target `/health/live` and readiness probes must target
`/health/ready`. Do not point liveness at `/health/ready` -- doing so will
cause restart loops during dependency outages.

### Kubernetes

```yaml
livenessProbe:
  httpGet:
    path: /health/live
    port: 3000
  initialDelaySeconds: 5
  periodSeconds: 10
  timeoutSeconds: 2
  failureThreshold: 3

readinessProbe:
  httpGet:
    path: /health/ready
    port: 3000
  initialDelaySeconds: 10
  periodSeconds: 5
  timeoutSeconds: 2
  failureThreshold: 2

startupProbe:
  httpGet:
    path: /health/live
    port: 3000
  periodSeconds: 2
  failureThreshold: 15
```

The readiness probe `timeoutSeconds` is set higher than the total readiness
latency budget (1000 ms) so a slowdown in a dependency is reported as not ready
rather than as a probe timeout.

### Docker Compose
```yaml
healthcheck:
  test: ["CMD", "curl", "-f", "http://localhost:3000/health/live"]
  interval: 10s
  timeout: 2s
  retries: 3
```

### AWS ALB
```
LivenessCheck:
  Path: /health/live
  Matcher:
    HttpCode: 200
  IntervalSeconds: 10
  TimeoutSeconds: 2
  HealthyThresholdCount: 2
  UnhealthyThresholdCount: 3

HealthCheck:
  Path: /health/ready
  Matcher:
    HttpCode: 200
  IntervalSeconds: 5
  TimeoutSeconds: 2
  HealthyThresholdCount: 2
  UnhealthyThresholdCount: 2
```
