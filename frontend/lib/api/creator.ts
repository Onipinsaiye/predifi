/**
 * Creator dashboard API client (Issue #279).
 *
 * Thin, typed wrappers around:
 *   GET /api/v1/creators/:address/stats
 *   GET /api/v1/pools?creator=:address  (uses existing pools endpoint)
 */

import { API_BASE_URL, ApiError, fetchWithTimeout } from "@/lib/api/pools";
import { fetchPools, poolsUrl } from "@/lib/api/pools";
import type { PoolsQuery, PoolsResponse } from "@/lib/api/pools";
export type { Pool, PoolsQuery, PoolsResponse } from "@/lib/api/pools";

// ─── Types ────────────────────────────────────────────────────────────────────

/**
 * Aggregate stats for a pool creator as returned by
 * `GET /api/v1/creators/:address/stats`.
 */
export interface CreatorStats {
  creator: string;
  pools_created: number;
  pools_reward_eligible: number;
  total_volume: number;
}

// ─── URL helpers ──────────────────────────────────────────────────────────────

export function creatorStatsUrl(address: string): string {
  return `${API_BASE_URL}/api/v1/creators/${encodeURIComponent(address)}/stats`;
}

/** Build a pools-query URL pre-filtered to `creator`. */
export function creatorPoolsUrl(
  creator: string,
  query: Omit<PoolsQuery, "creator"> = {},
): string {
  return poolsUrl({ ...query, creator });
}

// ─── Type guard ───────────────────────────────────────────────────────────────

function isCreatorStats(obj: unknown): obj is CreatorStats {
  if (!obj || typeof obj !== "object") return false;
  const s = obj as Record<string, unknown>;
  return (
    typeof s.creator === "string" &&
    typeof s.pools_created === "number" &&
    typeof s.pools_reward_eligible === "number" &&
    typeof s.total_volume === "number"
  );
}

// ─── Fetch ────────────────────────────────────────────────────────────────────

/**
 * Fetch aggregate creator stats for `address`.
 *
 * Returns `null` when the creator has no stats row yet (404).
 * Throws `ApiError` on other non-2xx responses.
 */
export async function fetchCreatorStats(
  address: string,
  bearerToken?: string,
): Promise<CreatorStats | null> {
  const headers: Record<string, string> = { Accept: "application/json" };
  if (bearerToken) {
    headers["Authorization"] = `Bearer ${bearerToken}`;
  }

  const res = await fetchWithTimeout(creatorStatsUrl(address), { headers });

  if (res.status === 404) return null;

  if (!res.ok) {
    throw new ApiError(
      `Failed to fetch creator stats (HTTP ${res.status})`,
      res.status,
    );
  }

  const body = await res.json();
  const payload =
    body && typeof body === "object" && "data" in body && body.data
      ? body.data
      : body;

  if (!isCreatorStats(payload)) {
    throw new ApiError("Invalid creator stats response shape", 500);
  }

  return payload;
}

/**
 * Fetch pools authored by `creator`.
 *
 * Uses the existing `GET /api/v1/pools?creator=` filter. The caller is
 * responsible for the status/sort filters relevant to the dashboard view.
 */
export async function fetchCreatorPools(
  creator: string,
  query: Omit<PoolsQuery, "creator"> = {},
): Promise<PoolsResponse> {
  return fetchPools(creatorPoolsUrl(creator, query));
}
