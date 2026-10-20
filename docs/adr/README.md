# Architecture Decision Records

Architecture Decision Records (ADRs) preserve decisions that affect more than one
PrediFi component. They explain why a choice was made so that future contract,
backend, frontend, and operations changes can be reviewed against the same
constraints.

## When an ADR is required

Write or update an ADR when a change can affect any of the following:

- contract call, event, error, storage, settlement, or upgrade compatibility;
- oracle trust, resolution authority, or verification assumptions;
- consistency between on-chain state and backend/frontend projections; or
- deployment, migration, rollback, or operational responsibilities across components.

An ADR is not required for an isolated implementation detail, a documentation
correction, or a change that follows an existing decision without changing its
tradeoffs. If the boundary is unclear, open a draft ADR and ask maintainers
whether it should be kept.

## ADR workflow

1. Copy [`TEMPLATE.md`](./TEMPLATE.md) to the next available four-digit number,
   for example `0002-short-title.md`.
2. Set the status to `Proposed` while the decision is under review. Link the
   issue(s) that motivated the work and any implementation PRs.
3. Ask the owners of every affected area to review the context, alternatives,
   and consequences. A decision spanning contract and backend code needs both
   areas represented in review.
4. Change the status to `Accepted` only when maintainers agree. Keep the ADR in
   place after implementation; supersede it with a new ADR rather than
   rewriting history when the decision changes.
5. Link the ADR from relevant architecture, compatibility, deployment, or API
   documentation when it is useful to readers outside the review.

## Status values

Use `Proposed`, `Accepted`, `Rejected`, `Deprecated`, or `Superseded by ADR-NNNN`.

## Index

| ADR | Decision | Status |
| --- | --- | --- |
| [ADR-0001](./0001-contract-state-authoritative.md) | Treat Soroban contract state as authoritative; expose backend data as an event-derived projection | Accepted |
