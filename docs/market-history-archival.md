# Market History Archival, Restore and Backfill

Covers the hot/cold retention tier for the historical market-value time
series introduced in #313/#314:

* `backend/src/db/market_history.rs` — the bounded query API (#313) and the
  hot-tier table, `market_history_samples`
  (`backend/migrations/022_create_market_history_samples.sql`).
* `backend/src/db/market_history_archive.rs` — the cold tier, checksummed
  archive format, and archive/restore functions (#314), backed by
  `market_history_archives`
  (`backend/migrations/023_create_market_history_archives.sql`).

## Why this exists

Historical market-value samples accumulate forever (one row per pool, per
bucket width, per bucket interval) while being queried far less often than
they are written. Left in a single table, this degrades planner behavior
and bloats the hot path that live reads (`GET /v1/markets/{pool_id}/history`)
depend on. Moving old rows out does not change what callers can ask for or
how the response is shaped — see "Query contract is tier-agnostic" below.

## Archive format

`archive_market_history(pool, pool_id, width, cutoff)`:

1. Selects every `market_history_samples` row for `(pool_id, bucket_width)`
   with `bucket_start < cutoff`.
2. Serializes those rows to a canonical JSON array (sorted by
   `bucket_start`, so the same logical row set always produces
   byte-identical payload text) — `{bucket_start, value, checkpoint_ledger}`
   per row.
3. Computes the SHA-256 hex digest of that JSON text.
4. Inserts one row into `market_history_archives` holding the payload, its
   checksum, the row count, and the covered `[bucket_start_min,
   bucket_start_max]` range (used to find candidate archives for a query
   without deserializing every payload).
5. Deletes the archived rows from `market_history_samples`.

Steps 1, 4 and 5 run inside one transaction, so a crash mid-archive can
never leave a row duplicated across tiers or dropped entirely.

This is intentionally a simple checksummed blob format reusing Postgres
storage — the same approach `portfolio_exports` already uses for its CSV
artifacts — not a new storage engine or object-storage dependency.

## Query contract is tier-agnostic

`get_market_history(pool, pool_id, width, from, to)` (the function behind
`GET /v1/markets/{pool_id}/history`) always:

1. Reads any matching rows from the hot tier.
2. For any requested bucket still missing, looks up candidate archives
   (`bucket_start_min < to AND bucket_start_max >= from`) and verifies each
   one's checksum before trusting its payload. A corrupted archive is
   logged and skipped — the response degrades to "these particular buckets
   are unavailable," never a hard 500 for the whole page.
3. For any bucket still missing after both tiers (never archived, never
   computed), aggregates fresh from `portfolio_activity_ledger` and caches
   the result back into the hot tier.

The returned `MarketValueSample` shape, decimal-string value encoding,
checkpoint cursor, and bucket ordering are identical no matter which tier
(or combination of tiers) actually served a given sample. Callers cannot
tell, and must not need to.

## Backfill procedure

Backfilling simply means populating the hot tier for a range that has never
been queried (and therefore never materialized) yet — there is no separate
backfill job to run. Calling `get_market_history` for the desired
`(pool_id, width, from, to)` computes and caches every bucket in that range
on demand. To proactively warm the cache for a pool (e.g. right after it is
created, so the first real client request isn't the one paying the
aggregation cost), call `get_market_history` once for the full desired
range from an operator shell, a one-off task, or a startup hook — the
result is identical to, and interchangeable with, an organically populated
cache.

## Restore procedure

If a pool's history needs to be queryable from the hot tier again (for
example, before a bulk re-export, or while debugging a discrepancy), restore
its archive(s):

1. Identify the archive id(s) covering the pool and bucket width in
   question:

   ```sql
   SELECT id, bucket_width, bucket_start_min, bucket_start_max, row_count, archived_at
   FROM market_history_archives
   WHERE pool_id = $1
   ORDER BY bucket_start_min;
   ```

2. For each id, call `restore_market_history_archive(pool, archive_id)`.
   This:
   * Recomputes the archive's SHA-256 checksum and compares it to the
     stored value. A mismatch returns `ArchiveError::ChecksumMismatch` and
     restores nothing — a corrupted archive is never silently restored.
   * Deserializes the verified payload and upserts every row back into
     `market_history_samples` (`ON CONFLICT` on `(pool_id, bucket_width,
     bucket_start)`, so re-running a restore is idempotent).
   * Deletes the archive row once its rows are back in the hot tier.
3. Re-run the query (`get_market_history`, or `GET
   /v1/markets/{pool_id}/history`) for the restored range and confirm the
   values match what was recorded before archival (see the
   `query_results_are_identical_before_archival_and_after_tier_migration`
   and `archive_then_restore_round_trips_exactly` tests in
   `backend/src/db/market_history.rs` for the exact invariant this
   preserves).

If step 2 reports a checksum mismatch, do not delete the archive row — treat
it as a data-integrity incident: capture the archive id and the mismatched
payload for investigation, and fall back to recomputing the affected range
from `portfolio_activity_ledger` via `get_market_history` (step 3 of the
query contract above repopulates the hot tier from the ledger directly,
independent of the corrupted archive, since the ledger — not the archive —
is the ultimate source of truth).

## Operational notes

* There is no scheduled archival job wired up yet; `archive_market_history`
  is a library function an operator or a future scheduled task calls with
  an explicit `(pool_id, width, cutoff)`. Running it is always safe:
  `ArchiveError::NothingToArchive` is returned (and nothing is modified)
  when no hot rows match.
* Archival and restore are per-`(pool_id, bucket_width)`; there is no
  bulk/all-pools entry point. A batch job wrapping
  `archive_market_history` in a loop over pools is the expected way to
  apply a retention window fleet-wide once one is defined.
* Because every hot-tier row is reproducible from
  `portfolio_activity_ledger`, losing a chunk of hot *and* cold data for a
  given range is recoverable by simply re-querying that range — it is a
  materialized cache of chain-derived history, never the only copy.
