# Distributed rate limiting

PrediFi's API limits are enforced **across all backend replicas**, not per
process. Every request goes through the middleware in `src/rate_limit.rs`,
which increments a shared Redis counter before the handler runs.

## Partitioning

| Tier | Routes | Key identity | Default limit | Redis outage |
| --- | --- | --- | --- | --- |
| `Light` | `/`, `/health`, `/fees`, `/prices` | client IP | 120 req / 60 s | fail open |
| `Read` | `/pools`, `/stats`, `/leaderboard`, tags, referrals | client IP | 60 req / 60 s | fail open |
| `User` | per-wallet history, predictions, profile, notifications | `x-user-address`, then client IP | 30 req / 60 s | fail open |
| `Write` | `/indexer/*`, pool tag updates | client IP | 20 req / 60 s | **fail closed (503)** |
| `Token` | `/auth/refresh`, `/ws` handshake | client IP | 10 req / 60 s | **fail closed (503)** |
| `WebSocket` | inbound WS messages | connection | 10 msg / 10 s | **fail closed** |

The client IP is taken from the first `X-Forwarded-For` hop, then `X-Real-IP`,
then the socket peer address. Deploy behind a proxy that overwrites these
headers; never trust client-supplied values directly.

The `User` tier prefers the authenticated wallet in `x-user-address` so a user
behind a shared NAT does not consume the whole IP bucket.

## Per-action posting limits (comments and reports)

Comments and reports are governed by `src/abuse.rs` (`AbuseController`),
independently of the HTTP tier limits above. The controller runs in-process and
checks **per-wallet** and **per-network** fixed-window counters before admitting
any post. The client IP is never part of the identity.

| Action | Per-wallet default | Per-network default | Window |
| --- | --- | --- | --- |
| `PostComment` | 20 / 60 s | 500 / 60 s | configurable |
| `PostReport` | 10 / 60 s | 200 / 60 s | configurable |

Additional controls applied before admitting a post:

- **Content-size limit** — comment bodies are capped at 2 000 characters;
  report bodies at 1 000 characters. Over-size content returns
  `reason: "content_too_large"` immediately (no retry delay; the caller
  must shrink the content).
- **Duplicate fingerprint detection** — a SHA-256 fingerprint of the
  normalized content is tracked per wallet for 60 seconds. Identical
  content within that window is rejected with `reason: "duplicate_content"`.
- **Escalating moderation signal** — once a wallet has exhausted more than
  80 % of its per-wallet quota, or submits 3 or more duplicate fingerprints,
  the rejection includes `"escalate": true` to flag the submission for
  moderation review.

Rejection bodies follow the standard error envelope and include:

```json
{
  "status": "error",
  "error": {
    "code": "RATE_LIMITED",
    "message": "<reason> (retry after <N>s)",
    "retryable": true
  }
}
```

The `Retry-After` response header is always set to the same value (minimum 1).
Internal counter values, quota ceilings, Redis keys, and fingerprint state are
**never** included in the rejection body.

### Runtime configuration

| Environment variable | Default | Description |
| --- | --- | --- |
| `PREDIFI_ABUSE_QUOTA_COMMENT_PER_WINDOW` | `20` | Per-wallet comment limit |
| `PREDIFI_ABUSE_QUOTA_NETWORK_COMMENT_PER_WINDOW` | `500` | Per-network comment limit |
| `PREDIFI_ABUSE_QUOTA_REPORT_PER_WINDOW` | `10` | Per-wallet report limit |
| `PREDIFI_ABUSE_QUOTA_NETWORK_REPORT_PER_WINDOW` | `200` | Per-network report limit |
| `PREDIFI_ABUSE_QUOTA_WINDOW_SECS` | `60` | Window length (shared with other actions) |

The per-wallet quota must not exceed the per-network quota; the config
validator rejects the combination before the server starts.

## Redis key and reset semantics

```text
predifi:rate_limit:<tier>:<identity>
```

`identity` is lowercased and sanitized (namespace separators, glob wildcards,
and whitespace are removed). The counter is a fixed window:

1. `INCR` runs atomically in Lua.
2. The first increment sets `PEXPIRE` to the tier window.
3. Concurrent requests on any replica receive the same count and reset time.

The response includes:

- `X-RateLimit-Limit`: the tier burst size.
- `X-RateLimit-Remaining`: remaining requests in the window (0 when denied).
- `X-RateLimit-Reset`: seconds until the window resets.
- `Retry-After`: same value on a 429.

A denied request returns `429 Too Many Requests` with the standard error body
from `src/response.rs`.

## Redis outage policy

The shared counter is a hard dependency for write and authentication traffic.
If Redis is unavailable:

- `Light`, `Read`, `User` **fail open** and serve the request. Availability wins
  for read-only traffic during a Redis incident.
- `Write`, `Token`, `WebSocket` **fail closed** with `503 Service Unavailable`
  and `Retry-After: 1`. Writes and credential operations must not bypass the
  shared limiter.

Set `PREDIFI_REDIS_REQUIRED=true` in production to fail fast at startup instead
of serving a degraded, partially fail-closed API. The default is `false` so
local development works without Redis.

## Configuration

The limits are compiled-in constants in `src/constants.rs` (the values in the
table above). The only runtime configuration is the existing Redis connection:

```dotenv
PREDIFI_REDIS_URL=redis://localhost:6379
PREDIFI_REDIS_REQUIRED=false
```

Changing a tier limit is a code change and must be reflected in the table above
and in the constants' documentation.

## Tests

- `src/rate_limit.rs` unit tests cover the key format, identity extraction,
  tier limits, and fail-open/fail-closed policy.
- `src/abuse.rs` unit tests cover per-wallet burst limits, network-quota
  wallet-rotation resistance, concurrent submission safety (100 threads),
  content-size limits, duplicate detection, escalating moderation signals,
  and retry-metadata opacity (no internal thresholds in rejections).
- `src/redis_integration_tests.rs` contains an ignored test that runs 20
  concurrent requests through two Redis clients (two replicas) and asserts
  exactly five are allowed, the rest are denied, and the window resets:

  ```bash
  cargo test -- --ignored distributed_rate_limit_is_shared_across_replicas_and_resets
  ```

  It requires Docker, like the existing Redis TTL integration tests.


```text
predifi:rate_limit:<tier>:<identity>
```

`identity` is lowercased and sanitized (namespace separators, glob wildcards,
and whitespace are removed). The counter is a fixed window:

1. `INCR` runs atomically in Lua.
2. The first increment sets `PEXPIRE` to the tier window.
3. Concurrent requests on any replica receive the same count and reset time.

The response includes:

- `X-RateLimit-Limit`: the tier burst size.
- `X-RateLimit-Remaining`: remaining requests in the window (0 when denied).
- `X-RateLimit-Reset`: seconds until the window resets.
- `Retry-After`: same value on a 429.

A denied request returns `429 Too Many Requests` with the standard error body
from `src/response.rs`.

## Redis outage policy

The shared counter is a hard dependency for write and authentication traffic.
If Redis is unavailable:

- `Light`, `Read`, `User` **fail open** and serve the request. Availability wins
  for read-only traffic during a Redis incident.
- `Write`, `Token`, `WebSocket` **fail closed** with `503 Service Unavailable`
  and `Retry-After: 1`. Writes and credential operations must not bypass the
  shared limiter.

Set `PREDIFI_REDIS_REQUIRED=true` in production to fail fast at startup instead
of serving a degraded, partially fail-closed API. The default is `false` so
local development works without Redis.

## Configuration

The limits are compiled-in constants in `src/constants.rs` (the values in the
table above). The only runtime configuration is the existing Redis connection:

```dotenv
PREDIFI_REDIS_URL=redis://localhost:6379
PREDIFI_REDIS_REQUIRED=false
```

Changing a tier limit is a code change and must be reflected in the table above
and in the constants' documentation.

## Tests

- `src/rate_limit.rs` unit tests cover the key format, identity extraction,
  tier limits, and fail-open/fail-closed policy.
- `src/redis_integration_tests.rs` contains an ignored test that runs 20
  concurrent requests through two Redis clients (two replicas) and asserts
  exactly five are allowed, the rest are denied, and the window resets:

  ```bash
  cargo test -- --ignored distributed_rate_limit_is_shared_across_replicas_and_resets
  ```

  It requires Docker, like the existing Redis TTL integration tests.
