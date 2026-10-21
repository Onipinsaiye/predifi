-- Leased background jobs shared by every backend replica (#65).
--
-- Workers claim a job by taking a time-bounded lease. Each claim increments
-- `fencing_token`; completion, failure and lease extension must present the
-- token they were issued, so a worker whose lease expired (paused, partitioned
-- or killed) can never overwrite the outcome recorded by the worker that
-- reclaimed the job.
--
-- Lifecycle:
--   pending    waiting for `available_at`
--   running    leased to `lease_owner` until `lease_expires_at`; an expired
--              lease is reclaimable by any worker
--   succeeded  terminal success
--   failed     terminal failure (terminal error or attempts exhausted)

CREATE TABLE IF NOT EXISTS background_jobs (
    id                BIGSERIAL   PRIMARY KEY,
    queue             TEXT        NOT NULL,
    kind              TEXT        NOT NULL,
    -- Optional producer-side de-duplication key (e.g. a Stellar event id).
    dedupe_key        TEXT        UNIQUE,
    payload           JSONB       NOT NULL DEFAULT '{}'::jsonb,
    status            TEXT        NOT NULL DEFAULT 'pending',
    attempts          INTEGER     NOT NULL DEFAULT 0,
    max_attempts      INTEGER     NOT NULL DEFAULT 5,
    available_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    lease_owner       TEXT,
    lease_expires_at  TIMESTAMPTZ,
    fencing_token     BIGINT      NOT NULL DEFAULT 0,
    last_error        TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at      TIMESTAMPTZ,
    CONSTRAINT background_jobs_status_check
        CHECK (status IN ('pending', 'running', 'succeeded', 'failed')),
    CONSTRAINT background_jobs_attempts_check
        CHECK (attempts >= 0 AND max_attempts >= 1)
);

-- Claim scans: due pending jobs per queue.
CREATE INDEX IF NOT EXISTS idx_background_jobs_due
    ON background_jobs (queue, available_at, id)
    WHERE status = 'pending';

-- Claim scans: running jobs whose lease may have expired.
CREATE INDEX IF NOT EXISTS idx_background_jobs_lease
    ON background_jobs (queue, lease_expires_at)
    WHERE status = 'running';
