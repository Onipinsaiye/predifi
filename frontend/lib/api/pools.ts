/**
 * Prediction pool API client.
 *
 * Thin, typed wrapper around the PrediFi backend's `GET /api/v1/pools`
 * endpoint. The exported {@link fetchPools} function is used as the SWR
 * fetcher (see `lib/hooks/usePools.ts`) so that pool data is cached and
 * deduplicated across the app.
 *
 * The shapes here mirror the backend OpenAPI schema (`PoolDoc` /
 * `PoolsResponse` in `backend/src/openapi.rs`).
 */

/**
 * Base URL of the PrediFi backend API.
 *
 * Configurable per environment via `NEXT_PUBLIC_API_BASE_URL`; falls back to
 * the local backend default so the dashboard works out of the box in dev.
 */
export const API_BASE_URL = process.env.NEXT_PUBLIC_API_BASE_URL ?? "http://localhost:3000";

/** WebSocket base derived from the HTTP API base. */
export function wsBaseUrl(): string {
  const http = API_BASE_URL.replace(/\/$/, "");
  if (http.startsWith("https://")) return http.replace(/^https/, "wss");
  if (http.startsWith("http://")) return http.replace(/^http/, "ws");
  return `ws://${http}`;
}

/**
 * A single prediction-market pool.
 *
 * Field ownership (see `docs/data-ownership.md`):
 * - `total_stake` — authoritative on-chain (pool contract); the indexer mirrors
 *   it and the API serves the mirrored value. Freshness: eventual (indexer lag).
 * - `state` / `result` / `end_time` — lifecycle fields owned by the chain; the
 *   API is a read-through mirror and must never invent transitions.
 * - `odds` (see {@link PoolDetail}) — derived by the backend from on-chain
 *   stakes; treat as a price snapshot, not a source of truth.
 */
export interface Pool {
  pool_id: number;
  name: string;
  category: string;
  /** Free-form tags assigned by the creator, e.g. `["btc", "price-prediction"]`. */
  tags: string[];
  /** Total amount staked across the pool, in the token's base units. */
  total_stake: number;
  /** Pool close time as a Unix timestamp (seconds). */
  end_time: number;
  created_at: string;
  state: string;
  creator: string;
  token: string;
  /** Settled outcome, or `null` while the pool is still open. */
  result: string | null;
  /** Optional market description. */
  description?: string;
  /**
   * When the indexer last mirrored this pool's on-chain state, as an ISO
   * timestamp. Absent when the backend doesn't report it (e.g. the indexer
   * is unreachable) — the UI must treat that as "freshness unavailable",
   * never as "fresh".
   */
  indexed_at?: string | null;
}

/**
 * Per-outcome stake and implied odds from `GET /api/v1/pools/:id`.
 *
 * `stake` is the chain-authoritative per-outcome balance mirrored by the
 * indexer; `odds` is a backend-derived price. When the two disagree with the
 * chain, the chain wins and the UI must show the on-chain stake.
 */
export interface OutcomeOdds {
  outcome: number;
  stake: number;
  odds: number;
}

/** Detailed pool payload including live odds. */
export interface PoolDetail extends Pool {
  odds: OutcomeOdds[];
  outcome_descriptions?: string[];
  /** Prediction count derived client-side from live WS updates when available. */
  prediction_count?: number;
}

/** Response body of `GET /api/v1/pools`. */
export interface PoolsResponse {
  pools: Pool[];
  total: number;
  limit: number;
  offset: number;
  status: string;
  category?: string | null;
  sort_by: string;
}

/** Filters accepted by `GET /api/v1/pools`. */
export interface PoolsQuery {
  /** Sort order. Defaults to `"new"` on the backend. */
  sort_by?: "popular" | "ending_soon" | "new";
  /** Category filter, e.g. `"Sports"` or `"Crypto"`. */
  category?: string;
  /** Tag filter — a pool matches if any of its tags overlap this list. */
  tags?: string[];
  /** Lifecycle filter. Defaults to `"active"` on the backend. */
  status?: "active" | "closed" | "settled";
  limit?: number;
  offset?: number;
}

/**
 * Build the request URL for a pools query.
 *
 * The returned string doubles as the SWR cache key: two calls with equal
 * filters produce an identical URL and therefore share one cache entry.
 */
export function poolsUrl(query: PoolsQuery = {}): string {
  const params = new URLSearchParams();
  if (query.sort_by) params.set("sort_by", query.sort_by);
  if (query.category) params.set("category", query.category);
  if (query.tags && query.tags.length > 0) params.set("tags", query.tags.join(","));
  if (query.status) params.set("status", query.status);
  if (query.limit != null) params.set("limit", String(query.limit));
  if (query.offset != null) params.set("offset", String(query.offset));

  const qs = params.toString();
  return `${API_BASE_URL}/api/v1/pools${qs ? `?${qs}` : ""}`;
}

/** Cache key / URL for a single pool detail fetch. */
export function poolDetailUrl(poolId: number | string): string {
  return `${API_BASE_URL}/api/v1/pools/${poolId}`;
}

/** Error raised when the pools endpoint responds with a non-2xx status. */
export class ApiError extends Error {
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

/** Error raised when a request exceeds the timeout. */
export class TimeoutError extends Error {
  constructor(message: string = "Request timeout") {
    super(message);
    this.name = "TimeoutError";
  }
}

/** Default timeout for API requests in milliseconds. */
const DEFAULT_REQUEST_TIMEOUT = 30000; // 30 seconds

/**
 * Execute a fetch with an optional timeout.
 *
 * @param url - The URL to fetch.
 * @param options - Fetch options and timeout configuration.
 * @param timeout - Timeout in milliseconds. Defaults to DEFAULT_REQUEST_TIMEOUT.
 * @throws {TimeoutError} When the request exceeds the timeout.
 * @throws {ApiError} When the response status is not 2xx.
 */
export async function fetchWithTimeout(
  url: string,
  options: RequestInit = {},
  timeout: number = DEFAULT_REQUEST_TIMEOUT,
): Promise<Response> {
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), timeout);

  try {
    const res = await fetch(url, {
      ...options,
      signal: controller.signal,
    });
    return res;
  } catch (err) {
    if (err instanceof Error && err.name === "AbortError") {
      throw new TimeoutError(`Request timeout after ${timeout}ms`);
    }
    throw err;
  } finally {
    clearTimeout(timeoutId);
  }
}

/** Type guard to validate PoolsResponse shape. */
function isPoolsResponse(obj: unknown): obj is PoolsResponse {
  if (!obj || typeof obj !== "object") return false;
  const response = obj as Record<string, unknown>;
  return (
    Array.isArray(response.pools) &&
    response.pools.every((p: unknown) => {
      if (!p || typeof p !== "object") return false;
      const pool = p as Record<string, unknown>;
      return (
        typeof pool.pool_id === "number" &&
        typeof pool.name === "string" &&
        typeof pool.category === "string" &&
        Array.isArray(pool.tags) &&
        typeof pool.total_stake === "number" &&
        typeof pool.end_time === "number" &&
        typeof pool.created_at === "string" &&
        typeof pool.state === "string" &&
        typeof pool.creator === "string" &&
        typeof pool.token === "string" &&
        (pool.result === null || typeof pool.result === "string")
      );
    }) &&
    typeof response.total === "number" &&
    typeof response.limit === "number" &&
    typeof response.offset === "number" &&
    typeof response.status === "string" &&
    typeof response.sort_by === "string"
  );
}

/**
 * SWR fetcher for pool data.
 *
 * @param url - A URL produced by {@link poolsUrl}.
 * @param timeout - Optional timeout in milliseconds. Defaults to DEFAULT_REQUEST_TIMEOUT.
 * @throws {ApiError} When the response status is not 2xx.
 * @throws {TimeoutError} When the request exceeds the timeout.
 */
export async function fetchPools(url: string, timeout?: number): Promise<PoolsResponse> {
  const res = await fetchWithTimeout(url, { headers: { Accept: "application/json" } }, timeout);

  if (!res.ok) {
    throw new ApiError(`Failed to load pools (HTTP ${res.status})`, res.status);
  }

  const body = await res.json();

  // Validate response shape at boundary
  if (!isPoolsResponse(body)) {
    throw new ApiError("Invalid pools response shape", 500);
  }

  return body;
}

/** Type guard to validate PoolDetail shape. */
function isPoolDetail(obj: unknown): obj is PoolDetail & { end_time?: number | string } {
  if (!obj || typeof obj !== "object") return false;
  const pool = obj as Record<string, unknown>;
  return (
    typeof pool.pool_id === "number" &&
    typeof pool.name === "string" &&
    typeof pool.category === "string" &&
    Array.isArray(pool.tags) &&
    typeof pool.total_stake === "number" &&
    (typeof pool.end_time === "number" || typeof pool.end_time === "string") &&
    typeof pool.created_at === "string" &&
    typeof pool.state === "string" &&
    typeof pool.creator === "string" &&
    typeof pool.token === "string" &&
    (pool.result === null || typeof pool.result === "string") &&
    (Array.isArray(pool.odds) || pool.odds === undefined)
  );
}

/** Type guard to validate wrapped pool response. */
function isWrappedPoolDetail(
  obj: unknown,
): obj is { data?: PoolDetail & { end_time?: number | string }; error?: string } {
  if (!obj || typeof obj !== "object") return false;
  const body = obj as Record<string, unknown>;
  return (
    ("data" in body && (body.data === undefined || isPoolDetail(body.data))) ||
    ("error" in body && typeof body.error === "string")
  );
}

/**
 * Fetch a single pool with live odds.
 *
 * Handles both raw `PoolWithOdds` JSON and wrapped `{ data: ... }` API responses.
 *
 * @param url - The pool detail URL.
 * @param timeout - Optional timeout in milliseconds. Defaults to DEFAULT_REQUEST_TIMEOUT.
 * @throws {ApiError} When the response status is not 2xx.
 * @throws {TimeoutError} When the request exceeds the timeout.
 */
export async function fetchPoolDetail(url: string, timeout?: number): Promise<PoolDetail> {
  const res = await fetchWithTimeout(url, { headers: { Accept: "application/json" } }, timeout);

  if (!res.ok) {
    throw new ApiError(`Failed to load pool (HTTP ${res.status})`, res.status);
  }

  const body = await res.json();

  // Validate response shape at boundary
  if (!isPoolDetail(body) && !isWrappedPoolDetail(body)) {
    throw new ApiError("Invalid pool response shape", 500);
  }

  if (body && typeof body === "object" && "error" in body && body.error) {
    throw new ApiError(String(body.error), 404);
  }

  const pool =
    body && typeof body === "object" && "data" in body && body.data
      ? body.data
      : (body as PoolDetail & { end_time?: number | string });

  if (!isPoolDetail(pool)) {
    throw new ApiError("Invalid pool detail shape after extraction", 500);
  }

  const rawEnd = pool.end_time;
  const endTime =
    typeof rawEnd === "string"
      ? Math.floor(new Date(rawEnd).getTime() / 1000)
      : Number(rawEnd ?? 0);

  // Fallback contract: a missing `odds` array means "no price data yet" and is
  // surfaced as an empty list rather than `undefined`, so consumers never have
  // to branch on presence. Balances (`total_stake`, per-outcome `stake`) are
  // never defaulted — a missing balance is a contract violation, not zero.
  return {
    ...pool,
    end_time: endTime,
    odds: pool.odds ?? [],
  };
}

/** Helper function to fetch a pool detail by ID directly.
 * @param id - The pool ID.
 * @param timeout - Optional timeout in milliseconds.
 */
export async function fetchPoolById(id: string | number, timeout?: number): Promise<PoolDetail> {
  return fetchPoolDetail(poolDetailUrl(id), timeout);
}

/**
 * Live event pushed over `/api/v1/ws`.
 *
 * Live events are a *hint*, not a source of truth: they may arrive out of
 * order or be dropped. Consumers must reconcile against the next API/chain
 * read (see `docs/data-ownership.md` → "Conflict behavior").
 */
export interface PoolLiveEvent {
  type: string;
  pool_id: number;
  user_address?: string;
  outcome?: number;
  amount?: number;
}

/**
 * Kinds of market activity surfaced in the timeline.
 *
 * These are *indexed* events: they describe what the indexer observed on
 * chain, not authoritative contract state. The UI must present them as a
 * chronological activity log and never treat them as a substitute for a
 * fresh chain read (see `docs/data-ownership.md`).
 */
export type MarketEventType = "creation" | "trade" | "close" | "resolution" | "claim";

/** Event types the timeline knows how to render. */
export const MARKET_EVENT_TYPES: readonly MarketEventType[] = [
  "creation",
  "trade",
  "close",
  "resolution",
  "claim",
];

/**
 * A single normalized market activity event.
 *
 * Optional fields are normalized to `null` (never `undefined`) so consumers
 * can branch on presence without worrying about missing keys. `timestamp` is
 * the *source* timestamp (when the chain event occurred), not when the
 * indexer observed it.
 */
export interface MarketEvent {
  /** Event kind, or `null` when the backend reports an unsupported type. */
  type: MarketEventType | null;
  /** Raw type string as reported by the backend (useful for debugging). */
  raw_type: string;
  pool_id: number;
  /** Canonical chain identity: transaction hash + log/event index. */
  tx_hash: string;
  event_index: number;
  /** Source timestamp as a Unix timestamp (seconds). */
  timestamp: number;
  /** Actor address, when the event has one. */
  user_address: string | null;
  /** Outcome index, when the event references one. */
  outcome: number | null;
  /** Amount in the token's base units, when the event carries one. */
  amount: number | null;
  /** Ledger/block number, when reported. */
  block_number: number | null;
}

/** Response body of `GET /api/v1/pools/:id/events`. */
export interface MarketEventsResponse {
  events: MarketEvent[];
  total: number;
  limit: number;
  offset: number;
}

/** Query parameters accepted by `GET /api/v1/pools/:id/events`. */
export interface MarketEventsQuery {
  limit?: number;
  offset?: number;
}

/** Cache key / URL for a pool's activity timeline. */
export function marketEventsUrl(poolId: number | string, query: MarketEventsQuery = {}): string {
  const params = new URLSearchParams();
  if (query.limit != null) params.set("limit", String(query.limit));
  if (query.offset != null) params.set("offset", String(query.offset));
  const qs = params.toString();
  return `${API_BASE_URL}/api/v1/pools/${poolId}/events${qs ? `?${qs}` : ""}`;
}

/** Canonical identity of an event on chain. */
export function marketEventKey(event: Pick<MarketEvent, "tx_hash" | "event_index">): string {
  return `${event.tx_hash}:${event.event_index}`;
}

function asFiniteNumber(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim() !== "") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return null;
}

function asTimestamp(value: unknown): number {
  const numeric = asFiniteNumber(value);
  if (numeric != null) {
    // Heuristic: values that look like milliseconds are normalized to seconds.
    return numeric > 1e12 ? Math.floor(numeric / 1000) : Math.floor(numeric);
  }
  if (typeof value === "string") {
    const parsed = Date.parse(value);
    if (Number.isFinite(parsed)) return Math.floor(parsed / 1000);
  }
  return 0;
}

function normalizeEventType(value: unknown): MarketEventType | null {
  if (typeof value !== "string") return null;
  const lower = value.toLowerCase();
  return (MARKET_EVENT_TYPES as readonly string[]).includes(lower)
    ? (lower as MarketEventType)
    : null;
}

/**
 * Normalize a raw event payload from the backend.
 *
 * Returns `null` when the payload lacks the canonical chain identity
 * (`tx_hash` + `event_index`) — such an event cannot be deduplicated or
 * ordered reliably and is dropped rather than guessed at.
 */
export function normalizeMarketEvent(raw: unknown): MarketEvent | null {
  if (!raw || typeof raw !== "object") return null;
  const obj = raw as Record<string, unknown>;

  const txHash =
    typeof obj.tx_hash === "string"
      ? obj.tx_hash
      : typeof obj.transaction_hash === "string"
        ? obj.transaction_hash
        : null;
  const eventIndex = asFiniteNumber(obj.event_index ?? obj.log_index ?? obj.index);
  const poolId = asFiniteNumber(obj.pool_id);

  if (!txHash || eventIndex == null || poolId == null) return null;

  const rawType = typeof obj.type === "string" ? obj.type : "";

  return {
    type: normalizeEventType(rawType),
    raw_type: rawType,
    pool_id: poolId,
    tx_hash: txHash,
    event_index: eventIndex,
    timestamp: asTimestamp(obj.timestamp ?? obj.block_time ?? obj.created_at),
    user_address: typeof obj.user_address === "string" ? obj.user_address : null,
    outcome: asFiniteNumber(obj.outcome),
    amount: asFiniteNumber(obj.amount),
    block_number: asFiniteNumber(obj.block_number),
  };
}

/**
 * Deduplicate and order events for the timeline.
 *
 * - Duplicates are collapsed by canonical chain identity
 *   ({@link marketEventKey}); the first occurrence wins.
 * - Events are sorted by source timestamp ascending, with `tx_hash` and
 *   `event_index` as stable tie-breakers so the order is deterministic and
 *   consistent across pagination boundaries.
 */
export function mergeMarketEvents(events: readonly MarketEvent[]): MarketEvent[] {
  const seen = new Set<string>();
  const unique: MarketEvent[] = [];
  for (const event of events) {
    const key = marketEventKey(event);
    if (seen.has(key)) continue;
    seen.add(key);
    unique.push(event);
  }

  return unique.sort((a, b) => {
    if (a.timestamp !== b.timestamp) return a.timestamp - b.timestamp;
    if (a.tx_hash !== b.tx_hash) return a.tx_hash < b.tx_hash ? -1 : 1;
    return a.event_index - b.event_index;
  });
}

/** Type guard for a normalized event payload. */
function isMarketEventsResponse(obj: unknown): obj is { events: unknown[] } {
  if (!obj || typeof obj !== "object") return false;
  const body = obj as Record<string, unknown>;
  return Array.isArray(body.events);
}

/**
 * Fetch a page of a pool's activity timeline.
 *
 * Raw events are normalized, deduplicated by canonical chain identity, and
 * sorted by source timestamp. Callers paginating through results should merge
 * pages with {@link mergeMarketEvents} to keep ordering stable across page
 * boundaries.
 *
 * @param url - A URL produced by {@link marketEventsUrl}.
 * @param timeout - Optional timeout in milliseconds. Defaults to DEFAULT_REQUEST_TIMEOUT.
 * @throws {ApiError} When the response status is not 2xx or the shape is invalid.
 * @throws {TimeoutError} When the request exceeds the timeout.
 */
export async function fetchMarketEvents(
  url: string,
  timeout?: number,
): Promise<MarketEventsResponse> {
  const res = await fetchWithTimeout(url, { headers: { Accept: "application/json" } }, timeout);

  if (!res.ok) {
    throw new ApiError(`Failed to load market events (HTTP ${res.status})`, res.status);
  }

  const body = await res.json();

  if (!isMarketEventsResponse(body)) {
    throw new ApiError("Invalid market events response shape", 500);
  }

  const normalized = body.events
    .map(normalizeMarketEvent)
    .filter((event): event is MarketEvent => event !== null);

  const record = body as Record<string, unknown>;
  const total = asFiniteNumber(record.total) ?? normalized.length;
  const limit = asFiniteNumber(record.limit) ?? normalized.length;
  const offset = asFiniteNumber(record.offset) ?? 0;

  return {
    events: mergeMarketEvents(normalized),
    total,
    limit,
    offset,
  };
}
