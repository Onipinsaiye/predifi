-- migration-phase: expand
-- compatible-app-versions: previous and current release; loosens an existing
-- CHECK constraint and adds a new table, both backward compatible.
--
-- Bounded asynchronous portfolio exports (#312).
--
-- Export jobs are leased through the existing `background_jobs` table (see
-- `013_create_background_jobs.sql`) with `kind = 'portfolio_export'`; this
-- table holds the export-specific metadata the generic job queue doesn't
-- know about: the requesting wallet, enforced row/time bounds, progress,
-- the checkpoint the export was read from, and the artifact itself (stored
-- inline since this deployment has no object storage) plus its expiry.
--
-- `background_jobs.status` gains `cancelled` as a second terminal state
-- alongside `succeeded`/`failed`, so a caller-requested cancellation is
-- representable without overloading `failed` (which implies an error).

ALTER TABLE background_jobs DROP CONSTRAINT IF EXISTS background_jobs_status_check;
ALTER TABLE background_jobs ADD CONSTRAINT background_jobs_status_check
    CHECK (status IN ('pending', 'running', 'succeeded', 'failed', 'cancelled'));

CREATE TABLE IF NOT EXISTS portfolio_exports (
    id                 BIGSERIAL   PRIMARY KEY,
    job_id             BIGINT      NOT NULL REFERENCES background_jobs (id) ON DELETE CASCADE,
    user_address       TEXT        NOT NULL,
    status             TEXT        NOT NULL DEFAULT 'pending',
    -- Checkpoint ledger the export was streamed from (set once the job runs).
    checkpoint_ledger  BIGINT,
    row_limit          INTEGER     NOT NULL,
    time_limit_secs    INTEGER     NOT NULL,
    rows_written       INTEGER     NOT NULL DEFAULT 0,
    -- Inline artifact storage (CSV text), bounded by row_limit.
    artifact_content   TEXT,
    expires_at         TIMESTAMPTZ,
    error              TEXT,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at         TIMESTAMPTZ,
    completed_at       TIMESTAMPTZ,
    CONSTRAINT portfolio_exports_status_check
        CHECK (status IN ('pending', 'running', 'succeeded', 'failed', 'cancelled'))
);

CREATE INDEX IF NOT EXISTS idx_portfolio_exports_wallet
    ON portfolio_exports (user_address, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_portfolio_exports_job
    ON portfolio_exports (job_id);
