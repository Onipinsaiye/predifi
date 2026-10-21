/**
 * Shared API fixtures for core-route component tests (#84).
 *
 * These mirror the shapes validated at the API boundary (`lib/api/pools.ts`,
 * `lib/api/profile.ts`, `lib/api/tags.ts`, `lib/api/notifications.ts`) so the
 * pool list, pool detail, user history, and dashboard tests all exercise the
 * same realistic payloads instead of each inventing its own mock.
 */

import type { Pool, PoolsResponse, PoolDetail } from "@/lib/api/pools";
import type { UserProfile, ClaimStatus } from "@/lib/api/profile";
import type { NotificationsResponse } from "@/lib/api/notifications";

export const TEST_ADDRESS = "0x1234567890abcdef1234567890abcdef12345678";

// ─── Pools ────────────────────────────────────────────────────────────────────

export const TEST_POOL: Pool = {
  pool_id: 1,
  name: "Will BTC close above $100k?",
  category: "Crypto",
  tags: ["btc", "price"],
  total_stake: 125_000,
  end_time: 1_800_000_000,
  created_at: "2025-01-01T00:00:00Z",
  state: "active",
  creator: "0xcreator0000000000000000000000000000000001",
  token: "XLM",
  result: null,
};

export const TEST_POOL_TWO: Pool = {
  pool_id: 2,
  name: "Will it rain in Lagos on Friday?",
  category: "Other",
  tags: [],
  total_stake: 4_200,
  end_time: 1_800_100_000,
  created_at: "2025-01-02T00:00:00Z",
  state: "closed",
  creator: "0xcreator0000000000000000000000000000000002",
  token: "XLM",
  result: null,
};

/** Build a list response, defaulting to a single pool. */
export function poolsResponse(
  pools: Pool[] = [TEST_POOL],
  overrides: Partial<PoolsResponse> = {},
): PoolsResponse {
  return {
    pools,
    total: pools.length,
    limit: 20,
    offset: 0,
    status: "active",
    sort_by: "new",
    ...overrides,
  };
}

export const EMPTY_POOLS_RESPONSE = poolsResponse([]);

// ─── Pool detail ──────────────────────────────────────────────────────────────

export const TEST_POOL_DETAIL: PoolDetail = {
  ...TEST_POOL,
  odds: [
    { outcome: 0, stake: 80_000, odds: 1.56 },
    { outcome: 1, stake: 45_000, odds: 2.78 },
  ],
  outcome_descriptions: ["Yes", "No"],
};

/** A pool detail whose odds have not been published yet (partial data). */
export const TEST_POOL_DETAIL_NO_ODDS: PoolDetail = {
  ...TEST_POOL_DETAIL,
  odds: [],
};

// ─── Profile / user history ───────────────────────────────────────────────────

export const TEST_CLAIM: ClaimStatus = {
  prediction_id: 101,
  pool_id: 1,
  pool_name: "Will BTC close above $100k?",
  outcome: 0,
  amount: 500,
  pool_state: "settled",
  pool_result: "0",
  is_winner: true,
  claimed: false,
  claimed_amount: 0,
  claim_window_expires_at: "2025-06-01T00:00:00Z",
  claim_expired: false,
};

export const TEST_CLAIM_PENDING: ClaimStatus = {
  ...TEST_CLAIM,
  prediction_id: 102,
  pool_id: 2,
  pool_name: "Will it rain in Lagos on Friday?",
  pool_state: "active",
  pool_result: null,
  is_winner: null,
  claimed: false,
};

export function profileFixture(
  overrides: Partial<UserProfile> = {},
): UserProfile {
  return {
    address: TEST_ADDRESS,
    stats: {
      total_predictions: 2,
      wins: 1,
      losses: 0,
      pending: 1,
      win_rate: 100,
      total_staked: 1_000,
      total_earnings: 250,
      active_positions: 1,
    },
    claims: [TEST_CLAIM, TEST_CLAIM_PENDING],
    performance: [
      { day: "2025-01-01", staked: 500, earnings: 0, predictions: 1 },
      { day: "2025-01-02", staked: 500, earnings: 250, predictions: 1 },
    ],
    ...overrides,
  };
}

/** A profile with no prediction history (empty state). */
export const EMPTY_PROFILE: UserProfile = profileFixture({
  stats: {
    total_predictions: 0,
    wins: 0,
    losses: 0,
    pending: 0,
    win_rate: 0,
    total_staked: 0,
    total_earnings: 0,
    active_positions: 0,
  },
  claims: [],
  performance: [],
});

// ─── Tags / notifications ─────────────────────────────────────────────────────

export const TEST_TAGS = ["btc", "price", "weather"];

export function notificationsResponse(): NotificationsResponse {
  return {
    address: TEST_ADDRESS,
    notifications: [],
    unread_count: 0,
    limit: 20,
    offset: 0,
  };
}

// ─── Fetch mocking ────────────────────────────────────────────────────────────

export type FetchHandler = (
  url: string,
  init?: RequestInit,
) => Response | Promise<Response>;

export function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  } as unknown as Response;
}

export function errorResponse(
  status = 500,
  body: unknown = { error: "boom" },
): Response {
  return {
    ok: false,
    status,
    json: async () => body,
  } as unknown as Response;
}

/** A promise you can settle later — used to hold a request in the loading state. */
export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

export interface ApiFetchHandlers {
  pools?: FetchHandler;
  poolDetail?: FetchHandler;
  profile?: FetchHandler;
  tags?: FetchHandler;
  notifications?: FetchHandler;
  fallback?: FetchHandler;
}

/**
 * Install a URL-routing `fetch` mock covering every endpoint the core routes
 * touch. Unmatched URLs throw so a test never silently passes on a stray
 * request.
 */
export function createApiFetchMock(handlers: ApiFetchHandlers = {}): jest.Mock {
  const mock = jest.fn(
    async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url;

      if (/\/api\/v1\/pools\/[^/?]+/.test(url)) {
        return (handlers.poolDetail ?? (() => jsonResponse(TEST_POOL_DETAIL)))(
          url,
          init,
        );
      }
      if (/\/api\/v1\/pools(\?|$)/.test(url)) {
        return (handlers.pools ?? (() => jsonResponse(poolsResponse())))(
          url,
          init,
        );
      }
      if (/\/api\/v1\/users\/[^/]+\/profile/.test(url)) {
        return (handlers.profile ?? (() => jsonResponse(profileFixture())))(
          url,
          init,
        );
      }
      if (/\/api\/v1\/tags/.test(url)) {
        return (handlers.tags ?? (() => jsonResponse({ tags: TEST_TAGS })))(
          url,
          init,
        );
      }
      if (/\/api\/v1\/notifications/.test(url)) {
        return (
          handlers.notifications ??
          (() => jsonResponse(notificationsResponse()))
        )(url, init);
      }
      if (handlers.fallback) {
        return handlers.fallback(url, init);
      }
      throw new Error(`Unexpected fetch request in test: ${url}`);
    },
  );

  (global as unknown as { fetch: unknown }).fetch = mock;
  return mock;
}

/** jsdom has no ResizeObserver; charts need one to measure their container. */
export function installResizeObserverMock(): void {
  class ResizeObserverMock {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
  (global as unknown as { ResizeObserver: unknown }).ResizeObserver =
    ResizeObserverMock;
}
