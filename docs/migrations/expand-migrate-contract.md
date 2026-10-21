# Expand–Migrate–Contract: Rollback-Safe Schema Changes

## Why

Migrations run forward only (see [`backend/migrations/README.md`](../../backend/migrations/README.md)).
If release *N* drops or renames something release *N−1* still uses, rolling
the application back to *N−1* fails — the schema it expects is gone. The fix is
not to roll back schemas; it is to make sure **every schema we deploy works with
both the release that introduced it and the release before it**.

We do that by splitting every incompatible change into three steps that ship
in **separate releases**.

## The three phases

| Phase | What it may do | What it may not do | Ships in |
|---|---|---|---|
| **expand** | Add tables, nullable columns, columns with a `DEFAULT`, indexes (`CONCURRENTLY`), views, triggers that dual-write | Drop, rename, change types, add `NOT NULL` without `DEFAULT`, delete data | Release *N* (with or before the code that uses it) |
| **migrate** | Backfill or copy data into the expanded shape, in batches | Anything an `expand` may not do | Release *N* or *N+1*, after the expand is live |
| **contract** | Drop the old column/table, add `NOT NULL`, remove dual-write triggers | Run before every supported release has stopped using the old shape | Release *N+2* at the earliest |

### Compatibility at every stage

Example: renaming `pools.name` to `pools.title`.

| Stage | Schema | Release *N−1* (reads/writes `name`) | Release *N* (writes both, reads `title`) | Release *N+1* (uses `title` only) |
|---|---|---|---|---|
| 1. **Expand** in *N*: add nullable `title`, trigger copies `name`→`title` | `name` + `title` | ✅ works — ignores `title` | ✅ works | — |
| 2. **Migrate** in *N*: backfill `title` from `name` | `name` + `title` (filled) | ✅ works | ✅ works | ✅ works |
| 3. Release *N+1* ships: stops using `name` | `name` + `title` | — no longer a rollback target | ✅ works — rollback target for *N+1* | ✅ works |
| 4. **Contract** in *N+2*: drop `name`, drop trigger, `title SET NOT NULL` | `title` only | ❌ breaks — no longer supported | ✅ works (reads `title`) | ✅ works |

The rule this table encodes: **after any migration, the current release and
the one before it must both work.** A contract migration is allowed only once
the release before it no longer touches the old shape.

Each migration states this in a header that CI checks:

```sql
-- migration-phase: expand
-- compatible-app-versions: previous (v1.4) and current (v1.5); old code ignores pools.title
```

## What CI enforces

`.github/workflows/migration-safety.yml` runs on every PR that touches
migrations or backend SQL.

**1. Sequencing gate** (`.github/scripts/check_migration_safety.py`), for
every migration numbered above `013`:

- A `-- migration-phase:` and `-- compatible-app-versions:` header is required.
- These statements fail the build unless the migration is a `contract`:
  `DROP TABLE/COLUMN/SCHEMA/VIEW/FUNCTION/TYPE`, `TRUNCATE`, `DELETE FROM`,
  `RENAME`, column `TYPE` changes, `SET NOT NULL`, `DROP DEFAULT`, and
  `ADD COLUMN … NOT NULL` without a `DEFAULT`.
- A `contract` migration must name the expand it finishes
  (`-- contracts: 014_add_pool_title.sql`). That file must **already be on
  `main`**, so the expand and the contract cannot ship in the same release.
- A `contract` migration must link a recovery plan
  (`-- recovery-plan: docs/migrations/recovery/016_drop_pool_name.md`) with
  all required sections and an `Approved-by:` line.
- Merged migration files cannot be edited, renamed, or deleted, and new files
  must use a fresh, higher numeric prefix (no more duplicate `004_`/`009_`).

**2. Rollback compatibility test** (`.github/scripts/migration_rollback_compat.py`):

- Applies all migrations from the PR to an empty Postgres.
- Collects every static SQL statement from the **previous release** (the PR's
  base commit) and from the PR itself — the sqlx offline cache plus string
  literals passed to `sqlx::query*` — and `PREPARE`s each one against the new
  schema. `PREPARE` plans the query without running it, so a missing column,
  table, or incompatible type fails CI instead of failing a production rollback.
- Run it against an older release manually with *Actions → Migration Safety →
  Run workflow* and a tag in `previous_ref`.

Limits: dynamic SQL built at runtime (`format!`, `QueryBuilder`) is not
covered. Review those changes by hand against the compatibility table.

## Approving a destructive migration

1. Copy [`recovery/TEMPLATE.md`](recovery/TEMPLATE.md) to
   `docs/migrations/recovery/<migration file name>.md` and fill it in.
2. Link it from the migration header.
3. A reviewer from the owning team (backend for application tables, infra for
   anything touching roles or extensions) reads the plan, confirms the
   prerequisites (expand shipped, previous release no longer uses the old
   shape, backup taken), and adds `Approved-by: @handle (YYYY-MM-DD)` in a
   review commit.
4. CODEOWNERS requests that review automatically for files in
   `docs/migrations/recovery/`.

## Release checklist

- [ ] Every new migration has a phase and compatibility header.
- [ ] No contract migration ships in the same release as its expand.
- [ ] The rollback target (previous release) is listed in the PR, and
      *Migration Safety* is green against it.
- [ ] Contract migrations have an approved recovery plan, and a fresh RDS
      snapshot is taken immediately before the deploy.
- [ ] Locking impact is classified in
      [`docs/MIGRATION_DEPLOYMENT_GUIDE.md`](../MIGRATION_DEPLOYMENT_GUIDE.md)
      (use `CREATE INDEX CONCURRENTLY`, `NOT VALID` constraints, batched
      backfills).

## Legacy migrations

Migrations `001`–`013` predate this policy. Their rollback and forward-fix
paths are in [`docs/MIGRATION_DEPLOYMENT_GUIDE.md`](../MIGRATION_DEPLOYMENT_GUIDE.md).
They are not re-checked for phase headers, but they are covered by the
rollback compatibility test and remain immutable.
