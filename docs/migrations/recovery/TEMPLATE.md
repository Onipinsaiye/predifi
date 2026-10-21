# Recovery plan: `<NNN>_<description>.sql`

Copy this file to `docs/migrations/recovery/<NNN>_<description>.md`, fill in
every section, and link it from the migration with
`-- recovery-plan: docs/migrations/recovery/<NNN>_<description>.md`.
Policy: [`../expand-migrate-contract.md`](../expand-migrate-contract.md).

## Change

What the migration removes or tightens, which tables it touches, and the
approximate row counts in production.

- Expand migration this contracts: `<NNN>_<description>.sql` (shipped in release `vX.Y`)
- Data permanently removed: yes / no — describe

## Compatible application versions

| Release | Uses old shape? | Works after this migration? |
|---|---|---|
| `vX.Y` (current) | no | yes |
| `vX.Y-1` (rollback target) | no | yes |
| `vX.Y-2` | yes | **no — not a supported rollback target after this deploy** |

Evidence that the rollback target no longer uses the old shape (PR links,
query search, *Migration Safety* run against that tag):

## Pre-flight

- [ ] RDS snapshot taken immediately before the deploy: `<snapshot id>`
- [ ] Old shape unused for at least one full release (check `pg_stat_user_tables` / query logs)
- [ ] Lock impact and `lock_timeout` set (see `docs/MIGRATION_DEPLOYMENT_GUIDE.md`)
- [ ] Data that will be dropped exported to `<location>` if it may be needed again

## Recovery

How to restore service if the migration fails halfway or a release that needs
the old shape must come back.

1. Stop: ...
2. Restore: e.g. re-add the column and backfill from the export / snapshot, or
   restore the snapshot to a side instance and copy rows back.
3. Expected time to recover: ...

## Verification

How to confirm the migration and (if used) the recovery worked: queries,
dashboards, row counts.

## Approval

Approved-by: <@handle (YYYY-MM-DD)>
