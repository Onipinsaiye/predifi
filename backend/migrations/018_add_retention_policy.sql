-- Retention and deletion policy for private user records.
--
-- Classification (see docs/data-inventory.md):
--   * Public chain data (pools, predictions, referrals, projected contract
--     events) is NOT touched by this policy. It mirrors on-chain state and is
--   reproducible by re-indexing.
--   * Private platform data is classified below with an explicit retention
--     window and deletion behaviour.

CREATE TABLE IF NOT EXISTS retention_policy (
    record_type     TEXT PRIMARY KEY,
    classification  TEXT NOT NULL CHECK (classification IN ('public_chain', 'private', 'operational', 'audit')),
    retention_days  INTEGER,            -- NULL = indefinite (public chain / audit)
    deletion_mode   TEXT NOT NULL CHECK (deletion_mode IN ('hard_delete', 'tombstone', 'ttl_purge', 'none')),
    description     TEXT NOT NULL
);

INSERT INTO retention_policy (record_type, classification, retention_days, deletion_mode, description) VALUES
    ('pools',                    'public_chain', NULL, 'none',        'Mirrors chain state; reproducible by re-index'),
    ('predictions',              'public_chain', NULL, 'none',        'Mirrors chain state; reproducible by re-index'),
    ('referrals',                'public_chain', NULL, 'none',        'Mirrors chain state; reproducible by re-index'),
    ('contract_events',          'public_chain', NULL, 'none',        'Raw projection of chain events; replayable'),
    ('profiles',                 'private',      NULL, 'hard_delete', 'Deleted on account deletion, cascades to dependents'),
    ('discussions',              'private',      NULL, 'tombstone',   'Content tombstoned on account deletion'),
    ('tags',                     'private',      NULL, 'hard_delete', 'Removed on account deletion'),
    ('claims',                   'private',      NULL, 'hard_delete', 'Revoked and removed on account deletion'),
    ('notification_outbox',      'private',      7,    'ttl_purge',   'Purged after delivery or expiry window'),
    ('notification_deliveries',  'private',      30,   'ttl_purge',   'Delivery status retained for bounded retry window'),
    ('exports',                  'private',      2,    'ttl_purge',   'Export artifacts expire after download window'),
    ('export_requests',          'private',      2,    'ttl_purge',   'Export request metadata purged after download window'),
    ('deletion_requests',        'private',      NULL, 'tombstone',   'Deletion request state tombstoned; no personal data retained'),
    ('idempotency_keys',         'operational',  1,    'ttl_purge',   'Request-scoped TTL'),
    ('background_jobs',          'operational',  14,   'ttl_purge',   'Completed job payloads purged after short retention'),
    ('token_revocation',         'operational',  NULL, 'ttl_purge',   'Retained until token would have expired'),
    ('credential_rotation_audit','audit',        365,  'ttl_purge',   'Operational audit of key rotation; identifiers only, never secrets');

-- Deletion audit log. Records that a deletion happened without retaining the
-- deleted personal data itself (only an opaque subject reference and counts).
CREATE TABLE IF NOT EXISTS retention_audit (
    id            BIGSERIAL PRIMARY KEY,
    record_type   TEXT NOT NULL,
    subject_ref   TEXT,                 -- salted hash of the user id, not the raw id
    action        TEXT NOT NULL CHECK (action IN ('hard_delete', 'tombstone', 'ttl_purge')),
    affected_rows BIGINT NOT NULL,
    executed_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_retention_audit_executed_at ON retention_audit (executed_at);

-- Account data export / deletion audit evidence.
--
-- Policy requires that we can prove an export or deletion was performed for a
-- subject without retaining the deleted personal data. We therefore store only:
--   * an opaque, salted subject reference (never the raw user id),
--   * the action performed and its outcome,
--   * aggregate counts of affected records per record type,
--   * timestamps.
-- No field values, payloads or export contents are stored here.
CREATE TABLE IF NOT EXISTS account_data_audit (
    id            BIGSERIAL PRIMARY KEY,
    subject_ref   TEXT NOT NULL,        -- salted hash of the user id, not the raw id
    action        TEXT NOT NULL CHECK (action IN ('export', 'delete')),
    outcome       TEXT NOT NULL CHECK (outcome IN ('completed', 'partial', 'failed')),
    record_counts JSONB NOT NULL DEFAULT '{}'::jsonb,  -- {record_type: affected_rows}
    requested_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at  TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_account_data_audit_subject_ref ON account_data_audit (subject_ref);
CREATE INDEX IF NOT EXISTS idx_account_data_audit_requested_at ON account_data_audit (requested_at);

-- Idempotent deletion support: track that a subject's private data has already
-- been deleted so repeated requests are no-ops. Stores only the opaque subject
-- reference and a timestamp; no personal data.
CREATE TABLE IF NOT EXISTS account_deletion_ledger (
    subject_ref   TEXT PRIMARY KEY,     -- salted hash of the user id, not the raw id
    deleted_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
