-- Durable JWT revocation state shared by every backend replica (#55).
--
-- Postgres is the source of truth; Redis only caches a per-user snapshot
-- (see backend/src/token_revocation.rs).
--
-- user_token_versions
--   One row per user that has ever revoked "all sessions". A token whose
--   `tv` claim is lower than `token_version` is revoked. Rows never expire:
--   the version only moves forward, so an old token can never become valid
--   again.
--
-- revoked_tokens
--   Single-token revocations (logout, refresh rotation). `expires_at` is the
--   token's own `exp` plus the verifier's clock-skew leeway, so a record is
--   only purged once the token it describes would fail expiry checks anyway.
--   Purging an expired record therefore cannot revive a token.

CREATE TABLE IF NOT EXISTS user_token_versions (
    user_address   TEXT        PRIMARY KEY,
    token_version  BIGINT      NOT NULL DEFAULT 0 CHECK (token_version >= 0),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS revoked_tokens (
    jti           TEXT        PRIMARY KEY,
    user_address  TEXT        NOT NULL,
    expires_at    TIMESTAMPTZ NOT NULL,
    revoked_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    reason        TEXT        NOT NULL
);

-- Per-user lookup of still-relevant revocations (hot-path cache fill).
CREATE INDEX IF NOT EXISTS idx_revoked_tokens_user_expires
    ON revoked_tokens (user_address, expires_at);

-- Periodic purge of records whose tokens have expired.
CREATE INDEX IF NOT EXISTS idx_revoked_tokens_expires
    ON revoked_tokens (expires_at);
