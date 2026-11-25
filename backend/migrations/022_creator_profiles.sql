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

    -- Monotonic revision counter.  Incremented on every accepted update so
    -- clients can detect stale writes and ordering can be audited.
    revision            BIGINT      NOT NULL DEFAULT 1,

    -- Explicit visibility state.  `public` is retained for backwards
    -- compatibility; `visibility` is the authoritative state going forward.
    --   'private'  — stored but never surfaced publicly (default)
    --   'unlisted' — reachable by direct link only
    --   'public'   — discoverable / listed
    visibility          TEXT        NOT NULL DEFAULT 'private'
        CHECK (visibility IN ('private', 'unlisted', 'public')),

    -- Wallet rotation support: the address this profile was rotated from
    -- (NULL = never rotated) and when the rotation was accepted.
    rotated_from        TEXT,
    wallet_rotated_at   TIMESTAMPTZ,

    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Keep `visibility` and the legacy `public` flag consistent for rows written
-- by older code paths that only set `public`.
CREATE OR REPLACE FUNCTION sync_creator_profiles_visibility()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.visibility = 'public' THEN
        NEW.public = TRUE;
    ELSIF NEW.public = TRUE AND NEW.visibility = 'private' THEN
        NEW.visibility = 'public';
    ELSE
        NEW.public = FALSE;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_creator_profiles_visibility ON creator_profiles;
CREATE TRIGGER trg_creator_profiles_visibility
    BEFORE INSERT OR UPDATE ON creator_profiles
    FOR EACH ROW EXECUTE FUNCTION sync_creator_profiles_visibility();

-- Proof material is stored separately from the normalized public fields.
-- Each row records a fresh wallet signature authorizing a create/update.
-- `nonce` is unique per address so a replayed proof cannot be consumed twice.
CREATE TABLE IF NOT EXISTS creator_profile_proofs (
    id                  BIGSERIAL   PRIMARY KEY,

    -- Wallet address that produced the signature.
    address             TEXT        NOT NULL,

    -- Base64-encoded wallet signature over the canonical signed payload.
    signature           TEXT        NOT NULL,

    -- Hex digest (e.g. sha256) of the exact canonical payload that was signed.
    payload_digest      TEXT        NOT NULL,

    -- Single-use nonce embedded in the signed payload.  Unique per address.
    nonce               TEXT        NOT NULL,

    -- When the signature was produced (client-supplied, validated for freshness).
    proof_timestamp     TIMESTAMPTZ NOT NULL,

    -- Set TRUE once the proof has been accepted; prevents replay.
    consumed            BOOLEAN     NOT NULL DEFAULT FALSE,
    consumed_at         TIMESTAMPTZ,

    -- Revision this proof authorized (NULL for the initial create).
    revision            BIGINT,

    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Reject replays: a given (address, nonce) may only ever appear once.
    CONSTRAINT uq_creator_profile_proofs_address_nonce UNIQUE (address, nonce)
);

-- Fast lookup of the latest proof for an address.
CREATE INDEX IF NOT EXISTS idx_creator_profile_proofs_address_created
    ON creator_profile_proofs (address, created_at DESC);

-- Background jobs scan for unconsumed proofs to expire / audit.
CREATE INDEX IF NOT EXISTS idx_creator_profile_proofs_unconsumed
    ON creator_profile_proofs (address)
    WHERE consumed = FALSE;

-- Revision ordering lookups (e.g. "give me the profile at revision N").
CREATE INDEX IF NOT EXISTS idx_creator_profiles_revision
    ON creator_profiles (address, revision DESC);

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

-- ---------------------------------------------------------------------------
-- Reversible: see 022_creator_profiles.down.sql for the rollback counterpart.
-- ---------------------------------------------------------------------------
