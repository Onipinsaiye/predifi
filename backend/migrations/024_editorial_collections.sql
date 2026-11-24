-- 024_editorial_collections.sql
-- Editorial collections: versioned, curated lists of canonical market IDs.
-- Ordering is explicitly editorial and must not affect protocol state or rankings.

BEGIN;

-- Collections: stable identity for a curated collection.
CREATE TABLE IF NOT EXISTS editorial_collections (
    id              BIGSERIAL PRIMARY KEY,
    slug            TEXT NOT NULL UNIQUE,
    title           TEXT NOT NULL,
    description     TEXT,
    editor_address  TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Versions: immutable snapshots of a collection's contents.
-- is_current marks the single active version per collection.
CREATE TABLE IF NOT EXISTS editorial_collection_versions (
    id                      BIGSERIAL PRIMARY KEY,
    collection_id           BIGINT NOT NULL REFERENCES editorial_collections(id) ON DELETE CASCADE,
    version_number          INTEGER NOT NULL,
    editor_address          TEXT NOT NULL,
    change_note             TEXT,
    -- Explicitly label ordering as editorial so consumers never treat it as
    -- a ranking or protocol-derived signal.
    editorial_ordering_label TEXT NOT NULL DEFAULT 'editorial',
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    is_current              BOOLEAN NOT NULL DEFAULT FALSE,
    CONSTRAINT editorial_collection_versions_unique_version
        UNIQUE (collection_id, version_number),
    CONSTRAINT editorial_collection_versions_version_positive
        CHECK (version_number > 0)
);

-- At most one current version per collection.
CREATE UNIQUE INDEX IF NOT EXISTS editorial_collection_versions_one_current
    ON editorial_collection_versions (collection_id)
    WHERE is_current;

-- Items: canonical market IDs (pool IDs) in editorial order.
-- No FK to markets: missing/unavailable markets are handled at query time
-- so items are never silently dropped.
CREATE TABLE IF NOT EXISTS editorial_collection_items (
    version_id  BIGINT NOT NULL REFERENCES editorial_collection_versions(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    market_id   TEXT NOT NULL,
    added_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT editorial_collection_items_unique_position
        UNIQUE (version_id, position),
    CONSTRAINT editorial_collection_items_unique_market
        UNIQUE (version_id, market_id),
    CONSTRAINT editorial_collection_items_position_nonnegative
        CHECK (position >= 0)
);

-- Public lookup by slug.
CREATE INDEX IF NOT EXISTS editorial_collections_slug_idx
    ON editorial_collections (slug);

-- Lookup of the current version for a collection.
CREATE INDEX IF NOT EXISTS editorial_collection_versions_current_idx
    ON editorial_collection_versions (collection_id, is_current)
    WHERE is_current;

-- Ordered item retrieval for a version.
CREATE INDEX IF NOT EXISTS editorial_collection_items_version_position_idx
    ON editorial_collection_items (version_id, position);

-- Reverse lookup: which collections reference a given market.
CREATE INDEX IF NOT EXISTS editorial_collection_items_market_idx
    ON editorial_collection_items (market_id);

COMMIT;