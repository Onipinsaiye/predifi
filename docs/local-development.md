# Local Development Environment

PrediFi provides a **one-command local full-stack environment** via Docker Compose. It automatically provisions and coordinates every required service: **PostgreSQL**, **Redis**, a **local Stellar/Soroban standalone network**, the **Axum backend API**, and the **Next.js frontend**. Database migrations and deterministic sample data are automatically seeded before the backend accepts traffic.

---

## Quickstart (One Command)

From the repository root, start the entire stack:

```bash
docker compose up
```

Or using the developer helper script:

```bash
# macOS / Linux
./scripts/dev.sh up

# Windows PowerShell
.\scripts\dev.ps1 up
```

Docker Compose automatically compiles dependencies, runs database migrations, seeds deterministic accounts and prediction pools, and brings up the services in their proper dependency order.

To run containers in the background (detached mode):
```bash
docker compose up -d
```

> **Isolated Wallet E2E Testing:** For the isolated wallet-signing quality gate, use `npm --prefix e2e/wallet run test:local` after installing the prerequisites in [Local Soroban wallet E2E](wallet-local-e2e.md). It starts only `stellar`, deploys fresh local contracts, and leaves the service running for further local work. If this machine has no Docker engine, run the same test on a GitHub-hosted Ubuntu runner from your fork as described there.

---

## Exposed Ports & Services

Every service exposes stable ports on your host machine:

| Service | Host URL / Port | Container Port | Protocol | Purpose |
| :--- | :--- | :--- | :--- | :--- |
| **`frontend`** | [`http://localhost:3001`](http://localhost:3001) | `3000` | HTTP | Next.js web application with hot reload |
| **`backend`** | [`http://localhost:3000`](http://localhost:3000) | `3000` | HTTP | Axum API, WebSocket feeds, OpenAPI docs |
| **`postgres`** | `localhost:5432` | `5432` | PostgreSQL | Relational database (`user: postgres`, `pass: postgres`, `db: predifi`) |
| **`redis`** | `localhost:6379` | `6379` | Redis | Hot data caching, rate limiting, pub/sub |
| **`stellar`** | [`http://localhost:8000`](http://localhost:8000) | `8000` | HTTP | Local standalone Stellar network (Horizon & Soroban RPC) |
| **`stellar` (peer)** | `localhost:11626` | `11626` | TCP | Stellar Core consensus & peer network |
| **`grafana`** *(optional)* | [`http://localhost:3002`](http://localhost:3002) | `3000` | HTTP | Observability dashboard (`admin`/`admin`, via `--profile logging`) |
| **`loki`** *(optional)* | `localhost:3100` | `3100` | HTTP | Log aggregation ingestion and query API |

> **Port Overrides:** All ports can be customized without altering `docker-compose.yml` by copying `.env.example` to `.env` and adjusting `FRONTEND_PORT`, `BACKEND_PORT`, `POSTGRES_PORT`, `REDIS_PORT`, or `STELLAR_PORT`.

---

## Automated Service Ordering & Health Checks

Services start deterministically without manual timing or guessing service order. Dependent containers use Docker Compose `condition: service_healthy` or `service_completed_successfully`:

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│  PostgreSQL  │     │    Redis     │     │   Stellar    │
│ (healthy)    │     │  (healthy)   │     │  (started)   │
└──────┬───────┘     └──────┬───────┘     └──────┬───────┘
       │                    │                    │
       ▼                    │                    │
┌──────────────┐            │                    │
│   db-seed    │            │                    │
│ (completed)  │            │                    │
└──────┬───────┘            │                    │
       │                    │                    │
       └──────────────┬─────┴────────────────────┘
                      ▼
               ┌──────────────┐
               │   backend    │
               │  (healthy)   │
               └──────┬───────┘
                      ▼
               ┌──────────────┐
               │   frontend   │
               │  (healthy)   │
               └──────────────┘
```

### Health Check Probes

1. **PostgreSQL (`postgres`)**
   - **Probe:** `pg_isready -U postgres -d predifi`
   - **Cadence:** Interval 10s, timeout 5s, 5 retries.
2. **Redis (`redis`)**
   - **Probe:** `redis-cli ping`
   - **Cadence:** Interval 10s, timeout 5s, 5 retries.
3. **Stellar Network (`stellar`)**
   - **Probe:** `curl -fsS http://localhost:8000/ >/dev/null || exit 1`
   - **Cadence:** Interval 15s, timeout 10s, 20 retries, start period 120s.
4. **Database Seeder (`db-seed`)**
   - One-shot initialization task. Waits for `postgres: condition: service_healthy`, applies all `sqlx` migrations, populates fixture data, and exits with code 0 (`service_completed_successfully`).
5. **Backend API (`backend`)**
   - **Probe:** `curl -fsS http://localhost:3000/health >/dev/null || curl -fsS http://localhost:3000/live >/dev/null || exit 1`
   - **Checks:** Validates database connection, Redis connectivity, Soroban RPC health, and in-memory price cache.
   - **Cadence:** Interval 30s, timeout 5s, 5 retries, start period 180s.
6. **Frontend UI (`frontend`)**
   - **Probe:** Node HTTP fetch probe on internal port 3000 (`node -e "fetch('http://localhost:3000').then(...)"`).
   - **Cadence:** Interval 15s, timeout 5s, 10 retries, start period 45s.

---

## Seeded Accounts & Sample Data

When starting the environment, the `db-seed` service automatically runs migrations and seeds deterministic sample data so you can test features immediately without waiting for blockchain events.

### Seeded Wallet Identities

The seeder inserts test prediction activity under stable fixture public keys:

| Account Type | Address | Notes |
| :--- | :--- | :--- |
| **Betting Wallet 1** | `GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA1` | Active predictor on sports & crypto |
| **Betting Wallet 2** | `GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA2` | Active predictor |
| **Betting Wallet 3** | `GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA3` | Active predictor |
| **Betting Wallet 4** | `GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA4` | Active predictor |
| **Betting Wallet 5** | `GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA5` | Active predictor |
| **Betting Wallet 6** | `GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA6` | Active predictor |
| **Betting Wallet 7** | `GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA7` | Active predictor |
| **Betting Wallet 8** | `GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA8` | Active predictor |
| **Referrer Wallet** | `GRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR` | Receives 50% referral fee accruals |

### Local Stellar Network Credentials

The `stellar` quickstart container provides a standalone Soroban testnet:

- **Network Passphrase:** `"Standalone Network ; February 2017"`
- **Horizon & Soroban RPC:** `http://localhost:8000` (RPC at `http://localhost:8000/rpc`)
- **Friendbot (Free XLM funding):**
  ```bash
  curl "http://localhost:8000/friendbot?addr=<YOUR_STELLAR_PUBLIC_KEY>"
  ```
- **Root Admin Account:**
  - **Public Key:** `GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H`
  - **Secret Key:** `SDUCF5V4N5OXYL6J5VXZQW54VFFI6N4W5V3Z4PXZV73Q37NVZ3NVMNXY`

### Seeded Markets & Tokens

- **10 Sample Prediction Pools:** Spread across `Sports`, `Crypto`, `Politics`, and `Entertainment` in `open`, `closed`, and `settled` states.
- **Tokens Supported:** Native XLM, USDC (`GA5ZSEJYB37JRC5AVCIA5MOP4RztMfo6BASE2QYX`), EURC, and AQUA.

### Re-seeding and Custom Scenarios

The seeder is idempotent (`ON CONFLICT DO NOTHING`). To re-seed or test specific edge cases:

```bash
# Wipe existing data and re-seed fresh:
docker compose run --rm db-seed cargo run --bin predifi-seed -- --fresh

# Seed settled pools scenario only:
docker compose run --rm db-seed cargo run --bin predifi-seed -- --scenario settled

# Seed 25 pools instead of the default 10:
docker compose run --rm db-seed cargo run --bin predifi-seed -- --num-pools 25
```

---

## Core Developer Workflows

### Workflow 1: Full-Stack Development (Recommended)

Start everything with live reload:
```bash
docker compose up
```
- Frontend code in `./frontend` hot-reloads via Next.js.
- Backend code in `./backend` re-compiles automatically via `cargo-watch`.

### Workflow 2: Backend + Infrastructure Only

If you are developing backend features or running frontend natively on your host machine:
```bash
docker compose up postgres redis stellar db-seed backend
```

### Workflow 3: Full-Stack with Public Stellar Testnet

If you prefer using the public Stellar Testnet instead of the local Stellar container (saves ~2 GB of memory):
```bash
PREDIFI_STELLAR_RPC_URL=https://soroban-testnet.stellar.org \
docker compose up postgres redis db-seed backend frontend
```

### Workflow 4: Hybrid (Native Host Services)

If you prefer running the Rust backend or Next.js frontend directly on your host machine:
1. Start infrastructure services in Docker:
   ```bash
   docker compose up -d postgres redis
   ```
2. Start the backend natively:
   ```bash
   cd backend
   cp .env.example .env
   cargo run --bin predifi-backend
   ```
3. Start the frontend natively:
   ```bash
   cd frontend
   cp .env.example .env.local
   pnpm install
   pnpm dev
   ```

---

## Optional Services: Log Aggregation & Observability

PrediFi includes an optional observability profile providing **Loki** (log ingestion), **Promtail** (log collection), and **Grafana** (dashboards):

```bash
docker compose --profile logging up
```

Access Grafana at [`http://localhost:3002`](http://localhost:3002) with credentials `admin` / `admin`. The pre-configured **PrediFi — Logs & Error Trends** dashboard is ready in the *PrediFi* folder.

---

## Cleanup & Teardown

### Standard Stop (Preserve Data & Caches)
Stops containers while preserving database state and Rust/Node compilation caches:
```bash
docker compose down
```

### Clean Reset (Wipe Database & Re-seed)
Removes all containers and named volumes, ensuring a clean slate on the next startup:
```bash
docker compose down -v
```

### Deep Clean
Removes all containers, volumes, and orphan networks:
```bash
docker compose down -v --remove-orphans
```

---

## Clean-Environment Smoke Run & Validation

To verify that your local environment is functioning correctly from a clean state:

1. **Start the stack:**
   ```bash
   docker compose up -d
   ```

2. **Check container status and health:**
   ```bash
   docker compose ps
   ```
   *Expected: `postgres`, `redis`, `stellar`, `backend`, and `frontend` report `Up` and `healthy`.*

3. **Validate backend health and dependencies:**
   ```bash
   curl -s http://localhost:3000/health | jq
   ```
   *Expected response:*
   ```json
   {
     "status": "ok",
     "service": "predifi-backend",
     "dependencies": {
       "db": "ok",
       "rpc": "ok",
       "redis": "ok",
       "price_cache": "ok"
     }
   }
   ```

4. **Validate seeded prediction pools API:**
   ```bash
   curl -s http://localhost:3000/api/v1/pools | jq
   ```
   *Expected: A JSON array containing the seeded prediction pools.*

5. **Validate frontend dashboard:**
   ```bash
   curl -I http://localhost:3001
   ```
   *Expected: `HTTP/1.1 200 OK`.*

6. **Tear down:**
   ```bash
   docker compose down
   ```
