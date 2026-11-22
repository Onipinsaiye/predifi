-- 022_portfolio_exposure_and_claims.sql
--
-- Expand-only migration supporting:
--   #261 Portfolio exposure by market and outcome
--   #262 Claimable-rewards work queue
--   #263 Claim-to-position reconciliation
--
-- All changes are additive and backward compatible. No columns are dropped,
-- renamed, or made NOT NULL on existing tables.

BEGIN;

-- ---------------------------------------------------------------------------
-- (a) Claim reconciliation ledger
-- ---------------------------------------------------------------------------
-- One row per originating claim event. The composite primary key makes
-- ledger credits idempotent: replaying the same event (retries, reordering,
-- or re-indexing) cannot create a duplicate payout credit.
CREATE TABLE IF NOT EXISTS portfolio_claim_reconciliation (
    network             TEXT        NOT NULL,
    contract_id         TEXT        NOT NULL,
    ledger              BIGINT      NOT NULL,
    tx_hash             TEXT        NOT NULL,
    event_index         INTEGER     NOT NULL,
    user_address        TEXT        NOT NULL,

    -- Canonical market / pool identity and outcome the claim belongs to.
    market_id           TEXT,
    pool_id             TEXT,
    outcome             TEXT,

    -- Payout details.
    claim_tx_hash       TEXT,
    payout_amount       NUMERIC(38, 18),
    payout_asset        TEXT,

    -- Source event identity (canonical event key for cross-referencing).
    source_event_id     TEXT,

    -- Reconciliation lifecycle: pending -> reconciled | delayed.
    reconciliation_status TEXT NOT NULL DEFAULT 'pending'
        CHECK (reconciliation_status IN ('pending', 'reconciled', 'delayed')),

    -- Optional linkage to the originating position, when known.
    position_id         TEXT,

    -- Diagnostics for delayed reconciliation.
    reconciliation_error TEXT,
    attempts            INTEGER     NOT NULL DEFAULT 0,

    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    reconciled_at       TIMESTAMPTZ,

    CONSTRAINT portfolio_claim_reconciliation_pkey
        PRIMARY KEY (network, contract_id, ledger, tx_hash, event_index, user_address)
);

-- Idempotency guard for ledger credits keyed on the source event identity.
CREATE UNIQUE INDEX IF NOT EXISTS ux_portfolio_claim_reconciliation_source_event
    ON portfolio_claim_reconciliation (network, contract_id, source_event_id)
    WHERE source_event_id IS NOT NULL;

-- Lookup by wallet for portfolio views.
CREATE INDEX IF NOT EXISTS ix_portfolio_claim_reconciliation_user
    ON portfolio_claim_reconciliation (network, user_address, reconciliation_status);

-- Lookup by claim transaction for reconciliation from wallet activity.
CREATE INDEX IF NOT EXISTS ix_portfolio_claim_reconciliation_claim_tx
    ON portfolio_claim_reconciliation (network, claim_tx_hash)
    WHERE claim_tx_hash IS NOT NULL;

-- Lookup by canonical market/outcome for exposure aggregation.
CREATE INDEX IF NOT EXISTS ix_portfolio_claim_reconciliation_market_outcome
    ON portfolio_claim_reconciliation (network, market_id, outcome)
    WHERE market_id IS NOT NULL;

-- Work queue for pending/delayed reconciliation.
CREATE INDEX IF NOT EXISTS ix_portfolio_claim_reconciliation_status
    ON portfolio_claim_reconciliation (reconciliation_status, updated_at);

-- ---------------------------------------------------------------------------
-- (b) Supporting indexes for exposure aggregation by (pool_id, outcome)
-- ---------------------------------------------------------------------------
-- Group open positions per wallet by canonical market and outcome.
CREATE INDEX IF NOT EXISTS ix_positions_wallet_pool_outcome
    ON positions (user_address, pool_id, outcome);

-- Filter to open positions only (partial index keeps it small).
CREATE INDEX IF NOT EXISTS ix_positions_open_wallet_pool_outcome
    ON positions (user_address, pool_id, outcome)
    WHERE closed_at IS NULL;

-- Predictions grouped by pool/outcome for exposure estimates.
CREATE INDEX IF NOT EXISTS ix_predictions_pool_outcome
    ON predictions (pool_id, outcome);

CREATE INDEX IF NOT EXISTS ix_predictions_wallet_pool_outcome
    ON predictions (user_address, pool_id, outcome);

-- ---------------------------------------------------------------------------
-- (c) Claimable-rewards queue status on the existing claims table
-- ---------------------------------------------------------------------------
-- Additive columns only; defaults keep existing rows valid.
ALTER TABLE claims
    ADD COLUMN IF NOT EXISTS claim_status TEXT NOT NULL DEFAULT 'eligible';

ALTER TABLE claims
    ADD COLUMN IF NOT EXISTS simulation_error TEXT;

ALTER TABLE claims
    ADD COLUMN IF NOT EXISTS estimated_amount NUMERIC(38, 18);

ALTER TABLE claims
    ADD COLUMN IF NOT EXISTS claim_tx_hash TEXT;

ALTER TABLE claims
    ADD COLUMN IF NOT EXISTS last_simulated_at TIMESTAMPTZ;

ALTER TABLE claims
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now();

-- Constrain status values without breaking existing data.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'claims_claim_status_check'
    ) THEN
        ALTER TABLE claims
            ADD CONSTRAINT claims_claim_status_check
            CHECK (claim_status IN ('eligible', 'ineligible', 'simulating', 'simulation_failed', 'claimed'));
    END IF;
END
$$;

-- Queue lookup: eligible claims per wallet, ordered for bounded selection.
CREATE INDEX IF NOT EXISTS ix_claims_wallet_status
    ON claims (user_address, claim_status);

CREATE INDEX IF NOT EXISTS ix_claims_status_updated
    ON claims (claim_status, updated_at);

-- Link a claim row to its reconciliation record.
CREATE INDEX IF NOT EXISTS ix_claims_claim_tx
    ON claims (claim_tx_hash)
    WHERE claim_tx_hash IS NOT NULL;

COMMIT;