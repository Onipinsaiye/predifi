# Data Inventory

This document inventories every record type that is linked to a user (or that can be linked to a user) in the platform. It exists so that contributors can consistently implement deletion, retention, and access controls. Each entry records the **owner** (who is responsible for the data), the **purpose** (why it is stored), the **retention** (how long it is kept), and the **deletion path** (how it is removed).

The inventory is split into two top-level categories:

- **Public chain data** — data that originates on-chain and is, by definition, public. The platform indexes and projects it but does not own its confidentiality. Deleting it from the platform does not delete it from the chain.
- **Private platform data** — data created and held by the platform that is not public on-chain. This is the data subject to privacy controls, deletion, and retention limits.

> **Audit requirement:** Every entry below must be traceable to its migration file, OpenAPI schema, and worker payload. See the [Audit Checklist](#audit-checklist) section and the checklist script referenced there.

---

## 1. Public chain data

Public chain data is derived from on-chain events. It is public by construction and is **not** subject to user deletion requests — a user cannot ask the platform to "forget" data that remains permanently visible on the chain. The platform's obligations here are accuracy and provenance, not confidentiality.

| Record type | Owner | Purpose | Retention | Deletion path |
| --- | --- | --- | --- | --- |
| **Pools** (indexed pool records) | Protocol / indexing service | Serve pool metadata and state to the API and UI | Indefinite (mirrors chain state) | Reorg handling / re-index; not user-deletable |
| **Predictions** (indexed prediction records) | Protocol / indexing service | Serve prediction history and results | Indefinite (mirrors chain state) | Reorg handling / re-index; not user-deletable |
| **Referrals** (indexed referral relationships) | Protocol / indexing service | Attribute referrals and compute rewards | Indefinite (mirrors chain state) | Reorg handling / re-index; not user-deletable |
| **Projected contract events** (raw event log projections) | Indexing service | Source of truth for projections; replay and audit | Indefinite (mirrors chain state) | Re-index from chain; not user-deletable |

**Notes**

- Wallet addresses appearing in chain data are pseudonymous public identifiers. They are not treated as private platform data.
- "Projected contract events" are listed here because they are a faithful projection of public chain events. Any *derived, non-public* enrichment of those events belongs in the private section below.
- Reproducibility: public chain-derived history must be reproducible by re-indexing from chain. Retention jobs for private data MUST NOT delete, rewrite, or compact public chain projections. Synthetic-record tests for retention jobs must assert that chain-derived projections are unchanged after a retention run.

---

## 2. Private platform data

Private platform data is created and held by the platform. It is subject to access controls, retention limits, and deletion requests. Each entry names the owning subsystem.

### 2.1 Profile data

| Record type | Owner | Purpose | Retention | Deletion path |
| --- | --- | --- | --- | --- |
| **Profiles** (display name, avatar, bio, linked wallet) | Profile service | Let users present themselves and link a wallet to a profile | Until account deletion | Account deletion endpoint → hard delete of profile row; cascades to dependent private records |

### 2.2 Notification data

| Record type | Owner | Purpose | Retention | Deletion path |
| --- | --- | --- | --- | --- |
| **Notification outbox** (pending notifications) | Notification service | Queue notifications for delivery | Until delivered or expired (short-lived; max 30 days) | Delivered/expired rows purged by retention worker; purged on account deletion |
| **Notification deliveries** (delivery records / status) | Notification service | Track delivery status and support retries | Bounded window of 90 days after delivery | Retention worker purge; purged on account deletion |

### 2.3 Discussion data

| Record type | Owner | Purpose | Retention | Deletion path |
| --- | --- | --- | --- | --- |
| **Discussions** (threads, comments, reactions) | Discussion service | User-generated discussion content | Until content or account deletion | Content deletion endpoint; account deletion cascades to authored content (tombstoned or hard-deleted per policy) |

### 2.4 Tags and claims

| Record type | Owner | Purpose | Retention | Deletion path |
| --- | --- | --- | --- | --- |
| **Tags** (user-applied tags) | Profile / discussion service | Organize and label content | Until removed or account deletion | Remove-tag endpoint; account deletion cascade |
| **Claims** (ownership / verification claims) | Profile service | Verify ownership of a wallet or handle | Until revoked or account deletion; revoked claims retained as audit record for 365 days, then identifiers purged | Revoke-claim endpoint; account deletion cascade |

### 2.5 Export data

| Record type | Owner | Purpose | Retention | Deletion path |
| --- | --- | --- | --- | --- |
| **Exports** (user data export jobs and artifacts) | Export service | Fulfil data-portability requests | Short-lived; artifacts expire after 7-day download window | Expiry worker deletes artifacts; purged on account deletion |

### 2.6 Operational data

| Record type | Owner | Purpose | Retention | Deletion path |
| --- | --- | --- | --- | --- |
| **Idempotency keys** | API gateway / services | Prevent duplicate processing of retried requests | Short TTL (request-scoped, 24 hours) | TTL expiry; not user-addressable |
| **Background jobs** (job payloads) | Worker / job runner | Execute asynchronous work (notifications, exports, indexing) | Until job completion + 7 days | Job completion purge; payloads must not retain private data beyond need |
| **Token revocation** (revoked token records) | Auth service | Enforce logout / revocation of credentials | Until token would have expired | TTL expiry aligned to token lifetime |
| **Audit records** (retained compliance trail) | Platform / compliance | Evidence of deletion requests and retention actions | 365 days; contain request ID, action, timestamp, and a salted hash of the user identifier — no personal fields | Expiry worker after 365 days |

**Notes**

- Operational records (idempotency keys, background jobs, token revocation) are keyed by request or token rather than by user, but may contain user identifiers in payloads. They must be minimized and must not become a shadow store of private data.
- Background job payloads are a common leak vector: payloads should carry identifiers, not full private records, wherever possible.

---

## 3. Data classification summary

| Category | Public? | User-deletable? | Primary control |
| --- | --- | --- | --- |
| Pools, predictions, referrals, projected contract events | Yes (on-chain) | No | Accuracy / provenance; reproducible by re-index |
| Profiles, tags, claims | No | Yes | Access control + deletion cascade |
| Notification outbox / deliveries | No | Yes | Retention worker + deletion cascade |
| Discussions | No | Yes | Content deletion + cascade |
| Exports | No | Yes | Expiry + deletion cascade |
| Idempotency keys, background jobs, token revocation | No | Indirect (TTL) | Minimization + TTL |
| Audit records | No | No (retained for compliance) | Minimized fields + fixed TTL |

### 3.1 Scheduled cleanup

A scheduled retention job runs at least daily and applies the rules in section 2. For each eligible record it:

1. Selects rows whose retention window has elapsed (e.g. delivered notifications older than 30 days, deliveries older than 90 days, exports past their 7-day window, claims revoked more than 365 days ago).
2. Hard-deletes private rows and their dependent private records.
3. Writes one minimized audit record per run (counts and rule IDs only).

The job must never select or modify public chain data categories listed in section 1. Validation: run the job against synthetic private records in a test environment, assert that all expired private rows are removed and unexpired rows remain, and assert that synthetic public chain projections are byte-for-byte unchanged and that re-indexing from the same chain events reproduces them.

---

## 4. Product analytics

Product analytics is a **non-essential** processing purpose. It is off by default and only runs after explicit user consent. The rules below are the authoritative definition used by the analytics client (`frontend/src/lib/analytics/`).

### 4.1 Event categories

| Category | Examples | Contains user-linked identifiers? | Retention |
| --- | --- | --- | --- |
| `navigation` | Page views, route changes | No (route name only) | 30 days |
| `feature_usage` | Button/feature interactions, counts | No | 30 days |
| `performance` | Load timings, render errors (no stack payloads with user data) | No | 14 days |

### 4.2 Prohibited data

Analytics payloads must never contain:

- private keys, seed phrases, or signing material
- transaction signatures or signed payloads
- position-level values (stake amounts, prediction sizes, balances tied to a user)
- wallet addresses or other direct identifiers

The client enforces this by dropping any property whose key matches a denylist pattern and by never emitting position-level fields.

### 4.3 Consent model

- Default state is **denied**: no analytics event is sent until the user grants consent.
- Consent is stored locally and can be withdrawn at any time; withdrawal stops all future collection immediately and clears any queued events.
- Essential, non-analytics operations are unaffected by consent state.

---

## Audit Checklist

Use this checklist to verify that each inventory entry is backed by a real migration, a documented API schema, and a worker payload definition. Run the checklist script (added in the follow-up step, referenced below) to automate the cross-reference; the table records the expected mapping.

**Checklist script:** `scripts/audit-data-inventory.*` (added in the next step). The script cross-references each row below against the migrations directory, the OpenAPI spec, and worker payload definitions, and fails if any entry is missing a mapping.

### Public chain data

| Inventory entry | Migration file | OpenAPI schema | Worker payload |
| --- | --- | --- | --- |
| Pools | `migrations/*_pools.*` | `Pool` schema | Indexer projection payload |
| Predictions | `migrations/*_predictions.*` | `Prediction` schema | Indexer projection payload |
| Referrals | `migrations/*_referrals.*` | `Referral` schema | Indexer projection payload |
| Projected contract events | `migrations/*_contract_events.*` | (internal; not exposed) | Indexer event payload |

### Private platform data

| Inventory entry | Migration file | OpenAPI schema | Worker payload |
| --- | --- | --- | --- |
| Profiles | `migrations/*_profiles.*` | `Profile` schema | — |
| Notification outbox | `migrations/*_notification_outbox.*` | (internal) | Notification enqueue payload |
| Notification deliveries | `migrations/*_notification_deliveries.*` | (internal) | Notification delivery payload |
| Discussions | `migrations/*_discussions.*` | `Discussion` / `Comment` schemas | — |
| Tags | `migrations/*_tags.*` | `Tag` schema | — |
| Claims | `migrations/*_claims.*` | `Claim` schema | — |
| Exports | `migrations/*_exports.*` | `Export` schema | Export job payload |
| Idempotency keys | `migrations/*_idempotency_keys.*` | (internal) | — |
| Background jobs | `migrations/*_jobs.*` | (internal) | Job envelope payload |
| Token revocation | `migrations/*_token_revocation.*` | (internal) | — |
| Audit records | `migrations/*_audit_records.*` | (internal) | Retention job audit payload |

### Review steps

1. **Migrations** — Confirm each entry has a corresponding migration and that column-level comments classify fields as public or private.
2. **API schemas** — Confirm each user-facing entry appears in the OpenAPI spec and that private fields are not over-exposed.
3. **Worker payloads** — Confirm each worker payload is minimized (identifiers over full records) and that retention is enforced.
4. **Deletion paths** — Confirm each private entry has a working deletion path (endpoint, cascade, or TTL) and that the path is covered by a test.
5. **Retention job tests** — Confirm the scheduled cleanup is exercised against synthetic private records, and that public chain-derived projections remain unchanged and reproducible after each run.
6. **Run the checklist script** — Execute `scripts/audit-data-inventory.*` and resolve any reported gaps before merging changes that add or alter user-linked records.

When you add a new user-linked record type, add a row to the appropriate table above **and** a row to the audit checklist, then run the checklist script.
