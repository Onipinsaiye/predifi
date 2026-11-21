-- migration-phase: expand
-- compatible-app-versions: previous and current release; adds a new table only.
--
-- Portfolio snapshots generated from committed indexer checkpoints (#311).
--
-- A snapshot is always computed against a specific, already-committed
-- listener checkpoint ledger (its source cursor) and is keyed by
-- (user_address, checkpoint_ledger), so recomputing for the same checkpoint
-- overwrites the row with — deterministically — the same values rather
-- than creating a duplicate or drifting history.

CREATE TABLE IF NOT EXISTS portfolio_snapshots (
    id                     BIGSERIAL   PRIMARY KEY,
    user_address           TEXT        NOT NULL,
    -- Source cursor: the committed checkpoint ledger this snapshot was
    -- computed against.
    checkpoint_ledger      BIGINT      NOT NULL CHECK (checkpoint_ledger >= 0),
    -- Algorithm/model version (see `crate::db::portfolio_pnl::PNL_MODEL_VERSION`).
    pnl_model_version      INTEGER     NOT NULL,
    total_staked           BIGINT      NOT NULL,
    total_realized_return  BIGINT      NOT NULL,
    net_realized_pnl       BIGINT      NOT NULL,
    open_positions         BIGINT      NOT NULL,
    computed_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_address, checkpoint_ledger)
);

CREATE INDEX IF NOT EXISTS idx_portfolio_snapshots_wallet_latest
    ON portfolio_snapshots (user_address, checkpoint_ledger DESC);
