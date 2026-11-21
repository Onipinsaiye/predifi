/**
 * Market watchlist API client (Issue #280).
 *
 * Typed wrappers around the watchlist endpoints:
 *
 *   GET    /api/v1/users/:address/watchlists
 *   POST   /api/v1/users/:address/watchlists
 *   GET    /api/v1/users/:address/watchlists/:id
 *   PATCH  /api/v1/users/:address/watchlists/:id
 *   DELETE /api/v1/users/:address/watchlists/:id
 *   POST   /api/v1/users/:address/watchlists/:id/markets
 *   DELETE /api/v1/users/:address/watchlists/:id/markets/:pool_id
 *   GET    /api/v1/watchlists/public/:slug   (unauthenticated)
 *
 * Privacy contract:
 * - Watchlists are private by default.
 * - `public_slug` is present only when `public = true`.
 * - Revoking public access (`public = false`) makes the old slug permanently
 *   invalid; re-enabling generates a fresh slug.
 * - The public read path (`/watchlists/public/:slug`) returns only market
 *   IDs — never the owner address.
 */

import { API_BASE_URL, ApiError, fetchWithTimeout } from "@/lib/api/pools";

// ─── Types ────────────────────────────────────────────────────────────────────

/** Full watchlist as returned to the authenticated owner. */
export interface Watchlist {
  id: number;
  owner_address: string;
  name: string;
  description: string | null;
  /** True when the watchlist is publicly accessible via `public_slug`. */
  public: boolean;
  /**
   * Opaque share token. Present when `public = true`, absent otherwise.
   * Use `publicWatchlistUrl(slug)` to build the shareable link.
   */
  public_slug: string | null;
  created_at: string;
  updated_at: string;
  market_ids: number[];
}

/** Watchlist as returned to unauthenticated public viewers. Never includes the owner address. */
export interface PublicWatchlist {
  name: string;
  description: string | null;
  market_ids: number[];
}

export interface CreateWatchlistInput {
  name: string;
  description?: string | null;
  /** Defaults to false (private) when omitted. */
  public?: boolean;
}

export interface UpdateWatchlistInput {
  name?: string;
  description?: string | null;
  /**
   * Set to `false` to revoke public access.
   * Set to `true` to generate (or keep) a public share link.
   */
  public?: boolean;
}

// ─── URL helpers ──────────────────────────────────────────────────────────────

export function watchlistsUrl(address: string): string {
  return `${API_BASE_URL}/api/v1/users/${encodeURIComponent(address)}/watchlists`;
}

export function watchlistUrl(address: string, id: number): string {
  return `${API_BASE_URL}/api/v1/users/${encodeURIComponent(address)}/watchlists/${id}`;
}

export function watchlistMarketsUrl(address: string, id: number): string {
  return `${watchlistUrl(address, id)}/markets`;
}

export function watchlistMarketUrl(address: string, id: number, poolId: number): string {
  return `${watchlistUrl(address, id)}/markets/${poolId}`;
}

export function publicWatchlistApiUrl(slug: string): string {
  return `${API_BASE_URL}/api/v1/watchlists/public/${encodeURIComponent(slug)}`;
}

/**
 * Build the shareable frontend URL for a public watchlist.
 * The UI renders this URL in the share button so users can copy it.
 */
export function publicWatchlistShareUrl(slug: string): string {
  const base = typeof window !== "undefined" ? window.location.origin : "https://predifi.app";
  return `${base}/watchlists/${encodeURIComponent(slug)}`;
}

// ─── Type guards ─────────────────────────────────────────────────────────────

function isWatchlist(obj: unknown): obj is Watchlist {
  if (!obj || typeof obj !== "object") return false;
  const w = obj as Record<string, unknown>;
  return (
    typeof w.id === "number" &&
    typeof w.owner_address === "string" &&
    typeof w.name === "string" &&
    (w.description === null || typeof w.description === "string") &&
    typeof w.public === "boolean" &&
    (w.public_slug === null || typeof w.public_slug === "string") &&
    typeof w.created_at === "string" &&
    typeof w.updated_at === "string" &&
    Array.isArray(w.market_ids) &&
    w.market_ids.every((id) => typeof id === "number")
  );
}

function isPublicWatchlist(obj: unknown): obj is PublicWatchlist {
  if (!obj || typeof obj !== "object") return false;
  const w = obj as Record<string, unknown>;
  return (
    typeof w.name === "string" &&
    (w.description === null || typeof w.description === "string") &&
    Array.isArray(w.market_ids) &&
    w.market_ids.every((id) => typeof id === "number")
  );
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

function authHeaders(bearerToken: string): Record<string, string> {
  return {
    Accept: "application/json",
    "Content-Type": "application/json",
    Authorization: `Bearer ${bearerToken}`,
  };
}

function extractPayload<T>(body: unknown): T {
  if (body && typeof body === "object" && "data" in body && body.data != null) {
    return (body as { data: T }).data;
  }
  return body as T;
}

// ─── CRUD ─────────────────────────────────────────────────────────────────────

/** Fetch all watchlists for `address` (authenticated). */
export async function fetchWatchlists(
  address: string,
  bearerToken: string,
): Promise<Watchlist[]> {
  const res = await fetchWithTimeout(watchlistsUrl(address), {
    headers: authHeaders(bearerToken),
  });
  if (!res.ok) throw new ApiError(`Failed to fetch watchlists (HTTP ${res.status})`, res.status);
  const body = await res.json();
  const payload = extractPayload<unknown>(body);
  if (!Array.isArray(payload) || !payload.every(isWatchlist)) {
    throw new ApiError("Invalid watchlists response shape", 500);
  }
  return payload;
}

/** Create a new watchlist (authenticated). */
export async function createWatchlist(
  address: string,
  bearerToken: string,
  input: CreateWatchlistInput,
): Promise<Watchlist> {
  const res = await fetchWithTimeout(watchlistsUrl(address), {
    method: "POST",
    headers: authHeaders(bearerToken),
    body: JSON.stringify(input),
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({}));
    throw new ApiError(
      String((err as Record<string, unknown>)?.error ?? `HTTP ${res.status}`),
      res.status,
    );
  }
  const body = await res.json();
  const payload = extractPayload<unknown>(body);
  if (!isWatchlist(payload)) throw new ApiError("Invalid watchlist response shape", 500);
  return payload;
}

/** Fetch a single watchlist by ID (authenticated). */
export async function fetchWatchlist(
  address: string,
  id: number,
  bearerToken: string,
): Promise<Watchlist | null> {
  const res = await fetchWithTimeout(watchlistUrl(address, id), {
    headers: authHeaders(bearerToken),
  });
  if (res.status === 404) return null;
  if (!res.ok) throw new ApiError(`Failed to fetch watchlist (HTTP ${res.status})`, res.status);
  const body = await res.json();
  const payload = extractPayload<unknown>(body);
  if (!isWatchlist(payload)) throw new ApiError("Invalid watchlist response shape", 500);
  return payload;
}

/**
 * Update a watchlist's metadata or visibility.
 *
 * - `public: false` revokes the share link. The previously shared URL stops working.
 * - `public: true` generates a fresh slug if the previous one was revoked.
 */
export async function updateWatchlist(
  address: string,
  id: number,
  bearerToken: string,
  input: UpdateWatchlistInput,
): Promise<Watchlist> {
  const res = await fetchWithTimeout(watchlistUrl(address, id), {
    method: "PATCH",
    headers: authHeaders(bearerToken),
    body: JSON.stringify(input),
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({}));
    throw new ApiError(
      String((err as Record<string, unknown>)?.error ?? `HTTP ${res.status}`),
      res.status,
    );
  }
  const body = await res.json();
  const payload = extractPayload<unknown>(body);
  if (!isWatchlist(payload)) throw new ApiError("Invalid watchlist response shape", 500);
  return payload;
}

/** Delete a watchlist and all its markets. */
export async function deleteWatchlist(
  address: string,
  id: number,
  bearerToken: string,
): Promise<void> {
  const res = await fetchWithTimeout(watchlistUrl(address, id), {
    method: "DELETE",
    headers: authHeaders(bearerToken),
  });
  if (!res.ok) throw new ApiError(`Failed to delete watchlist (HTTP ${res.status})`, res.status);
}

/** Add a pool to a watchlist. Idempotent. */
export async function addMarketToWatchlist(
  address: string,
  watchlistId: number,
  poolId: number,
  bearerToken: string,
): Promise<void> {
  const res = await fetchWithTimeout(watchlistMarketsUrl(address, watchlistId), {
    method: "POST",
    headers: authHeaders(bearerToken),
    body: JSON.stringify({ pool_id: poolId }),
  });
  if (!res.ok) throw new ApiError(`Failed to add market (HTTP ${res.status})`, res.status);
}

/** Remove a pool from a watchlist. */
export async function removeMarketFromWatchlist(
  address: string,
  watchlistId: number,
  poolId: number,
  bearerToken: string,
): Promise<void> {
  const res = await fetchWithTimeout(watchlistMarketUrl(address, watchlistId, poolId), {
    method: "DELETE",
    headers: authHeaders(bearerToken),
  });
  if (!res.ok) throw new ApiError(`Failed to remove market (HTTP ${res.status})`, res.status);
}

// ─── Public read path ─────────────────────────────────────────────────────────

/**
 * Fetch a public watchlist by its opaque share slug (unauthenticated).
 *
 * Returns `null` when the watchlist is not found or has been revoked.
 * The returned shape never includes the owner address.
 */
export async function fetchPublicWatchlist(slug: string): Promise<PublicWatchlist | null> {
  const res = await fetchWithTimeout(publicWatchlistApiUrl(slug), {
    headers: { Accept: "application/json" },
  });
  if (res.status === 404) return null;
  if (!res.ok) throw new ApiError(`Failed to fetch public watchlist (HTTP ${res.status})`, res.status);
  const body = await res.json();
  const payload = extractPayload<unknown>(body);
  if (!isPublicWatchlist(payload)) throw new ApiError("Invalid public watchlist response shape", 500);
  return payload;
}
