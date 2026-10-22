# Backend Secret Rotation and Revocation Runbook

## 1. Executive Summary & Scope

This runbook establishes a verified operational procedure for rotating backend signing keys, configuration secrets, and administrative credentials across the PrediFi platform without downtime, service disruption, or exposing secret material in logs, traces, or artifacts.

It defines:
- A comprehensive **Secret Inventory** covering owners, consumers, overlap windows, rollback constraints, and verification hooks.
- Dual-key and versioned **Rotation Workflows** allowing zero-downtime overlaps.
- Emergency **Compromised Credential Revocation Procedures** providing immediate revocation paths.
- An automated, non-leaking **Staging Rotation Drill** verifying rotation and revocation without logging secret material.

---

## 2. Secret Inventory & Classification Matrix

| Secret Identifier / Env Var | Classification & Purpose | Owner | Dependent Services / Consumers | Overlap / Grace Window | Rollback Constraints | Post-Rotation Verification Hook |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `PREDIFI_SECRET_KEY` + `PREDIFI_JWT_KEY_VERSION` | Backend HMAC signing secret and key version for access/refresh JWT tokens. | Backend Platform Team | Rust Axum API (`backend`), WebSocket handler (`/api/v1/ws`), Frontend Client session store. | **Dual-key / N-1 grace window:** When rotated with new version $V+1$, old tokens with version $V$ can be honored for access during remaining lifetime (max 1 hr) while issuing version $V+1$, or invalidated immediately upon emergency. | Rolling back `PREDIFI_JWT_KEY_VERSION` re-authorizes tokens issued with older key version unless JTIs or user cutoff timestamps are durably blacklisted in Redis. | Issue new token via `POST /api/v1/auth/refresh`, verify decode against key version $V+1$, ensure version $V$ rejected when strict enforcement active. |
| `PREDIFI_ADMIN_API_KEY` | High-privilege API key passed via `x-admin-api-key` header to mutate runtime logging (`PATCH /api/v1/admin/log-level`). | Security / DevOps SRE | Rust Axum API (`backend`), Admin CLI / Operational Scripts. | **Zero overlap (instant cutover):** Admin clients must switch to new key immediately upon backend environment update. | Revert secret in Secret Manager and restart backend pods if new key is lost. | Execute `PATCH /api/v1/admin/log-level` with new key expecting HTTP 200; verify old key yields HTTP 401 Unauthorized. |
| `PREDIFI_DATABASE_URL` | PostgreSQL connection string containing role credentials and database endpoint. | DBA / Platform Infrastructure | Axum API service, Database connection pool (`sqlx`), Indexer background worker. | **Dual-user overlap window (24 hrs):** Database provisions role `predifi_app_v2` alongside `predifi_app_v1` before revoking `v1`. | Rollback requires retaining old database user permissions until new deployment finishes and stabilizes. | Check `/api/v1/ready` and execute query probe on `GET /api/v1/health`. |
| `PREDIFI_REDIS_URL` | Redis instance URL containing host, port, and optional auth password. | Platform Infrastructure | Cache layer (`RedisCache`), Token revocation store (`jwt:revoked:*`, `jwt:user:revoke_before:*`), Rate limiter. | **Dual-credential overlap window (1 hr):** Redis server accepts previous and current ACL user/password concurrently during rollover. | Fail closed: If Redis connection is broken, token rotation and user revocation fail closed (HTTP 503). | Check Redis ping via `GET /api/v1/health` and verify cache key set/get operations. |
| `PREDIFI_STELLAR_RPC_URL` (API Keys) | RPC service endpoint URL containing optional basic auth or API key query/header. | Blockchain Core / Indexer Team | Ingestion workers, RPC client (`reqwest`), Contract event polling loop. | **Multi-key overlap window (48 hrs):** Upstream RPC provider allows multiple active API tokens concurrently. | Old provider token must remain active until traffic fully cut over and confirmed healthy. | Probe `/api/v1/health` verifying `getHealth` Soroban RPC check succeeds with HTTP 200. |

---

## 3. Secret Protection & Logging Redaction Guarantees

All operators, CI/CD pipelines, and scripts MUST adhere to zero-exposure rules:
1. **Never print plaintext secrets:** CLI commands must read secrets from environment variables, files with restricted permissions (`chmod 600`), or Secret Manager CLI integrations (`aws secretsmanager`, `vault kv get`).
2. **Redaction in Tracing and Structured Logs:** The Axum backend incorporates automated tracing filters (`backend/src/trace_propagation.rs` and `backend/src/telemetry.rs`) that sanitize and drop attributes matching sensitive patterns:
   - `*secret*`
   - `*key*`
   - `*password*`
   - `*token*`
   - `*credential*`
   - `*authorization*`
3. **Audit and Shell History:** Operators must disable shell history or prefix commands with a space (`HISTCONTROL=ignorespace`) when injecting temporary environment variables during manual operations.

---

## 4. Standard Zero-Downtime Rotation Procedure (Scheduled)

### Phase 1: Preparation & Dual-Secret Provisioning
1. Generate the new secret using a cryptographically secure RNG (e.g. `openssl rand -hex 32`).
2. Store the new secret in AWS Secrets Manager / Vault under the staged version label (`AWSCURRENT` / `AWSPENDING`).
3. For database and Redis credentials, provision the second user or password on the target cluster before updating the application.

### Phase 2: Staged Deployment (Canary / Rolling Update)
1. For JWT Signing Key:
   - Update `PREDIFI_SECRET_KEY` to the new secret string.
   - Increment `PREDIFI_JWT_KEY_VERSION` by 1 (e.g. from `1` to `2`).
   - Deploy backend pods with rolling deployment (`maxSurge: 25%`, `maxUnavailable: 0`).
2. Dual-Verification Window:
   - Existing valid tokens continue to operate until expiration or refresh.
   - Any client calling `POST /api/v1/auth/refresh` receives a new token pair signed with the new secret and version.

### Phase 3: Post-Rotation Verification
1. Run automated verification probe (refer to Section 6).
2. Monitor application metrics:
   - 401 Unauthorized rate (must remain within normal baseline).
   - `/api/v1/health` Soroban RPC and DB status (must remain `ok`).
   - Redis connection status.

### Phase 4: Deprecation & Cleanup
1. Once the maximum token lifespan expires (1 hour for access tokens, or refresh cycle grace window), deprecate the previous secret in Secrets Manager.
2. For database/Redis credentials, drop the old role/user after the 24-hour overlap window.

---

## 5. Emergency Compromised Credential Revocation Procedure

When a credential or signing secret is known or suspected to be compromised, immediate invalidation takes precedence over smooth token expiration.

### Scenario A: Compromised JWT Signing Secret (`PREDIFI_SECRET_KEY`)
1. **Immediate Key Invalidation:**
   - Immediately increment `PREDIFI_JWT_KEY_VERSION` by 1 in the runtime configuration and generate a new `PREDIFI_SECRET_KEY`.
   - Perform an expedited rollout restart of backend containers:
     ```bash
     kubectl rollout restart deployment/predifi-backend -n predifi-staging
     ```
   - **Effect:** Any token signed with the compromised key or prior version is instantly rejected with `KeyVersionMismatch` (HTTP 401) on access token verification and refresh token exchange.
2. **Global Session Purge in Redis:**
   - If specific user sessions or compromised accounts are known, set the user revocation cutoff in Redis:
     `SET jwt:user:revoke_before:<user_address> <current_unix_timestamp> EX 2592000`
   - To purge all active refresh tokens globally, flush the active refresh tokens or bump the version boundary.

### Scenario B: Compromised Administrative Key (`PREDIFI_ADMIN_API_KEY`)
1. **Immediate Revocation:**
   - Update `PREDIFI_ADMIN_API_KEY` in Secret Manager / deployment manifest with a new high-entropy string.
   - Restart backend instances.
2. **Audit Verification:**
   - Verify previous admin key returns HTTP 401 Unauthorized:
     ```bash
     curl -i -X PATCH http://localhost:3000/api/v1/admin/log-level \
       -H "Content-Type: application/json" \
       -H "x-admin-api-key: <REVOKED_OLD_KEY>" \
       -d '{"log_level":"info"}'
     ```
   - Inspect backend access logs for any unauthorized `PATCH /api/v1/admin/log-level` attempts during the compromise window.

### Scenario C: Compromised Database or Cache Credential
1. **Immediate Database Role Revocation:**
   - In PostgreSQL, alter the compromised role password immediately:
     ```sql
     ALTER ROLE predifi_app WITH PASSWORD '<NEW_STRONG_PASSWORD>';
     SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = 'predifi_app';
     ```
   - Restart the backend pods with the updated `PREDIFI_DATABASE_URL`.

---

## 6. Staging Rotation Drill & Automation Tool

To satisfy continuous compliance and readiness verification, run the automated drill script:

```bash
node scripts/secret-rotation-drill.mjs --staging
```

The drill validates:
1. Secret masking and absence of secret strings in outputs/logs.
2. JWT key rotation and version bump validation.
3. Strict rejection of revoked / expired credentials.
4. User-level token revocation cutoff (`jwt:user:revoke_before`).
5. Admin API key revocation validation.
