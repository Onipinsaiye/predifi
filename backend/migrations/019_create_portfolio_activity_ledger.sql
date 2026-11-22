-- migration-phase: expand
-- compatible-app-versions: previous and current release; adds a new table only.
--
-- Normalized wallet activity ledger (#309).
--
-- Every wallet-attributable chain event projected through
-- `project_event_tx` (pool_created, prediction_placed, pool_resolved) is
-- denormalized into this table at projection time, inside the same
-- transaction as the `projected_contract_events` dedupe claim and the
-- domain write — a row here is never visible before its event is fully,
-- atomically committed. `claim` rows are added by the claim ingest path
-- only when its payload supplies on-chain coordinates; `predictions.
-- claimed_amount` remains the payout ground truth regardless.
--
-- Rows share the same (network, contract_id, ledger, tx_hash, event_index)
-- coordinates as `EventKey`, plus `user_address`, because one event (e.g.
-- pool_resolved) can be relevant to many wallets at once.

CREATE TABLE IF NOT EXISTS portfolio_activity_ledger (
    id            BIGSERIAL   PRIMARY KEY,
    network       TEXT        NOT NULL,
    contract_id   TEXT        NOT NULL,
    ledger        BIGINT      NOT NULL CHECK (ledger >= 0),
    tx_hash       TEXT        NOT NULL,
    event_index   INTEGER     NOT NULL CHECK (event_index >= 0),
    event_type    TEXT        NOT NULL,
    user_address  TEXT        NOT NULL,
    pool_id       BIGINT,
    counterparty  TEXT,
    outcome       INTEGER,
    amount        BIGINT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (network, contract_id, ledger, tx_hash, event_index, user_address)
);

-- Wallet-activity pagination: `WHERE user_address = $1 ORDER BY ledger,
-- event_index, id`, matching `get_wallet_activity`'s cursor query exactly.
CREATE INDEX IF NOT EXISTS idx_portfolio_activity_ledger_wallet_cursor
    ON portfolio_activity_ledger (user_address, ledger, event_index, id);

-- Checkpoint-gated snapshot/export reads: `WHERE user_address = $1 AND
-- ledger <= $2`.
CREATE INDEX IF NOT EXISTS idx_portfolio_activity_ledger_wallet_ledger
    ON portfolio_activity_ledger (user_address, ledger);
