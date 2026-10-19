-- Keep a canonical hash checkpoint for every ledger containing an observed
-- contract event. Event payloads are staged until their ledger reaches the
-- listener's confirmation depth.
CREATE TABLE IF NOT EXISTS app_state (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

ALTER TABLE pools
    ADD COLUMN IF NOT EXISTS stellar_created_event_key TEXT,
    ADD COLUMN IF NOT EXISTS stellar_state_event_key   TEXT,
    ADD COLUMN IF NOT EXISTS stellar_canonical         BOOLEAN NOT NULL DEFAULT TRUE;

ALTER TABLE referrals
    ADD COLUMN IF NOT EXISTS stellar_event_key TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_referrals_stellar_event_key
    ON referrals (stellar_event_key)
    WHERE stellar_event_key IS NOT NULL;

CREATE TABLE IF NOT EXISTS stellar_listener_checkpoints (
    ledger_sequence BIGINT PRIMARY KEY,
    ledger_hash     TEXT        NOT NULL,
    canonical       BOOLEAN     NOT NULL DEFAULT TRUE,
    observed_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finalized_at    TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS stellar_listener_events (
    event_id        TEXT        NOT NULL,
    ledger_sequence BIGINT      NOT NULL,
    ledger_hash     TEXT        NOT NULL,
    payload         JSONB       NOT NULL,
    status          TEXT        NOT NULL DEFAULT 'pending'
                                CHECK (status IN ('pending', 'processed', 'quarantined')),
    observed_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processed_at    TIMESTAMPTZ,
    quarantined_at  TIMESTAMPTZ,
    PRIMARY KEY (event_id, ledger_hash)
);

ALTER TABLE predictions
    ADD COLUMN IF NOT EXISTS stellar_event_key TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_predictions_stellar_event_key
    ON predictions (stellar_event_key)
    WHERE stellar_event_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_stellar_listener_events_pending
    ON stellar_listener_events (ledger_sequence)
    WHERE status = 'pending';

CREATE INDEX IF NOT EXISTS idx_stellar_listener_checkpoints_canonical
    ON stellar_listener_checkpoints (ledger_sequence)
    WHERE canonical;