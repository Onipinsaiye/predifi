-- Migration 027: Opt-in public participation statistics
--
-- Stores thresholded aggregate snapshots that may be published publicly, plus
-- a raw internal table used only for wallet-level aggregation. Raw wallet-level
-- records are never exposed through any public API; only snapshots whose
-- cohort size meets the privacy threshold are eligible for publication.

BEGIN;

-- Raw internal table: wallet-level participation records used to compute
-- aggregates. This table is internal-only and must never be exposed publicly.
CREATE TABLE IF NOT EXISTS participation_raw_wallet_records (
    id              BIGSERIAL PRIMARY KEY,
    wallet_id       TEXT        NOT NULL,
    cohort_key      TEXT        NOT NULL,
    metric_name     TEXT        NOT NULL,
    metric_value    NUMERIC     NOT NULL DEFAULT 0,
    recorded_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Indexes supporting internal aggregation by cohort/metric and time window.
CREATE INDEX IF NOT EXISTS idx_participation_raw_wallet_records_cohort_metric
    ON participation_raw_wallet_records (cohort_key, metric_name, recorded_at);

CREATE INDEX IF NOT EXISTS idx_participation_raw_wallet_records_wallet
    ON participation_raw_wallet_records (wallet_id);

-- Public-facing snapshots: only thresholded aggregates. Each row documents the
-- denominator (cohort size at sampling time) and the sampling timestamp so
-- consumers can interpret the aggregate reproducibly.
CREATE TABLE IF NOT EXISTS participation_statistics_snapshots (
    id                BIGSERIAL PRIMARY KEY,
    cohort_key        TEXT        NOT NULL,
    cohort_label      TEXT        NOT NULL,
    metric_name       TEXT        NOT NULL,
    aggregate_value   NUMERIC     NOT NULL,
    denominator       INTEGER     NOT NULL,
    privacy_threshold INTEGER     NOT NULL DEFAULT 10,
    sampled_at        TIMESTAMPTZ NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_participation_statistics_denominator_nonnegative
        CHECK (denominator >= 0),
    CONSTRAINT chk_participation_statistics_privacy_threshold_positive
        CHECK (privacy_threshold > 0),
    -- Suppress cohorts below the privacy threshold: a snapshot may only be
    -- stored/published when its denominator meets the threshold.
    CONSTRAINT chk_participation_statistics_meets_privacy_threshold
        CHECK (denominator >= privacy_threshold)
);

-- Indexes for public reads by metric and sampling time.
CREATE INDEX IF NOT EXISTS idx_participation_statistics_snapshots_metric_sampled_at
    ON participation_statistics_snapshots (metric_name, sampled_at DESC);

CREATE INDEX IF NOT EXISTS idx_participation_statistics_snapshots_cohort_metric_sampled_at
    ON participation_statistics_snapshots (cohort_key, metric_name, sampled_at DESC);

CREATE INDEX IF NOT EXISTS idx_participation_statistics_snapshots_sampled_at
    ON participation_statistics_snapshots (sampled_at DESC);

COMMIT;