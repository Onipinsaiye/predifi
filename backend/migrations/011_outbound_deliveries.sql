-- Migration: outbound notification delivery tracking with retry + DLQ (#63)
--
-- The existing `notifications` table is the in-app feed: one row per
-- (user, pool, type) that the sweep inserts idempotently. This migration
-- adds a SEPARATE table for outbound delivery attempts (email / webhook /
-- push in a future version), where each attempt needs its own state.
--
-- One notification can produce zero or more outbound deliveries (e.g. an
-- email channel and a webhook channel). Each delivery tracks its own
-- retry state, so a permanently-failing email does not affect a working
-- webhook.
--
-- Crash recovery: `status = 'pending' AND next_attempt_at <= NOW()` is the
-- startup query. `attempts` and `failure_category` distinguish transient
-- failures from permanent ones, and `idempotency_key` lets sinks dedupe
-- at-least-once deliveries.

CREATE TABLE IF NOT EXISTS outbound_deliveries (
    id                  BIGSERIAL    PRIMARY KEY,
    notification_id     BIGINT       NOT NULL REFERENCES notifications (id) ON DELETE CASCADE,
    channel             VARCHAR(32)  NOT NULL
                                     CHECK (channel IN ('email', 'webhook', 'push', 'sms')),
    payload             JSONB        NOT NULL,
    idempotency_key     TEXT         NOT NULL,
    attempts            INT          NOT NULL DEFAULT 0,
    next_attempt_at     TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    status              VARCHAR(16)  NOT NULL DEFAULT 'pending'
                                     CHECK (status IN ('pending', 'sent', 'dead')),
    failure_category    VARCHAR(32)  CHECK (failure_category IN (
                                         'transient', 'rate_limited', 'permanent', 'auth'
                                     )),
    last_error          TEXT,
    created_at          TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- Startup recovery query: find all deliveries that are due.
CREATE INDEX IF NOT EXISTS idx_outbound_deliveries_pending_due
    ON outbound_deliveries (next_attempt_at)
    WHERE status = 'pending';

-- Operator query: list dead deliveries newest first.
CREATE INDEX IF NOT EXISTS idx_outbound_deliveries_dead
    ON outbound_deliveries (updated_at DESC)
    WHERE status = 'dead';

-- Idempotency: a sink must never deliver the same logical message twice.
CREATE UNIQUE INDEX IF NOT EXISTS idx_outbound_deliveries_idempotency
    ON outbound_deliveries (idempotency_key);

-- Convenience for per-notification history.
CREATE INDEX IF NOT EXISTS idx_outbound_deliveries_notification
    ON outbound_deliveries (notification_id);
