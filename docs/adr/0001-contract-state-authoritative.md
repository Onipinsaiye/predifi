# ADR-0001: Treat contract state as authoritative and backend data as a projection

- **Status:** Accepted
- **Date:** 2026-10-18
- **Decision owners:** Contract and backend maintainers
- **Linked implementation issues:** [#32](https://github.com/Onipinsaiye/predifi/issues/32), [#121](https://github.com/Onipinsaiye/predifi/issues/121), [#140](https://github.com/Onipinsaiye/predifi/issues/140)
- **Linked implementation PRs:** <!-- Add PR links as the event/indexing work lands. -->

## Context

PrediFi spans the Soroban contract, the Rust backend/indexer, and the Next.js
frontend. The contract executes signed writes and owns pool lifecycle, stakes,
resolution, payouts, and authorization. The backend consumes contract events
and stores query-friendly records in Postgres and Redis; the frontend consumes
those records for fast views while also submitting signed transactions.

These stores can temporarily disagree because event delivery, indexing, cache
refreshes, or RPC access can be delayed. Treating a backend record as settlement
truth would allow a stale or incomplete projection to drive claims, balances, or
resolution decisions. Contract interface changes also affect event consumers,
so event topics and payload meaning must remain compatible or be versioned and
migrated deliberately.

## Decision

The Soroban contract is the authoritative source for pool state, stakes,
resolution, claims, payouts, and authorization. Backend/Postgres/Redis data is a
recoverable, event-derived projection used for queryability, indexing,
notifications, and operational visibility. Frontend persistence and displayed
cache data are convenience state only.

All settlement-sensitive flows must verify the relevant contract state before
acting. Indexers must consume stable contract events, process them so that
replays are safe, and surface gaps or conflicting observations instead of
silently overwriting on-chain truth. A contract event schema change is therefore
a compatibility change: it requires an explicit versioning or migration plan,
updated consumers, and regression evidence before release.

When a projection disagrees with the contract, clients and operators must prefer
the contract, mark the projection stale, and repair or replay the affected event
range. The backend must not invent a settlement outcome from missing events or
from a cache value that has not been reconciled with the chain.

## Alternatives considered

### Alternative A: Make Postgres the system of record

This would make API reads simple and reduce repeated RPC calls, but it would
move settlement authority off-chain and create a dangerous failure mode when
indexing lags, events are missed, or the database is restored from an older
backup. It is rejected for protocol and financial state.

### Alternative B: Read the contract for every frontend query

This would minimize projection staleness, but it would make common list,
search, analytics, and notification queries expensive and difficult to paginate.
It also would not remove the need for an indexer to observe and monitor events.
It is rejected as the default read path, while remaining appropriate for
settlement-sensitive verification and reconciliation.

### Alternative C: Let the latest observation win

This would hide discrepancies and make recovery difficult: a stale cache or
malformed event could overwrite a newer on-chain value. It is rejected because
monotonic event processing and explicit conflict handling provide safer
operational behavior.

## Consequences

### Positive

- Contract compatibility and financial correctness have a clear authority
  boundary.
- Backend and frontend outages, stale caches, and indexer replay do not change
  settlement outcomes.
- Postgres and Redis can be rebuilt from contract events and reconciled against
  RPC state.
- Stable events become an explicit compatibility surface for indexers and other
  off-chain consumers.

### Tradeoffs and risks

- Settlement-sensitive paths require an RPC/contract read or equivalent verified
  state, adding latency and dependency on chain availability.
- Indexers need replay-safe processing, gap detection, and monitoring for stale
  projections.
- Event/schema changes require coordinated contract, backend, and frontend work
  rather than an isolated deployment.

### Follow-up

- Keep event topics and payload semantics documented and regression-tested under
  [#32](https://github.com/Onipinsaiye/predifi/issues/32) and
  [#121](https://github.com/Onipinsaiye/predifi/issues/121).
- Use the [architecture overview](../ARCHITECTURE_OVERVIEW.md) and
  [contract versioning policy](../contract-versioning.md) when changing the
  authority boundary or a public contract surface.
- Add implementation PR links above as the compatibility and reconciliation
  work is completed.
