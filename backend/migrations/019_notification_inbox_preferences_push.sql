-- migration-phase: expand
-- compatible-app-versions: previous and current release; adds notification APIs and separates the two existing outbox uses.
--
-- Preferences are wallet-scoped and default-deny. Push subscription payloads
-- are encrypted in the application before persistence; only the endpoint hash
-- is used for uniqueness checks.

CREATE TABLE IF NOT EXISTS notification_preferences (
    user_address   TEXT        PRIMARY KEY,
    schema_version SMALLINT    NOT NULL DEFAULT 1 CHECK (schema_version >= 0),
    preferences    JSONB       NOT NULL DEFAULT '{
        "channels": {"in_app": false, "push": false, "email": false},
        "events": {
            "pool_ending_soon": false,
            "pool_resolved": false,
            "claim_expiring": false,
            "new_pool_match": false
        }
    }'::jsonb CHECK (jsonb_typeof(preferences) = 'object'),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS browser_push_subscriptions (
    id                 BIGSERIAL   PRIMARY KEY,
    user_address       TEXT        NOT NULL,
    endpoint_hash      TEXT        NOT NULL UNIQUE,
    encrypted_payload  BYTEA       NOT NULL,
    expires_at         TIMESTAMPTZ NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_browser_push_subscriptions_active
    ON browser_push_subscriptions (user_address, expires_at);

-- Account data export/deletion: enumerate every push subscription for a wallet
-- so the export can be complete and deletion can be idempotent.
CREATE INDEX IF NOT EXISTS idx_browser_push_subscriptions_user
    ON browser_push_subscriptions (user_address, id);

CREATE TABLE IF NOT EXISTS user_notification_outbox (
    id              BIGSERIAL   PRIMARY KEY,
    event_key       TEXT        NOT NULL,
    user_address    TEXT        NOT NULL,
    notif_type      TEXT        NOT NULL,
    title           TEXT        NOT NULL,
    message         TEXT        NOT NULL,
    pool_id         BIGINT      NOT NULL,
    status          TEXT        NOT NULL DEFAULT 'pending'
                                CHECK (status IN ('pending', 'processing', 'delivered', 'failed')),
    attempts        INTEGER     NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    max_attempts    INTEGER     NOT NULL DEFAULT 5 CHECK (max_attempts > 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_error      TEXT,
    claimed_at      TIMESTAMPTZ,
    claimed_by      TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (event_key, user_address)
);

CREATE INDEX IF NOT EXISTS idx_user_notification_outbox_due
    ON user_notification_outbox (next_attempt_at, id)
    WHERE status = 'pending';

CREATE INDEX IF NOT EXISTS idx_user_notification_outbox_claimed
    ON user_notification_outbox (claimed_at)
    WHERE status = 'processing';

-- Account data export/deletion: scan all inbox rows for a wallet in a stable
-- order without relying on the (user_address, created_at) inbox index.
CREATE INDEX IF NOT EXISTS idx_user_notification_outbox_user
    ON user_notification_outbox (user_address, id);

-- Preserve pending intents from the legacy table while moving the in-app
-- notification worker onto its own schema. Terminal rows remain available in
-- the legacy table for the existing retention process.
INSERT INTO user_notification_outbox
    (event_key, user_address, notif_type, title, message, pool_id,
     status, attempts, max_attempts, next_attempt_at, last_error,
     claimed_at, claimed_by, created_at, updated_at)
SELECT
    'legacy:' || id::text,
    user_address,
    notif_type,
    title,
    message,
    pool_id,
    CASE status
        WHEN 'claimed' THEN 'pending'
        WHEN 'sent' THEN 'delivered'
        ELSE status
    END,
    attempts,
    max_attempts,
    next_attempt_at,
    last_error,
    NULL,
    NULL,
    created_at,
    updated_at
FROM notification_outbox
WHERE status IN ('pending', 'claimed', 'sent', 'failed')
ON CONFLICT (event_key, user_address) DO NOTHING;

CREATE INDEX IF NOT EXISTS idx_notifications_user_inbox
    ON notifications (user_address, created_at DESC, id DESC);

-- Account data export/deletion: stable, complete enumeration of a wallet's
-- inbox entries by primary key for export and idempotent deletion.
CREATE INDEX IF NOT EXISTS idx_notifications_user_id
    ON notifications (user_address, id);

-- The generic Stellar worker queue used to share the notification table name
-- despite having a different payload and lease schema. Keep it separate.
CREATE TABLE IF NOT EXISTS background_job_outbox (
    id           BIGSERIAL   PRIMARY KEY,
    job_id       TEXT        NOT NULL UNIQUE,
    kind         TEXT        NOT NULL,
    payload      TEXT        NOT NULL,
    ledger       BIGINT,
    attempts     INTEGER     NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    available_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    status       TEXT        NOT NULL DEFAULT 'pending'
                             CHECK (status IN ('pending', 'in_flight', 'delivered', 'failed')),
    locked_until TIMESTAMPTZ,
    delivered_at TIMESTAMPTZ,
    failed_at    TIMESTAMPTZ,
    last_error   TEXT
);

CREATE INDEX IF NOT EXISTS idx_background_job_outbox_due
    ON background_job_outbox (available_at, id)
    WHERE status = 'pending';
