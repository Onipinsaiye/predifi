"use client";

import useSWR from "swr";
import {
  fetchPools,
  poolsUrl,
  type Pool,
  type PoolsQuery,
  type PoolsResponse,
} from "@/lib/api/pools";
import {
  fetchMarketEvents,
  marketEventsUrl,
  type MarketEvent,
  type MarketEventsQuery,
  type MarketEventsResponse,
} from "@/lib/api/marketEvents";

/** Return value of {@link usePools}. */
export interface UsePoolsResult {
  /** The fetched pools (empty while loading or on error). */
  pools: Pool[];
  /** Total number of pools matching the query (for pagination). */
  total: number;
  /** Current page limit returned by the backend. */
  limit: number;
  /** Current page offset returned by the backend. */
  offset: number;
  /** The raw response, or `undefined` until the first load resolves. */
  data: PoolsResponse | undefined;
  /** True during the initial load when no cached data is available yet. */
  isLoading: boolean;
  /** True when the most recent request failed. */
  isError: boolean;
  /** True when serving cached data despite a failing/outage API response (stale-while-error). */
  isStale: boolean;
  /** True when an API outage or request failure is preventing active mutations/writes. */
  isReadOnly: boolean;
  /** The error from the most recent failed request, if any. */
  error: Error | undefined;
  /** Manually revalidate (e.g. a "retry" button). */
  refresh: () => void;
  /**
   * The SWR cache key — the full URL used for the request.
   * Expose it so callers can use it with `useSWRMutation` or `mutate()`.
   */
  cacheKey: string;
}

/**
 * usePools — SWR-cached access to the prediction pool list.
 *
 * Prediction-pool metadata is effectively static between updates, so this hook
 * leans on the cache: identical queries share a single cache entry, concurrent
 * requests are deduplicated, cached data is served instantly on remount, and
 * (via the global {@link SWRProvider} config) the data is not revalidated on
 * window focus or reconnect.
 *
 * Pass a {@link PoolsQuery} — typically obtained from {@link usePoolsQuery} —
 * to fetch a filtered/sorted page. The URL produced by {@link poolsUrl} doubles
 * as the SWR cache key, so two calls with equivalent filter objects share the
 * same cache entry automatically.
 *
 * @example
 * // With usePoolsQuery for URL-synced state:
 * const { poolsQuery, setSearch, setPage } = usePoolsQuery();
 * const { pools, total, isLoading } = usePools(poolsQuery);
 *
 * @example
 * // Standalone usage with inline options:
 * const { pools, isLoading, isError, refresh } = usePools({ status: "active", sort_by: "new" });
 */
export function usePools(query: PoolsQuery = {}): UsePoolsResult {
  const key = poolsUrl(query);

  const { data, error, isLoading, mutate } = useSWR<PoolsResponse>(
    key,
    fetchPools,
  );

  const hasData = Boolean(data);
  const isStale = Boolean(error && hasData);
  const isReadOnly = Boolean(error);

  return {
    pools: data?.pools ?? [],
    total: data?.total ?? 0,
    limit: data?.limit ?? 0,
    offset: data?.offset ?? 0,
    data,
    isLoading,
    isError: Boolean(error),
    isStale,
    isReadOnly,
    error: error as Error | undefined,
    refresh: () => {
      void mutate();
    },
    cacheKey: key,
  };
}

/**
 * Field-level ownership for pool data surfaced by {@link usePools}.
 *
 * The indexer/API is the source of truth for every pool field below; the hook
 * never derives or overrides them. When a request fails while cached data is
 * available (`isStale`), the last known authoritative values are served as-is
 * rather than being replaced with defaults. Defaults (`[]` / `0`) are only
 * applied when no data has ever loaded.
 *
 * See `docs/data-ownership.md` for the full matrix and conflict behavior.
 */
export const POOL_FIELD_OWNERSHIP = {
  pools: { source: "indexer", fallback: "empty-array", staleBehavior: "serve-cached" },
  total: { source: "indexer", fallback: "zero", staleBehavior: "serve-cached" },
  limit: { source: "indexer", fallback: "zero", staleBehavior: "serve-cached" },
  offset: { source: "indexer", fallback: "zero", staleBehavior: "serve-cached" },
} as const;

/** Return value of {@link useMarketEvents}. */
export interface UseMarketEventsResult {
  /** Deduplicated, chronologically ordered events (empty while loading or on error). */
  events: MarketEvent[];
  /** Total number of events matching the query (for pagination). */
  total: number;
  /** Current page limit returned by the backend. */
  limit: number;
  /** Current page offset returned by the backend. */
  offset: number;
  /** The raw response, or `undefined` until the first load resolves. */
  data: MarketEventsResponse | undefined;
  /** True during the initial load when no cached data is available yet. */
  isLoading: boolean;
  /** True when the most recent request failed. */
  isError: boolean;
  /** True when serving cached data despite a failing/outage API response (stale-while-error). */
  isStale: boolean;
  /** The error from the most recent failed request, if any. */
  error: Error | undefined;
  /** Manually revalidate (e.g. a "retry" button). */
  refresh: () => void;
  /** The SWR cache key — the full URL used for the request. */
  cacheKey: string;
}

/**
 * Canonical chain identity for a market event.
 *
 * Two deliveries of the same on-chain event (e.g. an indexer replay or a
 * pagination overlap) must collapse to a single timeline entry. The identity is
 * the transaction hash plus the log index; when a log index is unavailable we
 * fall back to the event id, and finally to a composite of the remaining
 * identifying fields so that events are never silently dropped.
 */
export function marketEventIdentity(event: MarketEvent): string {
  const txHash = event.tx_hash ?? "";
  const logIndex = event.log_index;
  if (txHash && logIndex !== undefined && logIndex !== null) {
    return `${txHash}:${logIndex}`;
  }
  if (event.id) {
    return `id:${event.id}`;
  }
  return [
    txHash,
    event.market_id ?? "",
    event.event_type ?? "",
    event.timestamp ?? "",
  ].join(":");
}

/** Source timestamp (ms since epoch) used for chronological ordering. */
function eventTimestamp(event: MarketEvent): number {
  const raw = event.timestamp;
  if (raw === undefined || raw === null || raw === "") {
    return 0;
  }
  const parsed = typeof raw === "number" ? raw : Date.parse(raw);
  return Number.isFinite(parsed) ? parsed : 0;
}

/**
 * Merge event pages, deduplicate by canonical chain identity and sort
 * chronologically (oldest first). Ties are broken by log index then id so the
 * ordering is stable across pagination boundaries.
 */
export function mergeMarketEvents(pages: MarketEvent[][]): MarketEvent[] {
  const seen = new Set<string>();
  const merged: MarketEvent[] = [];
  for (const page of pages) {
    for (const event of page) {
      const identity = marketEventIdentity(event);
      if (seen.has(identity)) {
        continue;
      }
      seen.add(identity);
      merged.push(event);
    }
  }
  return merged.sort((a, b) => {
    const delta = eventTimestamp(a) - eventTimestamp(b);
    if (delta !== 0) {
      return delta;
    }
    const logDelta = (a.log_index ?? 0) - (b.log_index ?? 0);
    if (logDelta !== 0) {
      return logDelta;
    }
    return marketEventIdentity(a).localeCompare(marketEventIdentity(b));
  });
}

/**
 * useMarketEvents — SWR-cached, paginated market activity timeline.
 *
 * Fetches every page up to the requested `offset`/`limit` window, merges the
 * pages, deduplicates events by canonical chain identity and returns them in
 * chronological order. Indexed events are presented as-is; they are never
 * treated as authoritative contract state.
 *
 * @example
 * const { events, isLoading, isError, refresh } = useMarketEvents({ market_id: "0xabc" });
 */
export function useMarketEvents(
  query: MarketEventsQuery = {},
): UseMarketEventsResult {
  const key = marketEventsUrl(query);

  const { data, error, isLoading, mutate } = useSWR<MarketEventsResponse>(
    key,
    fetchMarketEvents,
  );

  const events = data?.events ? mergeMarketEvents([data.events]) : [];
  const isStale = Boolean(error && data);

  return {
    events,
    total: data?.total ?? 0,
    limit: data?.limit ?? 0,
    offset: data?.offset ?? 0,
    data,
    isLoading,
    isError: Boolean(error),
    isStale,
    error: error as Error | undefined,
    refresh: () => {
      void mutate();
    },
    cacheKey: key,
  };
}
