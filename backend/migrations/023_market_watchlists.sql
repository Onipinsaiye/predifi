-- migration-phase: expand
-- Issue #280: Add public market watchlists with explicit privacy settings.
--
-- Design decisions:
--   * Watchlists are private by default (public = FALSE).
--   * When a user enables public sharing, we assign a stable `public_slug`
--     (a random UUID rendered as a URL-safe opaque token) so shared links
--     never expose the internal numeric ID or the wallet address.
--   * Revoking public access NULLs `public_slug` immediately; re-enabling
--     generates a fresh slug so old shared links stop working.
--   * The `watchlist_markets` join table holds only pool_id — the public read
--     path queries the pools table for display metadata and never exposes the
--     wallet address of the watchlist owner to public viewers.
--   * Ownership is enforced by `owner_address`; unique constraint on
--     (owner_address, name) prevents duplicate list names per wallet.
--   * Retained as private data under the user-controlled account-deletion flow.

CREATE TABLE IF NOT EXISTS watchlists (
    id              BIGSERIAL   PRIMARY KEY,

    -- Stellar wallet address of the list owner.
    owner_address   TEXT        NOT NULL,

    -- Display name for the watchlist (max 80 characters).
    name            TEXT        NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),

    -- Optional description (max 280 characters).
    description     TEXT        CHECK (char_length(description) <= 280),

    -- Private by default.  TRUE = publicly accessible via public_slug.
    public          BOOLEAN     NOT NULL DEFAULT FALSE,

    -- Opaque random slug used as the shareable URL token.
    -- Generated on demand when public is first set to TRUE.
    -- Revocation sets this to NULL; re-enabling generates a new slug.
    -- Never the owner address or the numeric ID.
    public_slug     TEXT        UNIQUE,

    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Owners cannot have two lists with the same name.
    UNIQUE (owner_address, name)
);

-- Efficient lookup by owner (private dashboard view).
CREATE INDEX IF NOT EXISTS idx_watchlists_owner
    ON watchlists (owner_address);

-- Efficient lookup by public_slug (unauthenticated public share link).
CREATE INDEX IF NOT EXISTS idx_watchlists_public_slug
    ON watchlists (public_slug)
    WHERE public_slug IS NOT NULL;

-- ── Join table: watchlist ↔ pool ──────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS watchlist_markets (
    watchlist_id    BIGINT      NOT NULL REFERENCES watchlists (id) ON DELETE CASCADE,
    -- Pool ID as recorded in the pools table.
    pool_id         BIGINT      NOT NULL,
    -- Position for user-defined ordering within the list.
    position        INT         NOT NULL DEFAULT 0,
    added_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (watchlist_id, pool_id)
);

CREATE INDEX IF NOT EXISTS idx_watchlist_markets_watchlist_id
    ON watchlist_markets (watchlist_id, position);

-- ── Trigger to keep updated_at current ───────────────────────────────────────

CREATE OR REPLACE FUNCTION set_watchlists_updated_at()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_watchlists_updated_at ON watchlists;
CREATE TRIGGER trg_watchlists_updated_at
    BEFORE UPDATE ON watchlists
    FOR EACH ROW EXECUTE FUNCTION set_watchlists_updated_at();
