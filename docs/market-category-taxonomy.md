# Governed Market Category Taxonomy

Status: Current

Market categories use stable IDs for storage and analytics. Display labels and
aliases may change without changing a pool's original category ID.

## Canonical categories

| Stable ID | Display label | Contract symbol / legacy aliases |
|---|---|---|
| `sports` | Sports | `Sports` |
| `finance` | Finance | `Finance` |
| `crypto` | Crypto | `Crypto` |
| `politics` | Politics | `Politics` |
| `entertainment` | Entertainment | `Entertainment`, `Entertain` |
| `technology` | Technology | `Technology`, `Tech` |
| `science` | Science | `Science` |
| `other` | Other | `Other` |

The stable ID is the durable key. The pool `category` column remains a
compatibility display projection. A pool stores both values and never rewrites
`category_id` when a label or alias is deprecated.

## Governance

- **Add category:** requires approval by the protocol governance owner (the
  multisig-admin role), with a unique ID, display label, and migration/alias
  plan recorded in the category registry.
- **Rename or relabel:** may be performed only by the multisig-admin role and
  must not change the existing ID or pool assignments.
- **Deprecate/retire:** requires multisig-admin approval and a retirement
  reason. Retirement removes the category from active creation choices but
  keeps it queryable for historic markets.
- **Alias:** may be added only to one category, never to multiple categories.
  Aliases are normalized case-insensitively for lookup.
- **Remove alias:** requires the same multisig-admin approval as retirement,
  and must not alter any historic pool ID.

Runtime admin endpoints are not authorization substitutes: governance approval
is recorded by the contract/admin process, and database registry changes must
be deployed with review and migration verification.

## Migration behavior

Migration `020_governed_category_taxonomy.sql` is an expand migration:

1. Creates the category and alias registry.
2. Adds nullable `pools.category_id` for application compatibility.
3. Inserts canonical categories and aliases.
4. Creates a deprecated `legacy_<md5(category)>` category for each previously
   unknown historic label.
5. Backfills each pool exactly once. The original display label remains in
   `pools.category`.

No application migration rewrites a historic pool's assigned ID. If an old
market used "Weather", its ID remains the generated legacy ID even if "Weather"
later becomes an active or deprecated category.

## Unknown-category handling

- New pool ingestion rejects unknown categories with HTTP 400
  `unknown market category`.
- Pool category filters reject unknown IDs and aliases with HTTP 400.
- A deprecated category or alias remains a valid filter for historic markets.
- Historic unrecognized labels are preserved under a stable deprecated legacy
  category and remain filterable.

## Filtering

`GET /api/v1/pools?category=<id-or-alias>` resolves the input through the
registry, then filters `pools.category_id`. `GET /api/v1/categories` exposes
stable IDs, display labels, status, and aliases. `GET /api/v1/stats` applies
the same category resolution.

## Validation

Seeded fixture suites must verify:

- migration backfill for canonical and legacy historic categories;
- unknown category creation and filter rejection;
- ID/alias canonicalization;
- filters return only markets with the resolved `category_id`;
- a retired category retains the same ID and historic rows.
