/**
 * Indexer freshness — classifies how far the indexer-mirrored data shown in
 * the UI may be behind the chain, per `docs/data-ownership-matrix.md`
 * (indexed fields are "eventual": bounded staleness, must be flagged rather
 * than presented as current).
 */

export type FreshnessState = "fresh" | "stale" | "unavailable";

/** How far behind the indexer can be before data is flagged stale. */
export const DEFAULT_STALE_THRESHOLD_MS = 60_000;

export interface FreshnessInfo {
  state: FreshnessState;
  /** Milliseconds between `now` and the indexed timestamp, when known. */
  ageMs: number | null;
}

/**
 * Classify freshness from an indexer timestamp.
 *
 * `indexedAt` is `null`/`undefined` when the backend hasn't reported a
 * timestamp at all (e.g. the indexer is unreachable or the field isn't wired
 * up yet) — that case is `"unavailable"`, distinct from `"stale"`, since the
 * UI shouldn't claim a specific lag it doesn't actually know.
 */
export function getFreshness(
  indexedAt: string | number | null | undefined,
  now: number = Date.now(),
  staleThresholdMs: number = DEFAULT_STALE_THRESHOLD_MS,
): FreshnessInfo {
  if (indexedAt == null) {
    return { state: "unavailable", ageMs: null };
  }

  const indexedMs = typeof indexedAt === "number" ? indexedAt : new Date(indexedAt).getTime();
  if (Number.isNaN(indexedMs)) {
    return { state: "unavailable", ageMs: null };
  }

  const ageMs = Math.max(0, now - indexedMs);
  return { state: ageMs > staleThresholdMs ? "stale" : "fresh", ageMs };
}

/** Human-readable "Xs/Xm ago" for a known age. */
export function formatAge(ageMs: number): string {
  const seconds = Math.round(ageMs / 1000);
  if (seconds < 5) return "just now";
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  return `${hours}h ago`;
}
