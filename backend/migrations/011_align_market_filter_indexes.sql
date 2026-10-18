-- Migration 011: Align market filters with database indexes
--
-- Adds composite and partial indexes on the `pools` table to support predictable,
-- bounded query plans for common filter combinations on GET /api/v1/pools:
--
-- 1. idx_pools_state_end_time: Supports `state` + `ORDER BY end_time ASC` (ending_soon)
--    and range filters `end_time >= $min AND end_time <= $max`.
-- 2. idx_pools_state_created_at: Supports `state` + `ORDER BY created_at DESC` (new).
-- 3. idx_pools_state_total_stake: Supports `state` + `ORDER BY total_stake DESC` (popular).
-- 4. idx_pools_creator_state: Supports filtering by creator wallet and lifecycle state.
-- 5. idx_pools_settled_result: Partial index for settled markets filtered by outcome result.

CREATE INDEX IF NOT EXISTS idx_pools_state_end_time
    ON pools (state, end_time ASC);

CREATE INDEX IF NOT EXISTS idx_pools_state_created_at
    ON pools (state, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_pools_state_total_stake
    ON pools (state, total_stake DESC);

CREATE INDEX IF NOT EXISTS idx_pools_creator_state
    ON pools (creator, state);

CREATE INDEX IF NOT EXISTS idx_pools_settled_result
    ON pools (result)
    WHERE state = 'settled' AND result IS NOT NULL;
