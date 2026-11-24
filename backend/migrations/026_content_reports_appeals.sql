-- Migration 026: Content reports and appeals
-- Implements user-facing content reporting and a bounded appeal path.
-- Content moderation NEVER alters on-chain funds or resolution; this schema
-- only records off-chain moderation state and an audit trail.

BEGIN;

-- ---------------------------------------------------------------------------
-- content_reports
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS content_reports (
    id                BIGSERIAL PRIMARY KEY,
    reporter_address  TEXT        NOT NULL,
    target_type       TEXT        NOT NULL,
    target_id         TEXT        NOT NULL,
    reason            TEXT        NOT NULL,
    details           TEXT,
    status            TEXT        NOT NULL DEFAULT 'open',
    dedupe_key        TEXT        NOT NULL,
    -- Hash of the moderation_audit_log entry that records creation of this
    -- report. Preserves an immutable audit trail without a hard FK (the audit
    -- log may be append-only / partitioned).
    audit_record_hash TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT content_reports_target_type_check
        CHECK (target_type IN ('market', 'comment', 'profile', 'other')),

    CONSTRAINT content_reports_reason_check
        CHECK (reason IN (
            'misleading',
            'fraud',
            'spam',
            'harassment',
            'illegal',
            'other'
        )),

    -- Bounded report state machine.
    CONSTRAINT content_reports_status_check
        CHECK (status IN (
            'open',
            'under_review',
            'resolved_actioned',
            'resolved_no_action',
            'rejected',
            'appealed'
        )),

    CONSTRAINT content_reports_reporter_address_check
        CHECK (length(btrim(reporter_address)) > 0),

    CONSTRAINT content_reports_target_id_check
        CHECK (length(btrim(target_id)) > 0),

    CONSTRAINT content_reports_dedupe_key_check
        CHECK (length(btrim(dedupe_key)) > 0),

    -- Deduplication: one report per reporter/target/reason combination.
    CONSTRAINT content_reports_dedupe_key_unique UNIQUE (dedupe_key)
);

-- Reviewer access: quickly find reports awaiting review.
CREATE INDEX IF NOT EXISTS idx_content_reports_status_created
    ON content_reports (status, created_at);

-- Reviewer access: filter by target.
CREATE INDEX IF NOT EXISTS idx_content_reports_target
    ON content_reports (target_type, target_id);

-- Reporter visibility: list a reporter's own reports.
CREATE INDEX IF NOT EXISTS idx_content_reports_reporter
    ON content_reports (reporter_address, created_at DESC);

-- Deduplication lookups.
CREATE INDEX IF NOT EXISTS idx_content_reports_dedupe_key
    ON content_reports (dedupe_key);

-- Audit trail linkage.
CREATE INDEX IF NOT EXISTS idx_content_reports_audit_hash
    ON content_reports (audit_record_hash);

-- ---------------------------------------------------------------------------
-- content_appeals
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS content_appeals (
    id                BIGSERIAL PRIMARY KEY,
    report_id         BIGINT      NOT NULL,
    appellant_address TEXT        NOT NULL,
    message           TEXT        NOT NULL,
    status            TEXT        NOT NULL DEFAULT 'pending',
    reviewer_address  TEXT,
    decision_note     TEXT,
    -- Hash of the moderation_audit_log entry recording the appeal decision.
    audit_record_hash TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT content_appeals_report_fk
        FOREIGN KEY (report_id) REFERENCES content_reports (id)
        ON DELETE RESTRICT,

    -- Bounded appeal state machine.
    CONSTRAINT content_appeals_status_check
        CHECK (status IN (
            'pending',
            'under_review',
            'upheld',
            'overturned',
            'withdrawn'
        )),

    CONSTRAINT content_appeals_appellant_address_check
        CHECK (length(btrim(appellant_address)) > 0),

    CONSTRAINT content_appeals_message_check
        CHECK (length(btrim(message)) > 0),

    -- Reviewer must be recorded once a decision is reached.
    CONSTRAINT content_appeals_decision_requires_reviewer
        CHECK (
            status NOT IN ('upheld', 'overturned')
            OR (reviewer_address IS NOT NULL AND length(btrim(reviewer_address)) > 0)
        ),

    -- A decision note is required for terminal decisions.
    CONSTRAINT content_appeals_decision_requires_note
        CHECK (
            status NOT IN ('upheld', 'overturned')
            OR (decision_note IS NOT NULL AND length(btrim(decision_note)) > 0)
        ),

    -- Only one appeal per report (bounded appeal path).
    CONSTRAINT content_appeals_report_id_unique UNIQUE (report_id)
);

-- Reviewer access: appeals awaiting review.
CREATE INDEX IF NOT EXISTS idx_content_appeals_status_created
    ON content_appeals (status, created_at);

-- Appellant visibility: list an appellant's own appeals.
CREATE INDEX IF NOT EXISTS idx_content_appeals_appellant
    ON content_appeals (appellant_address, created_at DESC);

-- Reviewer workload lookups.
CREATE INDEX IF NOT EXISTS idx_content_appeals_reviewer
    ON content_appeals (reviewer_address, created_at DESC);

-- Audit trail linkage.
CREATE INDEX IF NOT EXISTS idx_content_appeals_audit_hash
    ON content_appeals (audit_record_hash);

-- ---------------------------------------------------------------------------
-- updated_at maintenance
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION set_content_moderation_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at := now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_content_reports_updated_at ON content_reports;
CREATE TRIGGER trg_content_reports_updated_at
    BEFORE UPDATE ON content_reports
    FOR EACH ROW
    EXECUTE FUNCTION set_content_moderation_updated_at();

DROP TRIGGER IF EXISTS trg_content_appeals_updated_at ON content_appeals;
CREATE TRIGGER trg_content_appeals_updated_at
    BEFORE UPDATE ON content_appeals
    FOR EACH ROW
    EXECUTE FUNCTION set_content_moderation_updated_at();

COMMIT;