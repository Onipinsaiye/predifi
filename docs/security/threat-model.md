# Threat Model

**Issue:** #93

**Last reviewed:** 2026-10-21

This document documents a practical protocol threat model across Soroban contracts, backend/indexer, wallet clients, and oracle providers for PrediFi. It identifies assets, actors, trust boundaries, attack paths, mitigations, and residual risks. Mitigations that are already implemented in code are distinguished from recommendations for future work.

---

## 1. Assets and Security Objectives

| Asset                               | Protection Objective                                                                         |
| ----------------------------------- | -------------------------------------------------------------------------------------------- |
| User stakes and prediction outcomes | Prevent unauthorized withdrawal or alteration; ensure payouts match resolved outcomes        |
| Oracle price data                   | Prevent manipulation, staleness, or substitution that could alter resolution results         |
| Admin/operator keys                 | Prevent unauthorized configuration changes (pauses, fee changes, oracle whitelisting)        |
| Backend indexer consistency         | Ensure indexed state mirrors contract state; prevent undetected divergence                   |
| Wallet transaction integrity        | Ensure user-signed transactions cannot be forged or tampered with in transit                 |
| Dependency integrity                | Prevent compromised third-party packages or container images from altering protocol behavior |

Security objectives:

- **Confidentiality**: Protect sensitive configuration and user data from unauthorized disclosure.
- **Integrity**: Ensure contract state, price data, and payout results are resistant to undetected modification.
- **Availability**: Ensure markets can operate and users can stake/claim without being permanently locked out.
- **Non-repudiation**: Wallet signatures and admin actions should be cryptographically verifiable.

---

## 2. Actors, Trust Boundaries, and Assumptions

| Actor                  | Role                                                | Trust Boundary                                                 | Key Assumptions                                                                                      |
| ---------------------- | --------------------------------------------------- | -------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| **User**               | Connects wallet (Freighter), stakes, claims         | User controls private key; signs transactions locally          | User does not reveal private key; runs up-to-date wallet software                                    |
| **Admin**              | Configures pools, oracle, fees, pauses              | Controls the admin key; exercises privileged functions         | Admin key is stored securely; admin acts in protocol's best interest                                 |
| **Operator**           | Assists admin with certain config changes           | Operates with operator key/role; limited privileged functions  | Operator key is separate from admin key; operator acts responsibly                                   |
| **Oracle Keeper**      | Pushes price data to Pyth contract                  | Whitelisted oracle address; signs price updates                | Oracle keeper operator is trusted; Pyth network data feeds are accurate                              |
| **Backend API**        | Exposes REST endpoints, runs indexer, caches prices | Binds to network; reads from Stellar RPC, writes to PostgreSQL | Stellar RPC endpoints are reliable; PostgreSQL data is durable; Redis cache is eventually consistent |
| **Indexer**            | Processes contract events into PostgreSQL           | Reads from Soroban RPC, writes to DB, updates Redis cache      | Event processing is ordering-correct; Redis and DB are in sync                                       |
| **Soroban Validators** | Execute contracts, produce ledgers                  | Network consensus; finality                                    | > 2/3 of validators are honest (SCP); network is not under adversarial control                       |

Trust boundaries:

- **Contract ⇄ Backend**: Oracle price feeds flow from contract to backend via RPC; backend writes to DB. Divergence possible if RPC fails or indexer lags.
- **Backend ⇄ Frontend**: Price API and pool data served via REST/GraphQL; frontend caches may be stale.
- **Wallet ⇄ Frontend**: Freighter/XDR signing; user approves each transaction. No third-party signing.
- **Admin ⇄ Contract**: `require_auth()` + `require_admin_role()` gates all privileged operations.

Assumptions:

- Stellar Consensus Protocol (SCP) provides finality and prevents unilateral reordering.
- Pyth network provides accurate price data with confidence intervals.
- CoinGecko price feed is used as a secondary reference for the backend price cache.
- User wallets follow SEP-10 authentication and do not leak session keys.
- Admin and operator keys are stored in hardware security modules or otherwise protected environments.

---

## 3. Attack Paths

### 3.1 Front-Running / Transaction Ordering

| Path                                                                                                                                | Likelihood                                                                   | Impact                                                                                                  | Mitigations (existing)                                                                                                                                                                                                                                                                                                                                                                        | Responsible Component                                                                                                                                                            |
| ----------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Observer sees a pending prediction transaction and submits a competing prediction with a fee bump to land in the same ledger close. | Low on Stellar (no public mempool). Requires validator-adjacent access.      | Medium — front-runner gains slightly better odds if they stake on the same outcome before ledger close. | - `prediction_cooldown_seconds` rate-limits consecutive predictions from the same address (configurable, default 0).<br>- `max_stake` / `max_total_stake` caps per-pool advantage.<br>- `resolution_delay` creates a quiet period after market close, preventing last-second stakes from being resolved in the same close.<br>- `HIGH_VALUE_THRESHOLD` monitoring for unusually large stakes. | Contract: `prediction.rs` — `place_prediction`, `prediction_cooldown_seconds`, `max_stake`, `max_total_stake`, `resolution_delay`<br>Backend: rate limiting on `/v1/predictions` |
| Commit-reveal scheme could suppress this vector but is not implemented as default.                                                  | Advisory (opt-in)                                                            | —                                                                                                       | See Section 4.2 below.                                                                                                                                                                                                                                                                                                                                                                        | N/A                                                                                                                                                                              |
| Parimutuel odds sniping: a late transaction shifts payout ratios for the winning side.                                              | Low — Stellar SCP does not expose a public mempool for arbitrary reordering. | Low — stake caps limit extractable advantage.                                                           | Same as above + public event exposure is inherent to a public market.                                                                                                                                                                                                                                                                                                                         | Contract + Backend                                                                                                                                                               |

**Conclusion:** Classical MEV (miner-extractable value) is largely absent on Stellar/Soroban. Residual risk is concentrated on validator-adjacent observers and oracle-timing races, both of which are bounded by existing controls.

### 3.2 Stale, Manipulated, or Malicious Oracle Data

| Path                                                                                                 | Likelihood                                                                                | Impact                                                                                     | Mitigations (existing)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | Responsible Component                                                                                                                                                                             |
| ---------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Oracle feed expires or exceeds `max_price_age` but is still accepted due to missing staleness check. | Very low — dual staleness check in `is_price_valid` (`expires_at` + `max_price_age`).     | High — resolution could be based on stale price, altering outcomes.                        | - `is_price_valid` checks (1) `current_time > feed.expires_at` and (2) `current_time > feed.timestamp + config.max_price_age`.<br>- `min_confidence_ratio` filter rejects feeds with abnormally wide uncertainty bands.<br>- 5× price deviation guard (`flash loan resistance`) in `evaluate_price_condition`.<br>- Manual fallback: operator may `resolve_pool` if price resolution fails.<br>- Oracle whitelist: `add_oracle`/`remove_oracle` gated by `admin.require_auth()` + `require_admin_role()`. | Contract: `price_feed.rs`, `oracle.rs` — Pyth adapter, staleness checks, confidence ratio, deviation guard<br>Backend: `price_cache.rs` — 60s CoinGecko fetches, `ensure_fresh` on-demand refresh |
| Malicious substitute of the Pyth contract address.                                                   | Low — admin must call `init_oracle` to set the contract; `add_oracle` whitelists keepers. | Very high — all price resolution would be under attacker control.                          | - `init_oracle` gated by `admin.require_auth()` + admin role check.<br>- Oracle updaters require `oracle.require_auth()` (whitelist enforcement).<br>- `remove_oracle` allows rotating off a compromised feed.                                                                                                                                                                                                                                                                                            | Contract: `oracle.rs` — `init_oracle`, `add_oracle`, `remove_oracle`, `oracle.require_auth()`                                                                                                     |
| Confidence ratio too low (manipulated feed with wide uncertainty).                                   | Low — validated in `is_price_valid`.                                                      | Medium — could allow price values far from true market rate.                               | - `min_confidence_ratio` config (admin-set; recommended ≤ 100 bps for stable assets, ≤ 500 bps for volatile).<br>- Confidence ratio check in `is_price_valid`.                                                                                                                                                                                                                                                                                                                                            | Contract: `price_feed.rs`                                                                                                                                                                         |
| Sustained price push (not flash loan) moving spot price between pool close and resolution.           | Low to medium — depends on `max_price_age` and pool duration.                             | Medium — if `max_price_age` is too large, a sustained push could shift resolution outcome. | - `max_price_age` should be set conservatively (≤ 60 s for volatile assets).<br>- Minimum pool duration enforced via `set_min_pool_duration`.<br>- Short pool durations naturally limit impact.                                                                                                                                                                                                                                                                                                           | Contract: `price_feed.rs`, `pool.rs`<br>Backend: `max_price_age` config enforcement                                                                                                               |

**Conclusion:** Core protections are in place: dual staleness checks, confidence ratio filtering, deviation guard, oracle whitelist, and admin-gated oracle configuration. Residual risk remains around oracle key compromise and sustained price pushes if `max_price_age` is not set conservatively.

### 3.3 Admin-Key Compromise

| Path                                                                                                     | Likelihood                                                                                           | Impact                                                                                               | Mitigations (existing)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | Responsible Component                                                                                                                                             |
| -------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Admin key leaked / stolen → unauthorized `pause`/`unpause`, fee changes, oracle whitelist modifications. | Medium — admin key is a single point of failure; if compromised, all privileged operations are open. | Very high — attacker could halt all trading, redirect fees, add malicious oracle, or censor payouts. | - `require_auth()` + `require_admin_role()` on every privileged function (see `security-access-control-audit.md`).<br>- Role separation: Operator role (id 1) for less-sensitive ops; Admin (id 0) for full control.<br>- `set_fee_bps` uses `PendingFeeBps` + `effective_at` timelock — changes are not immediate.<br>- Timelocked fee changes: `PendingFeeBps` + `effective_at` delay before taking effect.<br>- Withdrawal limits and audit trail events (`HighValuePredictionEvent`, `UnauthorizedAdminOpEvent`). | Contract: `admin.rs` — all admin functions follow `admin.require_auth()` + `require_admin_role()` pattern<br>Backend: API rate limiting, audit logging via events |
| Operator key compromise → limited but still harmful changes (stake limits, price conditions).            | Medium — operator key is separate but still privileged.                                              | High — could manipulate stake caps, price conditions, or referral parameters.                        | - Operator functions use `operator.require_auth()` + `require_role(&env, &operator, 1)`.<br>- See `security-access-control-audit.md` for full audited function list.                                                                                                                                                                                                                                                                                                                                                  | Contract: `admin.rs` / `access-control`                                                                                                                           |

**Conclusion:** Admin-key compromise is the highest-residual risk. Mitigations include defense-in-depth auth (Soroban-level + role-based), timelocked fee changes, and audit events. Key rotation procedure and multi-sig admin are recommended follow-ups (not yet implemented).

### 3.4 RPC / Indexer Divergence

| Path                                                                                                                | Likelihood                                                                              | Impact                                                                                       | Mitigations (existing)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | Responsible Component                                                                                      |
| ------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| Backend reads stale/wrong data from Stellar RPC → serves incorrect pool odds or prices to frontend.                 | Medium — network partitions, RPC rate limits, or node sync issues can cause stale data. | High — frontend may display wrong odds, users may stake on miscalculated pools.              | - Backend pools data from SQLx PostgreSQL (persistent).<br>- Redis cache has TTL fallback: `MARKET_CACHE_TTL_SECS = 300` in indexer worker; `POOL_CACHE_TTL = 10 s` in-memory.<br>- Indexer worker processes events from Soroban RPC and persists them; cache invalidation on each indexed event (`backend/src/indexer/worker.rs`).<br>- Health endpoints: `/healthz`, `/readyz` reflect backend readiness state.<br>- Price cache: background fetcher every 60 s + on-demand `ensure_fresh` with single-flight protection. | Backend: `price_cache.rs`, `pool_cache.rs`, `indexer/worker.rs`<br>RPC: Stellar RPC endpoint configuration |
| Indexer lags behind contract state → DB reflects outdated pool status (e.g., market shown as active when resolved). | Low to medium — depends on event processing lag.                                        | Medium — frontend may show stale market state; users may attempt claims on resolved markets. | - Indexer process runs as a Tokio task, processing events in batches.<br>- Each event triggers PostgreSQL persist + Redis cache invalidation.<br>- TTL fallbacks bound staleness (300 s market cache, 10 s pool cache).<br>- Missing/invalid events are logged but do not block processing.                                                                                                                                                                                                                                 | Backend: `indexer/worker.rs`                                                                               |

**Conclusion:** Divergence is bounded by TTL fallbacks and event-driven cache invalidation. The indexer is the single source of truth sync mechanism. Residual risk is limited-time staleness until the next event processes or TTL expires.

### 3.5 Dependency / Supply-Chain Compromise

| Path                                                                                                                      | Likelihood                                                   | Impact                                                                                                   | Mitigations (existing)                                                                                                                                                                                                                                                                                                                                                                             | Responsible Component                                                                  |
| ------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| Compromised Cargo dependency (soroban-sdk, reqwest, etc.) → runtime behavior change, secret leakage, or state corruption. | Low to medium — depends on crate provenance and CI scanning. | Very high — could alter contract logic, price validation, or wallet integration.                         | - CI secret scanning detects committed credentials (`.github/workflows/secret-scan.yml`).<br>- Container vulnerability scanning prior to deployment (see `docs/deploy/container-vulnerabilities.md`).<br>- Dependency allowlisting not currently enforced but reviewed per-release.<br>- `soroban-sdk` 23 locked in workspace `Cargo.toml`.<br>- `proptest` 1.6.0 used for property-based testing. | CI/CD: `.github/`<br>Contracts: `contract/Cargo.toml`<br>Backend: `backend/Cargo.toml` |
| Compromised Node.js dependency (frontend build tools, API client).                                                        | Low — frontend uses locked `package.json` versions.          | High — could inject malicious code into frontend build, redirect wallet connections, or exfiltrate data. | - `package-lock.json` locked in `frontend/`.<br>- CI runs `npm audit`.<br>- Secret scanning on every PR/push.<br>- Container image scanning.                                                                                                                                                                                                                                                       | Frontend: `frontend/package.json`<br>CI/CD: `.github/`                                 |

**Conclusion:** Supply-chain mitigations are primarily CI-level scanning and container scanning. No formal dependency allowlisting or SBOM exists yet. This is a recommendation for future hardening.

---

## 4. Residual Risks and Recommended Follow-Up Actions

| Risk                                         | Current Status                                                                                       | Recommended Action                                                                                                                                   | Priority |
| -------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | -------- |
| **Admin-key single point of failure**        | Mitigated by role separation and timelocked fee changes, but no multi-sig or key rotation procedure. | Implement admin multi-sig (e.g., 2-of-3) for critical operations; document key rotation procedure; add `docs/key-management.md`.                     | HIGH     |
| **Oracle key compromise**                    | Oracle whitelist + `admin.require_auth()` gating in place, but no key rotation or fallback oracle.   | Define oracle key rotation procedure; consider multi-oracle redundancy or fallback price source; add `docs/oracle-key-management.md`.                | HIGH     |
| **RPC/indexer divergence beyond TTL bounds** | TTL fallbacks exist (300 s market cache, 10 s pool cache), but no alerting on persistent divergence. | Add monitoring alert if indexer event lag exceeds threshold; publish indexer lag metric; add `/debug/indexer-lag` endpoint.                          | MEDIUM   |
| **Front-running on limited scale**           | Cooldown + stake caps + resolution_delay in place.                                                   | Evaluate commit-reveal as opt-in per-pool for high-value markets; set `prediction_cooldown_seconds` > 0 on mainnet.                                  | MEDIUM   |
| **Dependency compromise**                    | CI secret scanning and container scanning in place.                                                  | Generate SBOM; adopt dependency allowlisting or reproducible builds; add runtime dependency integrity checks where feasible.                         | MEDIUM   |
| **Price cache staleness (backend)**          | Background fetcher every 60 s + on-demand `ensure_fresh` with single-flight.                         | Evaluate TWAP accumulator in place of spot price for resolution (recommendation from `security-price-feed-analysis.md`); add price staleness metric. | LOW      |
| **Commit-reveal not implemented**            | Documented as opt-in advisory only.                                                                  | If future Stellar changes introduce public mempool, revisit commit-reveal scheme design.                                                             | LOW      |

---

## 5. Maintenance

This threat model must be reviewed when any of the following change:

- **Smart contracts**: new contract deployment, contract upgrade, role or parameter changes (e.g., `max_price_age`, `min_confidence_ratio`, `resolution_delay`, stake caps).
- **Oracle integrations**: new oracle provider, changes to Pyth contract configuration, confidence ratio adjustments, or staleness threshold changes.
- **Backend/indexer behavior**: changes to price cache TTL, indexer event processing logic, Redis key structure, or database schema.
- **Wallet flows**: changes to Freighter connection, wallet signing flow, SEP-10 auth, or network configuration.
- **Deployments**: new container images, infrastructure changes, environment variable updates, or dependency version bumps.
- **Dependencies**: any Cargo or npm dependency updates, especially major version bumps of `soroban-sdk`, `reqwest`, or core runtime crates.

### Review Triggers and Workflow

| Trigger                                                  | Action                                                                                                        | Owner               |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- | ------------------- |
| Contract deployment or upgrade                           | Update threat model; verify all mitigations still apply; run contract test suite.                             | Contracts team      |
| Oracle configuration change                              | Revalidate `max_price_age`, `min_confidence_ratio`, and deviation guard thresholds; run oracle vector tests.  | Oracle/Backend team |
| Backend release (new Docker image, dependency bump)      | Run container vulnerability scan; review `CHANGELOG` for security-relevant changes; re-run integration tests. | DevOps/Backend team |
| Wallet client change (Freighter version, network switch) | Validate wallet connection flow against `SECURITY.md`; test on target network.                                | Frontend team       |
| New dependency addition                                  | Run `cargo audit` / `npm audit`; evaluate transitive dependencies; add to SBOM if applicable.                 | CI/Build team       |

### Unresolved High-Risk Scenarios

Any unresolved high-risk scenario (severity CRITICAL or HIGH in this threat model) **must** be tracked as a GitHub issue with the label `security-threat-model` and a link back to this document. The issue should describe the specific residual risk, the mitigations being attempted, and the acceptance criteria for closure. Open high-risk items must not be carried across releases without explicit evaluation and documentation of the new risk posture.

---

## 6. Security-Review Workshop Record

| Field                     | Value                                                                               |
| ------------------------- | ----------------------------------------------------------------------------------- |
| **Workshop date**         | Pending — no security-review workshop has been conducted yet for this threat model. |
| **Participants**          | Pending — to be populated after workshop.                                           |
| **Scenarios reviewed**    | Pending — to be populated after workshop.                                           |
| **Decisions**             | Pending — to be populated after workshop.                                           |
| **Follow-up issue links** | Pending — to be populated after workshop.                                           |

_When a security-review workshop is held, this section should be completed with actual participant notes, reviewed scenarios, decisions made, and any GitHub issues created as follow-ups. Each follow-up issue should be linked via its URL (e.g., `closes #1234`). Until a workshop occurs, this section must remain as "Pending" and must not be fabricated._

---

## 7. References (existing security documentation)

- `docs/security-price-feed-analysis.md` — Price feed manipulation resistance analysis
- `docs/security-front-running-analysis.md` — Front-running protection analysis
- `docs/security-reentrancy-analysis.md` — Reentrancy analysis of claim functions
- `docs/security-access-control-audit.md` — Access control audit for admin functions
- `SECURITY.md` — Vulnerability reporting policy and SLAs
- `docs/deploy/container-vulnerabilities.md` — Container scanning policy
- `ARCHITECTURE_OVERVIEW.md` — System architecture overview
- `contract-versioning.md` — Contract upgrade and migration policy

---

_This threat model is a living document. Its purpose is to make the security posture explicit, surface residual risks, and provide a framework for ongoing maintenance. It does not modify any application or contract behavior — it is documentation only._
