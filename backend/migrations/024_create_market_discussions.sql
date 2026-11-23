-- Migration 024: Versioned persistence for market discussions (issue #299)
--
-- Adds relational, versioned storage for market discussion comments so that
-- edits and moderation never touch the indexed contract event tables
-- (created in migrations 016/017). This migration is idempotent and
-- reversible for round-trip tests.

BEGIN;

-- ---------------------------------------------------------------------------
-- Constants
-- ---------------------------------------------------------------------------
-- Maximum allowed thread depth (root comment = depth 0).
-- Maximum allowed body size in bytes.
-- These are enforced both by CHECK constraints and by the depth trigger.

-- ---------------------------------------------------------------------------
-- Table: market_discussion_comments
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS market_discussion_comments (
    comment_id        UUID PRIMARY KEY,
    pool_id           TEXT NOT NULL,
    parent_id         UUID NULL,
    wallet_address    TEXT NOT NULL,
    revision          INTEGER NOT NULL DEFAULT 1,
    body              TEXT NOT NULL,
    body_length       INTEGER NOT NULL,
    depth             INTEGER NOT NULL DEFAULT 0,
    moderation_state  TEXT NOT NULL DEFAULT 'visible',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at        TIMESTAMPTZ NULL,

    CONSTRAINT market_discussion_comments_parent_fk
        FOREIGN KEY (parent_id)
        REFERENCES market_discussion_comments (comment_id)
        ON DELETE RESTRICT,

    CONSTRAINT market_discussion_comments_pool_id_not_empty
        CHECK (length(pool_id) > 0),

    CONSTRAINT market_discussion_comments_wallet_not_empty
        CHECK (length(wallet_address) > 0),

    CONSTRAINT market_discussion_comments_revision_positive
        CHECK (revision >= 1),

    -- Bounded body size (bytes) and consistency with body_length.
    CONSTRAINT market_discussion_comments_body_length_bounded
        CHECK (body_length >= 1 AND body_length <= 4096),

    CONSTRAINT market_discussion_comments_body_length_matches
        CHECK (body_length = octet_length(body)),

    -- Bounded thread depth (root = 0, max depth = 5).
    CONSTRAINT market_discussion_comments_depth_bounded
        CHECK (depth >= 0 AND depth <= 5),

    -- A root comment has no parent and depth 0; a reply must have a parent.
    CONSTRAINT market_discussion_comments_root_depth
        CHECK (
            (parent_id IS NULL AND depth = 0)
            OR (parent_id IS NOT NULL AND depth >= 1)
        ),

    CONSTRAINT market_discussion_comments_moderation_state_valid
        CHECK (moderation_state IN ('visible', 'hidden', 'removed', 'pending')),

    CONSTRAINT market_discussion_comments_no_self_parent
        CHECK (parent_id IS NULL OR parent_id <> comment_id),

    CONSTRAINT market_discussion_comments_updated_after_created
        CHECK (updated_at >= created_at)
);

-- Thread traversal index.
CREATE INDEX IF NOT EXISTS market_discussion_comments_thread_idx
    ON market_discussion_comments (pool_id, parent_id, created_at);

-- Wallet lookup index.
CREATE INDEX IF NOT EXISTS market_discussion_comments_wallet_idx
    ON market_discussion_comments (wallet_address, created_at);

-- Moderation lookup index.
CREATE INDEX IF NOT EXISTS market_discussion_comments_moderation_idx
    ON market_discussion_comments (moderation_state, created_at);

-- ---------------------------------------------------------------------------
-- Table: market_discussion_comment_revisions
-- Captures the full edit history of each comment.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS market_discussion_comment_revisions (
    comment_id      UUID NOT NULL,
    revision        INTEGER NOT NULL,
    body            TEXT NOT NULL,
    body_length     INTEGER NOT NULL,
    editor_wallet   TEXT NOT NULL,
    edited_at       TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT market_discussion_comment_revisions_pk
        PRIMARY KEY (comment_id, revision),

    CONSTRAINT market_discussion_comment_revisions_comment_fk
        FOREIGN KEY (comment_id)
        REFERENCES market_discussion_comments (comment_id)
        ON DELETE CASCADE,

    CONSTRAINT market_discussion_comment_revisions_revision_positive
        CHECK (revision >= 1),

    CONSTRAINT market_discussion_comment_revisions_editor_not_empty
        CHECK (length(editor_wallet) > 0),

    CONSTRAINT market_discussion_comment_revisions_body_length_bounded
        CHECK (body_length >= 1 AND body_length <= 4096),

    CONSTRAINT market_discussion_comment_revisions_body_length_matches
        CHECK (body_length = octet_length(body))
);

CREATE INDEX IF NOT EXISTS market_discussion_comment_revisions_comment_idx
    ON market_discussion_comment_revisions (comment_id, revision);

-- ---------------------------------------------------------------------------
-- Trigger: enforce bounded depth and derive depth from parent on insert/update
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION market_discussion_comments_enforce_depth()
RETURNS TRIGGER AS $$
DECLARE
    parent_depth INTEGER;
    max_depth CONSTANT INTEGER := 5;
BEGIN
    IF NEW.parent_id IS NULL THEN
        NEW.depth := 0;
    ELSE
        SELECT depth INTO parent_depth
        FROM market_discussion_comments
        WHERE comment_id = NEW.parent_id;

        IF parent_depth IS NULL THEN
            RAISE EXCEPTION
                'market_discussion_comments: parent % does not exist',
                NEW.parent_id
                USING ERRCODE = 'foreign_key_violation';
        END IF;

        NEW.depth := parent_depth + 1;

        IF NEW.depth > max_depth THEN
            RAISE EXCEPTION
                'market_discussion_comments: maximum thread depth % exceeded',
                max_depth
                USING ERRCODE = 'check_violation';
        END IF;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS market_discussion_comments_depth_trigger
    ON market_discussion_comments;

CREATE TRIGGER market_discussion_comments_depth_trigger
    BEFORE INSERT OR UPDATE OF parent_id
    ON market_discussion_comments
    FOR EACH ROW
    EXECUTE FUNCTION market_discussion_comments_enforce_depth();

-- ---------------------------------------------------------------------------
-- Trigger: keep updated_at fresh and record revisions on body edits
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION market_discussion_comments_touch_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at := now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS market_discussion_comments_touch_trigger
    ON market_discussion_comments;

CREATE TRIGGER market_discussion_comments_touch_trigger
    BEFORE UPDATE
    ON market_discussion_comments
    FOR EACH ROW
    EXECUTE FUNCTION market_discussion_comments_touch_updated_at();

-- ---------------------------------------------------------------------------
-- Trigger: append previous body to revision history on edit
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION market_discussion_comments_record_revision()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.body IS DISTINCT FROM OLD.body THEN
        INSERT INTO market_discussion_comment_revisions (
            comment_id,
            revision,
            body,
            body_length,
            editor_wallet,
            edited_at
        ) VALUES (
            OLD.comment_id,
            OLD.revision,
            OLD.body,
            OLD.body_length,
            OLD.wallet_address,
            OLD.updated_at
        )
        ON CONFLICT (comment_id, revision) DO NOTHING;

        NEW.revision := OLD.revision + 1;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS market_discussion_comments_revision_trigger
    ON market_discussion_comments;

CREATE TRIGGER market_discussion_comments_revision_trigger
    BEFORE UPDATE OF body
    ON market_discussion_comments
    FOR EACH ROW
    EXECUTE FUNCTION market_discussion_comments_record_revision();

COMMIT;
```

-- Down migration (for round-trip tests):
-- BEGIN;
-- DROP TRIGGER IF EXISTS market_discussion_comments_revision_trigger ON market_discussion_comments;
-- DROP FUNCTION IF EXISTS market_discussion_comments_record_revision();
-- DROP TRIGGER IF EXISTS market_discussion_comments_touch_trigger ON market_discussion_comments;
-- DROP FUNCTION IF EXISTS market_discussion_comments_touch_updated_at();
-- DROP TRIGGER IF EXISTS market_discussion_comments_depth_trigger ON market_discussion_comments;
-- DROP FUNCTION IF EXISTS market_discussion_comments_enforce_depth();
-- DROP TABLE IF EXISTS market_discussion_comment_revisions;
-- DROP TABLE IF EXISTS market_discussion_comments;
-- COMMIT;