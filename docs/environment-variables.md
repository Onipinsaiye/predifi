# Environment Variables Reference

Complete reference for all environment variables used across the PrediFi full-stack architecture.

---

## 1. Root Docker Compose Variables

These variables configure the local Docker Compose full-stack environment. They can be set in a `.env` file at the repository root (see `.env.example`).

| Variable | Default | Purpose |
| :--- | :--- | :--- |
| `BACKEND_PORT` | `3000` | Host port mapped to the Axum backend service (`3000:3000`). |
| `FRONTEND_PORT` | `3001` | Host port mapped to the Next.js frontend service (`3001:3000`). |
| `POSTGRES_PORT` | `5432` | Host port mapped to PostgreSQL. |
| `REDIS_PORT` | `6379` | Host port mapped to Redis. |
| `STELLAR_PORT` | `8000` | Host port mapped to Stellar Horizon & Soroban RPC. |
| `POSTGRES_USER` | `postgres` | PostgreSQL username. |
| `POSTGRES_PASSWORD` | `postgres` | PostgreSQL password. |
| `POSTGRES_DB` | `predifi` | PostgreSQL database name. |
| `PREDIFI_STELLAR_RPC_URL` | `http://stellar:8000/rpc` | Soroban RPC endpoint used by backend containers. |
| `NEXT_PUBLIC_API_BASE_URL` | `http://localhost:3000` | Base URL used by the frontend to contact the backend. |

---

## 2. Backend Environment Variables

These variables configure the Rust Axum backend (`backend/src/config.rs`). They can be placed in `backend/.env` for local execution or supplied via container environment variables.

### Server & Runtime

| Variable | Type | Default | Required | Description |
| :--- | :--- | :--- | :---: | :--- |
| `PREDIFI_APP_HOST` | String | `0.0.0.0` | No | Network interface address to bind to. |
| `PREDIFI_APP_PORT` | u16 | `3000` | No | TCP port the HTTP/WebSocket server listens on. |
| `PREDIFI_APP_ENV` | String | `development` | No | Deployment environment (`development`, `staging`, `production`). |
| `RUST_LOG` | String | `info` | No | Tracing filter directive (e.g. `info`, `debug`, `warn`). |
| `PREDIFI_SHUTDOWN_TIMEOUT_SECS` | u64 | `30` | No | Grace period in seconds for in-flight requests to drain during SIGINT/SIGTERM. |

### Database (PostgreSQL)

| Variable | Type | Default | Required | Description |
| :--- | :--- | :--- | :---: | :--- |
| `PREDIFI_DATABASE_URL` | String | `postgres://postgres:postgres@localhost:5432/predifi` | Yes (in prod) | PostgreSQL connection string. |
| `PREDIFI_DB_MAX_CONNECTIONS` | u32 | `10` | No | Maximum pool connections. |
| `PREDIFI_DB_MIN_CONNECTIONS` | u32 | `1` | No | Minimum idle pool connections. |
| `PREDIFI_DB_ACQUIRE_TIMEOUT_SECS` | u64 | `30` | No | Maximum wait time to acquire an idle connection from the pool. |
| `PREDIFI_DB_CONNECT_TIMEOUT_SECS` | u64 | `10` | No | TCP/TLS connection handshake timeout. |
| `PREDIFI_DB_CONNECT_MAX_ATTEMPTS` | u32 | `5` | No | Number of connection retries at server startup. |
| `PREDIFI_DB_CONNECT_BASE_DELAY_MS` | u64 | `200` | No | Initial exponential backoff delay between startup DB connection attempts. |
| `PREDIFI_DB_CONNECT_MAX_DELAY_MS` | u64 | `5000` | No | Maximum backoff delay for DB connection retries. |

### Cache & Rate Limiting (Redis)

| Variable | Type | Default | Required | Description |
| :--- | :--- | :--- | :---: | :--- |
| `PREDIFI_REDIS_URL` | String | `redis://localhost:6379` | No | Redis connection URL. |
| `PREDIFI_REDIS_REQUIRED` | bool | `false` | No | If `true`, server aborts startup when Redis is unreachable; if `false`, runs in degraded mode. |
| `PREDIFI_POOL_NEGATIVE_CACHE_TTL_SECS` | u64 | `5` | No | Cache duration in seconds for non-existent pool lookups. |

### Blockchain (Stellar & Soroban RPC)

| Variable | Type | Default | Required | Description |
| :--- | :--- | :--- | :---: | :--- |
| `PREDIFI_STELLAR_RPC_URL` | String | `https://soroban-testnet.stellar.org` | No | Soroban JSON-RPC endpoint. |
| `PREDIFI_RPC_HEALTH_TIMEOUT_SECS` | u64 | `2` | No | Timeout for the `/health` endpoint's `getHealth` RPC check. |
| `PREDIFI_RPC_HEALTH_RETRY_COUNT` | u8 | `3` | No | Number of retry attempts when probing Soroban RPC health. |
| `RPC_TIMEOUT_SECS` | u64 | `30` | No | General timeout for background contract event ingestion. |
| `PREDIFI_INDEXER_MAX_BATCH_SIZE` | usize | `500` | No | Maximum number of ledger events fetched per indexer cycle. |

### Security & Authentication

| Variable | Type | Default | Required | Description |
| :--- | :--- | :--- | :---: | :--- |
| `PREDIFI_CORS_ALLOWED_ORIGINS` | String | `http://localhost:3000,http://localhost:5173,https://predifi.app` | No | Comma-separated list of allowed HTTP/HTTPS origins. |
| `PREDIFI_WS_ALLOWED_ORIGINS` | String | *empty* (permissive) | No | Comma-separated list of allowed WebSocket origins. |
| `PREDIFI_SECRET_KEY` | String | `predifi-dev-secret-do-not-use-in-production-32` | No (in dev) | JWT signing secret (minimum 32 bytes required in production). |
| `PREDIFI_JWT_KEY_VERSION` | u64 | `0` | No | Version number for JWT secret key rotation. |

### Protocol & Fees

| Variable | Type | Default | Required | Description |
| :--- | :--- | :--- | :---: | :--- |
| `PREDIFI_TREASURY_FEE_BPS` | u32 | `300` (3%) | No | Platform protocol fee basis points. |
| `PREDIFI_REFERRAL_FEE_BPS` | u32 | `5000` (50%) | No | Share of platform fee allocated to referrers. |
| `PREDIFI_SENTRY_DSN` | String | *unset* | No | Sentry DSN for error and panic tracking. |

---

## 3. Frontend Environment Variables

These variables configure the Next.js web application (`frontend/.env.example` -> `frontend/.env.local`). Next.js requires the `NEXT_PUBLIC_` prefix for variables accessible in the browser.

| Variable | Default | Purpose |
| :--- | :--- | :--- |
| `NEXT_PUBLIC_API_BASE_URL` | `http://localhost:3000` | Base URL for the PrediFi REST and WebSocket API. |
| `NEXT_PUBLIC_STELLAR_NETWORK` | `TESTNET` | Stellar network mode (`TESTNET`, `MAINNET`, `FUTURENET`, `STANDALONE`). |
| `WATCHPACK_POLLING` | `true` | Enables polling-based file watching inside Docker containers on Windows/macOS. |
