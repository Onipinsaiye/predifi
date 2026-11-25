-- Migration 025: feedback_submissions
-- Structured feedback channel for product issues.
-- Stores opt-in feedback with category, free-text message, consent flag,
-- and optional diagnostic metadata. Wallet addresses are redacted by default
-- before insert; no wallet address column is stored.

BEGIN;

CREATE TABLE IF NOT EXISTS feedback_submissions (
    id              BIGSERIAL PRIMARY KEY,
    category        TEXT NOT NULL,
    message         TEXT NOT NULL,
    consent         BOOLEAN NOT NULL DEFAULT FALSE,
    diagnostics     JSONB,
    redacted        BOOLEAN NOT NULL DEFAULT TRUE,
    dedupe_hash     TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- Enum-like category constraint.
    CONSTRAINT feedback_submissions_category_check
        CHECK (category IN (
            'bug',
            'feature_request',
            'performance',
            'usability',
            'documentation',
            'security',
            'other'
        )),

    -- Free-text length limits: non-empty and bounded.
    CONSTRAINT feedback_submissions_message_length_check
        CHECK (char_length(message) >= 1 AND char_length(message) <= 5000),

    -- Consent must be explicitly granted for submission.
    CONSTRAINT feedback_submissions_consent_check
        CHECK (consent = TRUE),

    -- Diagnostics, when present, must be a JSON object.
    CONSTRAINT feedback_submissions_diagnostics_object_check
        CHECK (diagnostics IS NULL OR jsonb_typeof(diagnostics) = 'object'),

    -- Client-generated dedupe hash: bounded, non-empty.
    CONSTRAINT feedback_submissions_dedupe_hash_check
        CHECK (char_length(dedupe_hash) >= 8 AND char_length(dedupe_hash) <= 128)
);

-- Dedupe: prevent duplicate submissions from the same client payload.
CREATE UNIQUE INDEX IF NOT EXISTS ux_feedback_submissions_dedupe_hash
    ON feedback_submissions (dedupe_hash);

-- Triage indexes.
CREATE INDEX IF NOT EXISTS ix_feedback_submissions_created_at
    ON feedback_submissions (created_at DESC);

CREATE INDEX IF NOT EXISTS ix_feedback_submissions_category_created_at
    ON feedback_submissions (category, created_at DESC);

CREATE INDEX IF NOT EXISTS ix_feedback_submissions_redacted
    ON feedback_submissions (redacted)
    WHERE redacted = FALSE;

COMMIT;