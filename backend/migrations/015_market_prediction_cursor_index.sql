-- Support stable keyset traversal for market prediction feeds.
CREATE INDEX IF NOT EXISTS idx_predictions_market_cursor
    ON predictions (pool_id, created_at DESC, id DESC);
