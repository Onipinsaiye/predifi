# Staged Deployment and Rollback Checklist

Use this checklist for each release candidate before promoting it beyond
staging. It covers the backend, PostgreSQL, Redis, and Soroban contracts when
they are part of the release. It is an execution record as well as a procedure:
the contributor carrying out the deployment must record a result and evidence
for every gate.

The person executing the staging rehearsal **must not be an author of this
checklist**. The release author may prepare the release and evidence, but a
different contributor must independently verify the staged deployment and
complete the rehearsal record below. Do not mark a gate passed without
verifiable evidence.

## Stop conditions

Stop the release at the current gate and notify the release owner if any of the
following applies:

- The release commit, tag, build, artifact digest, target environment, or
  contract network/address cannot be verified.
- Required CI checks fail, required owners are unavailable, or the staging
  environment is unhealthy before deployment.
- A migration is unreviewed, incompatible with the deployed application, or
  has no tested recovery path.
- A migration lock, statement timeout, migration ledger, schema, or data
  invariant is unexpected.
- A contract storage change is not readable by the proposed rollback artifact,
  or a required migration cannot be safely reversed or compensated.
- A smoke check fails, an alert fires, errors increase beyond the agreed
  release threshold, or the backlog grows without recovering.
- Any required verification has no recorded result or evidence.

Do not bypass a stop condition by continuing deployment. Record the failure,
notify the owner, and resume only after an approved remediation and a new
verification.

## Roles and release record

Complete before starting:

| Field | Record |
|---|---|
| Release/tag and source commit (full SHA) | |
| Release owner | |
| Deployment operator (name/handle) | |
| Checklist author(s) | |
| Independent operator confirmed not an author | |
| Staging environment/account/region | |
| Planned deployment window (UTC) | |
| Backend image/artifact reference and immutable digest | |
| Frontend artifact reference and immutable digest, if changed | |
| Contract release manifest and artifact hashes, if changed | |
| Previous known-good application and contract artifact references | |
| Database backup/PITR restore point and verification reference | |
| Rollback owner and decision-maker | |
| Monitoring window and agreed error/latency/backlog thresholds | |
| Release/change record | |

The release owner must provide concrete monitoring thresholds before the
deployment starts. At minimum, specify acceptable 5xx rate, latency, dependency
health, and worker/indexer backlog behavior. If a metric has no agreed threshold
or baseline, deployment is blocked until the owner records one.

## Ordered deployment procedure

For every numbered gate, enter **PASS**, **FAIL**, or **N/A** and cite a
timestamped command result, dashboard, CI run, query output, or approved change
record in the rehearsal evidence table. N/A requires the release owner to state
why the component is not included.

### 1. Authorize and freeze the release candidate

- Confirm the release commit is reviewed, immutable, and the intended commit
  for this deployment. Record its full SHA and tag.
- Confirm required backend, frontend, and contract CI checks completed
  successfully for that exact commit. Record run links; do not substitute a
  local build or a result from another commit.
- Review the release notes, affected services, API/event compatibility,
  configuration changes, database migration plan, contract storage changes,
  and rollback plan.
- Confirm the independent staging operator and release owner are available
  through deployment and the entire monitoring window.
- Confirm the staging change window, communications channel, alert routing, and
  an agreed go/no-go decision-maker.

**Gate:** No deployment without approval, passing required CI, named owners,
and a complete rollback plan.

### 2. Verify build artifacts before upload or promotion

- Build from the approved source commit using the repository's locked
  dependencies and documented toolchain.
- Record the exact build command, toolchain versions, artifact names, and
  cryptographic checksums. For contracts, use the final optimized Wasm that
  will actually be uploaded; the raw build is not the deployable artifact.
- Compare the artifact checksum to the reviewed release manifest and, where
  applicable, the registry's immutable image digest. Reject mutable tags such
  as `latest` as the only artifact identity.
- Confirm the artifact being promoted to staging is the same artifact intended
  for production, not a rebuild with different bytes.
- For Soroban, record the network passphrase, contract IDs, uploaded Wasm hash,
  source commit, schema version, and the confirmed deployment/upgrade
  transaction evidence. An upload alone is not proof of deployment.

**Gate:** Any digest, source, target-network, or manifest mismatch blocks
deployment.

### 3. Validate staging configuration and dependencies

- Confirm the target is staging (cloud account/project, region, hostname, and
  infrastructure state), not production.
- Verify the staging configuration against
  [environment-variables.md](../environment-variables.md) and the staging
  Terraform variable example. Confirm database, Redis, Stellar RPC/network,
  CORS/origin, rate limits, telemetry, and health-check settings are intended
  for staging.
- Confirm secrets are supplied through the approved secret manager and are not
  in source control, build artifacts, command history, release records, or
  logs. Never copy production credentials into staging.
- Verify PostgreSQL, Redis, RPC/network access, DNS/TLS, load balancer health,
  and available capacity before changing the application.
- Review the Terraform plan (if infrastructure changes are included) and get
  the environment owner’s approval before applying it.
- Capture baseline `/live`, `/ready`, `/health`, application error/latency
  metrics, database connections/locks, Redis health, and worker/indexer queue
  depth. Resolve existing staging faults before proceeding.

**Gate:** Stop if staging identity is uncertain, a dependency is unhealthy, or
baseline measurements are missing.

### 4. Back up and validate database migration ordering

- Capture the approved database backup or confirm point-in-time recovery is
  enabled, record its recovery point, and verify the documented restore
  procedure and owner. A backup that has not been shown restorable is not a
  rollback plan.
- Inspect current PostgreSQL version, schema, table sizes, active transactions,
  lock blockers, and `_sqlx_migrations` success state. Use the checks and
  populated-table guidance in
  [MIGRATION_DEPLOYMENT_GUIDE.md](../MIGRATION_DEPLOYMENT_GUIDE.md).
- Review migrations in dependency order. Classify lock impact, duration,
  backfill size, compatibility with both the current and candidate application,
  and whether the change is reversible. Prefer expand/contract for changes to
  populated tables.
- Set and record suitable lock and statement timeouts. Agree on the abort
  condition and who may terminate a blocked migration.
- **Repository-specific stop condition:** the current migration set has
  duplicate SQLx version prefixes for `004` and `009`. Do not run migrations
  until the release owner has verified a controlled migration plan for the
  target database that respects existing `_sqlx_migrations` history and does
  not blindly renumber already-applied migrations. Record the approved plan and
  its evidence.
- Apply database migrations as a distinct, controlled step before deploying
  application code that depends on the new schema. Do not rely on application
  startup as an undocumented migration mechanism.
- After migration, confirm every intended migration is recorded successful,
  the schema/data invariants hold, no unexpected invalid indexes or failed
  migration rows exist, and application-compatible reads/writes work.

**Gate:** Stop on a migration failure, unexpected lock wait, duplicate version
ambiguity, schema mismatch, or failed invariant. Follow the interrupted
migration runbook; do not delete migration-ledger rows or renumber history
without an approved recovery plan.

### 5. Deploy the staged artifacts

- Record the deployment mechanism, operator identity, start time, artifact
  digest, and change/approval reference. Deploy the verified candidate
  progressively where the environment supports it.
- If backend and frontend are released together, deploy in the compatibility
  order documented for the release and verify the previous client/server
  combination remains supported during transition.
- For contract upgrades, follow
  [contract-upgrade-procedure.md](../contract-upgrade-procedure.md) and the
  release-specific manifest. Check admin authorization, pause/unpause order,
  the new Wasm hash, transaction confirmation, contract version, and storage
  compatibility. The access-control contract has no in-place upgrade entry
  point.
- Watch deployment events, target health, startup logs, database connections,
  and worker/indexer progress while instances roll out. Do not remove the
  previous known-good application artifact or contract metadata.
- Record the deployed image/artifact digest and observed contract hashes/IDs;
  compare them with the approved manifest.

**Gate:** Stop rollout and begin the rollback decision procedure if the new
deployment fails health checks, cannot connect to dependencies, or serves
unexpected artifact hashes.

### 6. Run smoke checks and verify projections

Use the staging base URL and approved, non-production test identities/data.
Record HTTP status, response summary, timestamp, and evidence for each check:

- `GET /live` reports that the process is running.
- `GET /ready` reports readiness and healthy required dependencies.
- `GET /health` returns healthy status for database, RPC, Redis, and price
  cache. A missing or unreachable required dependency is a failure, not a
  warning to ignore.
- `GET /` identifies the expected backend service. Record the reported
  application version and compare it with the release.
- `GET /api/v1/pools` (or the configured API base path) returns a valid
  response; inspect one known staged pool and its details.
- If indexing is part of the release, submit or observe one approved test
  event, confirm it is projected once, verify dependent reads/aggregates, and
  confirm worker backlog returns to baseline. Do not submit real user or
  production-network transactions during a staging rehearsal.
- Exercise changed write paths and frontend flows only with disposable staging
  data and authorized test accounts. Verify expected error cases as well as
  the success path.
- For contract changes, query the read-only getters and verify the staged
  client/indexer can consume the new events and errors. Confirm contract
  network, address, observed Wasm hash, and version against the release
  manifest.
- Confirm cache invalidation, monitoring/alert delivery, and logs/traces for
  the changed path.

**Gate:** Any failed smoke check, duplicate/missing projection, incorrect
aggregate, unexpected response, or absent monitoring signal blocks promotion.

### 7. Monitor staging and decide go/no-go

- Monitor for at least **30 minutes after the last deployment change**. Record
  a health, error-rate, latency, database/Redis, and worker/indexer-backlog
  snapshot at least every five minutes.
- Compare readings to the thresholds and baseline recorded before deployment.
  Verify errors are within the agreed limit, latency remains within the agreed
  limit, required dependencies stay healthy, and backlog is stable or draining
  to its baseline without new dead letters.
- Inspect application logs, traces, database lock/deadlock indicators, Redis
  errors, and relevant alerts. Record all alerts, including those that
  recovered.
- The release owner and staging operator jointly record **GO** or **NO-GO**.
  Production promotion requires a separate, explicit authorization; passing
  staging is not authorization to deploy production.

**Gate:** Any threshold breach, unexplained alert, sustained backlog growth, or
missing monitoring evidence is NO-GO. Invoke rollback or hold staging for
investigation.

## Rollback and recovery decision procedure

1. **Declare and contain.** The release owner records the NO-GO/rollback
   decision, timestamp, affected artifact, observed impact, and incident/change
   reference. Stop further rollout and writes that could worsen data or contract
   state, using the service's approved operational controls.
2. **Select the recovery path.** Prefer a forward fix when it is safe and
   faster. Otherwise restore the previous immutable backend/frontend artifact
   only if it is compatible with the schema and data already written.
3. **Check database compatibility first.** Application rollback does not undo
   schema changes. Do not reverse a migration by improvisation. Use only the
   migration's reviewed down/forward-recovery plan. A point-in-time restore
   requires database owner approval, an explicit recovery point, and acceptance
   of the writes lost after that point.
4. **Check contract storage compatibility first.** A Soroban rollback is a
   forward upgrade to a previous Wasm hash; it does not revert storage or
   decrement the upgrade counter. Only use an older artifact proven to read
   the current schema. If it cannot, use a reviewed compensating migration or
   forward fix. Never treat a contract redeploy as a state restore.
5. **Verify recovery.** Repeat the health and smoke checks, confirm artifact
   identities, check schema and data invariants, check event/indexer progress,
   verify backlog recovery, and confirm alert state. Keep heightened monitoring
   until the owner accepts recovery.
6. **Preserve evidence.** Record the old/new artifact identities, action,
   operator, timestamps, approvals, database/contract recovery details, data
   impact, smoke-check evidence, and follow-up actions. Do not overwrite or
   delete the failed deployment record.

If there is no known-safe rollback path, keep the deployment stopped and
escalate to the release owner and database/contract owners. Do not perform an
unapproved restore, migration reversal, contract upgrade, or key operation.

## Independent staging rehearsal record

The staging operator who completes this record must not have authored this
checklist. Use one row per numbered gate above, including N/A decisions. Attach
or link evidence; retain the completed record with the release/change record.
Do not fill this with expected results in advance.

| Check | Result (PASS/FAIL/N/A) | Verification result and evidence link | Operator | UTC timestamp |
|---|---|---|---|---|
| 1a. Release commit, tag, and required CI | | | | |
| 1b. Change review, compatibility, and migration/rollback plan | | | | |
| 1c. Independent operator, owners, window, and communications | | | | |
| 2a. Build command, toolchain, and artifact checksums | | | | |
| 2b. Manifest, immutable digest, and promoted artifact identity | | | | |
| 2c. Contract upload/deployment evidence (if applicable) | | | | |
| 3a. Staging account, region, hostname, and infrastructure plan | | | | |
| 3b. Environment configuration and secret handling | | | | |
| 3c. Dependency health, capacity, and pre-deployment baseline | | | | |
| 4a. Backup/PITR recovery point and restore procedure | | | | |
| 4b. PostgreSQL version, schema, locks, migration ledger, and table sizes | | | | |
| 4c. Migration order, compatibility, timeouts, and recovery plan | | | | |
| 4d. Duplicate SQLx version-prefix resolution and approval | | | | |
| 4e. Migration completion, schema/data invariants, and failed-index check | | | | |
| 5a. Deployment approval, operator, start time, and artifact digest | | | | |
| 5b. Backend/frontend rollout and health | | | | |
| 5c. Contract network/address/hash/version (if applicable) | | | | |
| 6a. `/live`, `/ready`, `/health`, and root response/version | | | | |
| 6b. Pool-list and pool-detail API smoke checks | | | | |
| 6c. Indexer projection, deduplication, dependent reads, and backlog (if applicable) | | | | |
| 6d. Changed client/write flows and contract compatibility (if applicable) | | | | |
| 6e. Cache invalidation, logs, traces, and alert delivery | | | | |
| 7a. Five-minute monitoring snapshots for at least 30 minutes | | | | |
| 7b. Error, latency, dependency, and backlog thresholds | | | | |
| 7c. Alerts reviewed and joint staging GO/NO-GO decision | | | | |
| Rollback/recovery (if invoked) | | | | |

### Final sign-off

| Sign-off | Name/handle | Decision | UTC timestamp | Evidence/reference |
|---|---|---|---|---|
| Independent staging operator (not checklist author) | | | | |
| Release owner | | | | |
| Database owner (if migrations included) | | | | |
| Contract owner (if contract release included) | | | | |
| Production promotion authorization (separate decision) | | | | |

**Rehearsal outcome:** Not started / Passed / Failed / Rolled back
**Follow-up actions and owners:**
