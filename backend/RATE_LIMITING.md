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
- `src/redis_integration_tests.rs` contains an ignored test that runs 20
  concurrent requests through two Redis clients (two replicas) and asserts
  exactly five are allowed, the rest are denied, and the window resets:

  ```bash
  cargo test -- --ignored distributed_rate_limit_is_shared_across_replicas_and_resets
  ```

  It requires Docker, like the existing Redis TTL integration tests.
