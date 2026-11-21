-- migration-phase: expand
-- compatible-app-versions: previous and current release; adds an independent
-- proposal table without changing existing pool or event semantics.

CREATE TABLE IF NOT EXISTS market_proposals (
    proposal_id UUID PRIMARY KEY,
    question TEXT NOT NULL,
    event_window_start TIMESTAMPTZ NOT NULL,
    event_window_end TIMESTAMPTZ NOT NULL,
    source TEXT NOT NULL,
    outcomes JSONB NOT NULL,
    distinction_note TEXT,
    status TEXT NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'submitted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (event_window_end >= event_window_start)
);

CREATE INDEX IF NOT EXISTS idx_market_proposals_updated_at
    ON market_proposals (updated_at DESC);
