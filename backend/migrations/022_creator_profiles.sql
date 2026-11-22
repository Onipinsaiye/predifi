-- migration-phase: expand
-- Issue #278: Add opt-in creator profiles with provenance links.
--
-- Creator profiles are strictly opt-in and private-by-default. A row in this
-- table is only created when the creator explicitly saves a profile via
-- `PUT /api/v1/users/:address/creator-profile`.  An absence of a row means the
-- creator has not opted in and their profile must not be surfaced publicly.
--
-- Provenance links are verified at display time (not at write time) so the
-- backend can store them and validate them on demand without blocking the save.
-- Each link carries a `verified` flag that starts false and is set true by the
-- background verification job.
--
-- Privacy:
--   * `display_name`, `avatar_url`, `bio`, `provenance_links` —
--     private unless `public = TRUE`.
--   * Row is deleted by the user-initiated account-deletion flow
--     (`delete_account_data` in `profile.rs`).
--   * Retained for 365 days after last update when inactive (matches
--     `user_profiles.display_data` in RETENTION_POLICY).

CREATE TABLE IF NOT EXISTS creator_profiles (
    -- Stellar wallet address.  Primary key because a creator has at most one
    -- profile row; the address IS the identity, not a surrogate key.
    address             TEXT        NOT NULL PRIMARY KEY,

    -- Opt-in display name shown beside markets the creator authored.
    -- NULL = not set.  Max 64 characters enforced by the application layer.
    display_name        TEXT        CHECK (char_length(display_name) <= 64),

    -- Optional avatar URL (HTTPS only; enforced by application layer).
    avatar_url          TEXT        CHECK (char_length(avatar_url) <= 512),

    -- Short biography shown on the creator's public profile page.
    -- Max 280 characters (enforced by application layer).
    bio                 TEXT        CHECK (char_length(bio) <= 280),

    -- Provenance links: JSON array of objects with shape:
    --   { "platform": "twitter" | "github" | "farcaster" | "lens" | "url",
    --     "handle_or_url": "...",
    --     "verified": false }
    -- The array is validated and stored as-is; `verified` is updated by the
    -- background verification job.
    provenance_links    JSONB       NOT NULL DEFAULT '[]'::jsonb,

    -- Governs public visibility.  FALSE = profile is private / not surfaced.
    -- Creators must explicitly flip this to make their profile discoverable.
    public              BOOLEAN     NOT NULL DEFAULT FALSE,

    -- Idempotency guard: prevents the creator from registering a conflicting
    -- second row (the PK handles that at the DB level, but this helps the
    -- app layer detect "already opted in" vs "first save").
    opted_in_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Speed up the common query "fetch public creator profile by address".
CREATE INDEX IF NOT EXISTS idx_creator_profiles_public
    ON creator_profiles (address)
    WHERE public = TRUE;

-- Allow background verification jobs to scan for un-verified provenance links.
CREATE INDEX IF NOT EXISTS idx_creator_profiles_updated_at
    ON creator_profiles (updated_at DESC);

-- Trigger to keep `updated_at` current on every row update.
CREATE OR REPLACE FUNCTION set_creator_profiles_updated_at()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_creator_profiles_updated_at ON creator_profiles;
CREATE TRIGGER trg_creator_profiles_updated_at
    BEFORE UPDATE ON creator_profiles
    FOR EACH ROW EXECUTE FUNCTION set_creator_profiles_updated_at();
