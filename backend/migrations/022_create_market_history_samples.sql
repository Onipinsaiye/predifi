-- migration-phase: expand
-- compatible-app-versions: previous and current release; adds a new table only.
--
-- Historical market-value time series, hot tier (#313, #314).
--
-- Samples are bucketed cumulative-stake values for a pool, aggregated from
-- `portfolio_activity_ledger` (`event_type = 'prediction_placed'`) — the
-- same canonical, checkpoint-gated projection table backing the wallet
-- ledger/snapshot/export APIs (#309-#312). This table is a materialized
-- cache of that aggregation (see `crate::db::market_history`), not an
-- independent price feed: every row is reproducible from the ledger.
--
-- `(pool_id, bucket_width, bucket_start)` is unique so recomputing a bucket
-- overwrites it in place rather than creating a duplicate or drifting
-- history, mirroring the `portfolio_snapshots` recomputation contract.
--
-- Old rows are moved out of this table into `market_history_archives`
-- (cold tier, see 023_create_market_history_archives.sql) once they age
-- past a pool's retention window; the bounded query API in
-- `crate::db::market_history` reads transparently from both tiers.

CREATE TABLE IF NOT EXISTS market_history_samples (
    pool_id            BIGINT      NOT NULL,
    -- '1m' | '5m' | '1h' | '1d' — see `crate::db::market_history::BucketWidth`.
    bucket_width       TEXT        NOT NULL,
    -- Start of the bucket's interval, UTC. Floored to an exact multiple of
    -- the bucket width since the Unix epoch, so boundaries never shift with
    -- a local daylight-saving transition.
    bucket_start       TIMESTAMPTZ NOT NULL,
    -- Cumulative stake as of the end of the interval (bucket_start + width),
    -- an exact integer (never a float).
    value              BIGINT      NOT NULL,
    -- Committed listener checkpoint this sample was derived from (source
    -- cursor), same convention as `portfolio_snapshots.checkpoint_ledger`.
    checkpoint_ledger  BIGINT      NOT NULL CHECK (checkpoint_ledger >= 0),
    computed_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (pool_id, bucket_width, bucket_start)
);
