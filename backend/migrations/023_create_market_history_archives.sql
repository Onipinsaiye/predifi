-- migration-phase: expand
-- compatible-app-versions: previous and current release; adds a new table only.
--
-- Cold-tier archive for market-history samples (#314).
--
-- `archive_market_history` moves a contiguous range of old
-- `market_history_samples` rows for one `(pool_id, bucket_width)` into a
-- single checksummed row here: `payload` is the canonical JSON array of
-- archived samples and `checksum` is its SHA-256 hex digest, so a restore
-- (`restore_market_history_archive`) can detect corruption before trusting
-- the payload. This is a simple checksummed blob format reusing Postgres
-- storage (the same "inline artifact" approach `portfolio_exports` already
-- uses for its CSV artifacts), not a new storage engine.
--
-- `bucket_start_min`/`bucket_start_max` let the bounded query API
-- (`crate::db::market_history::get_market_history`) find a candidate
-- archive for a requested range without deserializing every payload.

CREATE TABLE IF NOT EXISTS market_history_archives (
    id                 BIGSERIAL   PRIMARY KEY,
    pool_id            BIGINT      NOT NULL,
    bucket_width       TEXT        NOT NULL,
    bucket_start_min   TIMESTAMPTZ NOT NULL,
    bucket_start_max   TIMESTAMPTZ NOT NULL,
    row_count          INTEGER     NOT NULL CHECK (row_count > 0),
    checksum           TEXT        NOT NULL,
    payload            TEXT        NOT NULL,
    archived_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Range lookup: `WHERE pool_id = $1 AND bucket_width = $2 AND
-- bucket_start_min < $4 AND bucket_start_max >= $3` (overlap test), matching
-- `lookup_archived_samples`'s query exactly.
CREATE INDEX IF NOT EXISTS idx_market_history_archives_lookup
    ON market_history_archives (pool_id, bucket_width, bucket_start_min, bucket_start_max);
