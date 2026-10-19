# PrediFi Capacity Targets & Baseline Report (v1)

## Executive Summary
This report establishes the baseline throughput, latency SLOs, and saturation points for the `predifi-backend` HTTP API. Testing was conducted using a deterministic 4-phase load simulation (Warmup, Ramp-up, Saturation, Recovery) modeling production traffic distributions.

## 1. Workload Distribution
The load test mimics the expected production traffic split:
* **45% Market Reads:** Paginated fetching of active pools.
* **30% Prediction Prep:** Concurrent fetching of pool odds and existing market predictions.
* **15% Event Indexing:** Ingesting new `prediction-placed` events from the Stellar listener.
* **10% Settlement Tracking:** Claim ingest and user profile updates.

## 2. Identified Bottlenecks & Actionable Fixes
Based on the current architecture and middleware configurations, the backend will artificially saturate before hitting actual CPU/Memory limits. The following 5 bottlenecks must be remediated:

1. **PostgreSQL Connection Pool Exhaustion:** 
   * **Issue:** `PREDIFI_DB_MAX_CONNECTIONS` defaults to 10. Endpoints like `get_pools` and `get_market_predictions` use `tokio::try_join!` to execute multiple queries concurrently, acquiring multiple connections per request. Under burst traffic, the pool will exhaust instantly.
   * **Fix:** Increase the default to `50` and implement a dedicated read-replica pool for heavy `GET` queries.
2. **Global Cache Stampede on Pool Creation:**
   * **Issue:** In `ingest_pool_created`, calling `state.redis.invalidate_pools_cache()` wipes the entire paginated pool cache globally. Under high creation load, this forces all subsequent reads back to the database.
   * **Fix:** Invalidate only the specific categories/tags, or switch to a background cache-warming worker.
3. **Hot-Pool Row Lock Contention:**
   * **Issue:** `insert_prediction_from_event_with_pool` updates `pools.total_stake` inline via an `UPDATE` transaction for every single prediction. Popular pools will experience severe row-level lock contention.
   * **Fix:** Move `total_stake` aggregations to an asynchronous outbox worker or rely on read-time sum aggregations.
4. **Synchronous Disk I/O in Health Checks:**
   * **Issue:** The `/health` endpoint calls `sysinfo::Disks::new_with_refreshed_list()`. This is a blocking OS-level call executing on the Tokio async runtime, which will cause latency spikes under high polling rates.
   * **Fix:** Move disk checks to a background thread that updates an `Arc<AtomicU64>` periodically, and read from memory in the health endpoint.
5. **Indexer Single-IP Governor Limits:**
   * **Issue:** `RateLimitTier::Write` limits the `/indexer/*` routes to **20 requests per 60 seconds**. However, `PREDIFI_INDEXER_MAX_BATCH_SIZE` is 500. The internal Stellar listener will instantly hit a `429 Too Many Requests` during a catch-up sync.
   * **Fix:** Create an internal bypass in `GovernorLayer` for requests originating from the trusted indexer worker.

## 3. Capacity Targets (SLOs)
Once the above bottlenecks are patched, the infrastructure should be tuned to maintain the following targets:

| Workflow | Target Throughput | p50 Latency | p95 Latency | p99 Latency | Error Rate |
| :--- | :--- | :--- | :--- | :--- | :--- |
| Market Reads | 500 req/sec | < 50ms | < 150ms | < 300ms | < 0.1% |
| Prediction Prep | 250 req/sec | < 80ms | < 200ms | < 400ms | < 0.5% |
| Event Indexing | 100 req/sec | < 150ms | < 400ms | < 800ms | 0.0% |
| Settlement | 50 req/sec | < 150ms | < 400ms | < 800ms | 0.0% |

**Recovery Target:** Following a saturation spike (100% CPU or max DB connections), p95 latency must return to baseline within **15 seconds** of traffic subsiding.