-- Migration 011: Create listener checkpoints table and ensure app_state exists
--
-- Persists Stellar event listener checkpoints atomically alongside indexed events.
-- Also creates/ensures app_state exists for backward compatibility.

CREATE TABLE IF NOT EXISTS listener_checkpoints (
    id VARCHAR(128) PRIMARY KEY,
    last_ledger BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS app_state (
    key VARCHAR(128) PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for timestamp-based monitoring or health checks if needed
CREATE INDEX IF NOT EXISTS idx_listener_checkpoints_updated_at ON listener_checkpoints (updated_at);
