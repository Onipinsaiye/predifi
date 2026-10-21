-- Idempotency records for market mutations (#48).
--
-- A record is scoped to (wallet, operation, idempotency_key): the same key
-- sent by a different wallet, or to a different endpoint, is an independent
-- record and never replays someone else's response.
--
-- Lifecycle:
--   in_progress  the request that inserted the row is executing; it holds a
--                short lease (`locked_until`) identified by `owner_token`.
--                A crashed owner's lease expires and an exact retry may take
--                the record over.
--   completed    the final response is stored and replayed verbatim to exact
--                retries until `expires_at`.
-- Rows past `expires_at` are treated as absent and purged periodically.

CREATE TABLE IF NOT EXISTS idempotency_keys (
    wallet                 TEXT        NOT NULL,
    operation              TEXT        NOT NULL,
    idempotency_key        TEXT        NOT NULL,
    -- SHA-256 over method, path and canonicalised JSON body.
    request_fingerprint    TEXT        NOT NULL,
    state                  TEXT        NOT NULL,
    owner_token            TEXT,
    locked_until           TIMESTAMPTZ,
    response_status        SMALLINT,
    response_content_type  TEXT,
    response_body          BYTEA,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at           TIMESTAMPTZ,
    expires_at             TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (wallet, operation, idempotency_key),
    CONSTRAINT idempotency_keys_state_check
        CHECK (state IN ('in_progress', 'completed'))
);

-- Retention sweeps delete by expiry.
CREATE INDEX IF NOT EXISTS idx_idempotency_keys_expires_at
    ON idempotency_keys (expires_at);
