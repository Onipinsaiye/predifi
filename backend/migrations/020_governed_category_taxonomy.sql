-- migration-phase: expand
-- compatible-app-versions: previous and current release; category_id is additive
-- and existing category text remains the compatibility projection.

CREATE TABLE IF NOT EXISTS market_categories (
    category_id  TEXT PRIMARY KEY,
    display_label TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'deprecated')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    retired_at TIMESTAMPTZ,
    retirement_reason TEXT
);

CREATE TABLE IF NOT EXISTS market_category_aliases (
    alias TEXT PRIMARY KEY,
    category_id TEXT NOT NULL REFERENCES market_categories(category_id),
    created_at TIMESTAMPTZ NOT NOT NULL DEFAULT NOW()
);

INSERT INTO market_categories (category_id, display_label, status)
VALUES
    ('sports', 'Sports', 'active'),
    ('finance', 'Finance', 'active'),
    ('crypto', 'Crypto', 'active'),
    ('politics', 'Politics', 'active'),
    ('entertainment', 'Entertainment', 'active'),
    ('technology', 'Technology', 'active'),
    ('science', 'Science', 'active'),
    ('other', 'Other', 'active')
ON CONFLICT (category_id) DO NOTHING;

INSERT INTO market_category_aliases (alias, category_id) VALUES
    ('sports', 'sports'),
    ('Sports', 'sports'),
    ('entertainment', 'entertainment'),
    ('Entertainment', 'entertainment'),
    ('Entertain', 'entertainment'),
    ('finance', 'finance'),
    ('Finance', 'finance'),
    ('crypto', 'crypto'),
    ('Crypto', 'crypto'),
    ('politics', 'politics'),
    ('Politics', 'politics'),
    ('technology', 'technology'),
    ('Technology', 'technology'),
    ('Tech', 'technology'),
    ('science', 'science'),
    ('Science', 'science'),
    ('other', 'other'),
    ('Other', 'other')
ON CONFLICT (alias) DO NOTHING;

ALTER TABLE pools ADD COLUMN IF NOT EXISTS category_id TEXT;

DO $$
DECLARE
    row_record RECORD;
BEGIN
    FOR row_record IN
        SELECT DISTINCT category
        FROM pools
        WHERE category IS NOT NULL
          AND category <> ''
          AND NOT EXISTS (
              SELECT 1
              FROM market_category_aliases alias
              WHERE lower(alias.alias) = lower(pools.category)
          )
    LOOP
        INSERT INTO market_categories (
            category_id,
            display_label,
            status,
            retired_at,
            retirement_reason
        )
        VALUES (
            'legacy_' || md5(row_record.category),
            row_record.category,
            'deprecated',
            NOW(),
            'Preserved from the historic free-form category migration'
        )
        ON CONFLICT (category_id) DO UPDATE
        SET display_label = EXCLUDED.display_label;

        INSERT INTO market_category_aliases (
            alias,
            category_id
        )
        VALUES (
            row_record.category,
            'legacy_' || md5(row_record.category)
        )
        ON CONFLICT (alias) DO NOTHING;
    END LOOP;
END $$;

UPDATE pools
SET category_id = COALESCE(
    (
        SELECT alias.category_id
        FROM market_category_aliases alias
        WHERE lower(alias.alias) = lower(pools.category)
        LIMIT 1
    ),
    'other'
)
WHERE category_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_pools_state_category_id
    ON pools (state, category_id);
