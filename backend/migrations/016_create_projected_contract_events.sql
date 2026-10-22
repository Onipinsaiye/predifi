-- migration-phase: expand
-- compatible-app-versions: previous and current release; adds a new table and a unique index the previous release's ON CONFLICT clause already expects.
--
-- Canonical deduplication keys for indexed contract events (#45).
--
-- A contract event is uniquely identified on-chain by its network, emitting
-- contract, ledger, transaction, and position within that transaction. The
-- event projector inserts the key in the same transaction as the domain writes
-- and skips the projection when the key already exists, so retries and
-- overlapping listener windows never duplicate pools, predictions, or
-- notifications.

CREATE TABLE IF NOT EXISTS projected_contract_events (
    network      TEXT        NOT NULL,
    contract_id  TEXT        NOT NULL,
    ledger       BIGINT      NOT NULL CHECK (ledger >= 0),
    tx_hash      TEXT        NOT NULL,
    event_index  INTEGER     NOT NULL CHECK (event_index >= 0),
    event_type   TEXT        NOT NULL,
    projected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (network, contract_id, ledger, tx_hash, event_index)
);

-- `notifications::enqueue_notification` uses
-- `ON CONFLICT (user_address, pool_id, notif_type)`, which requires a matching
-- unique index; this also caps the outbox at one intent per user/pool/type.
CREATE UNIQUE INDEX IF NOT EXISTS idx_notification_outbox_dedupe
    ON notification_outbox (user_address, pool_id, notif_type);
