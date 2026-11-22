/**
 * Tests for wallet-owned market watchlists (#285)
 *
 * Covers: duplicate IDs, wallet switch, reconnect, and partial unavailability.
 */

import {
  getWatchlist,
  addToWatchlist,
  removeFromWatchlist,
  toggleWatchlist,
  isWatched,
  clearWatchlist,
  MAX_WATCHLIST_SIZE,
} from "@/lib/watchlist";

// ── Helpers ──────────────────────────────────────────────────────────────────

const WALLET_A = "GABC1234567890";
const WALLET_B = "GXYZ9876543210";

function resetStorage() {
  localStorage.clear();
  // Clear cookies set by persistentStorage
  document.cookie.split(";").forEach((c) => {
    document.cookie = c.replace(/^ +/, "").replace(/=.*/, "=;expires=Thu, 01 Jan 1970 00:00:00 UTC;Path=/");
  });
}

beforeEach(resetStorage);

// ── Core operations ───────────────────────────────────────────────────────────

test("empty watchlist returns []", () => {
  expect(getWatchlist(WALLET_A)).toEqual([]);
});

test("add returns true and stores the ID", () => {
  expect(addToWatchlist(WALLET_A, 42)).toBe(true);
  expect(getWatchlist(WALLET_A)).toEqual([42]);
});

test("duplicate add is idempotent and returns true", () => {
  addToWatchlist(WALLET_A, 42);
  expect(addToWatchlist(WALLET_A, 42)).toBe(true);
  expect(getWatchlist(WALLET_A)).toEqual([42]);
});

test("remove deletes the ID", () => {
  addToWatchlist(WALLET_A, 1);
  addToWatchlist(WALLET_A, 2);
  removeFromWatchlist(WALLET_A, 1);
  expect(getWatchlist(WALLET_A)).toEqual([2]);
});

test("remove on missing ID is a no-op", () => {
  addToWatchlist(WALLET_A, 1);
  removeFromWatchlist(WALLET_A, 99);
  expect(getWatchlist(WALLET_A)).toEqual([1]);
});

test("isWatched returns correct boolean", () => {
  addToWatchlist(WALLET_A, 7);
  expect(isWatched(WALLET_A, 7)).toBe(true);
  expect(isWatched(WALLET_A, 8)).toBe(false);
});

test("toggle adds when absent and returns true", () => {
  expect(toggleWatchlist(WALLET_A, 10)).toBe(true);
  expect(isWatched(WALLET_A, 10)).toBe(true);
});

test("toggle removes when present and returns false", () => {
  addToWatchlist(WALLET_A, 10);
  expect(toggleWatchlist(WALLET_A, 10)).toBe(false);
  expect(isWatched(WALLET_A, 10)).toBe(false);
});

test("clear removes all IDs", () => {
  addToWatchlist(WALLET_A, 1);
  addToWatchlist(WALLET_A, 2);
  clearWatchlist(WALLET_A);
  expect(getWatchlist(WALLET_A)).toEqual([]);
});

// ── Capacity bound ────────────────────────────────────────────────────────────

test("add returns false when at capacity", () => {
  for (let i = 1; i <= MAX_WATCHLIST_SIZE; i++) {
    addToWatchlist(WALLET_A, i);
  }
  expect(getWatchlist(WALLET_A)).toHaveLength(MAX_WATCHLIST_SIZE);
  expect(addToWatchlist(WALLET_A, MAX_WATCHLIST_SIZE + 1)).toBe(false);
  expect(getWatchlist(WALLET_A)).toHaveLength(MAX_WATCHLIST_SIZE);
});

test("adding an existing ID when full still returns true (idempotent)", () => {
  for (let i = 1; i <= MAX_WATCHLIST_SIZE; i++) {
    addToWatchlist(WALLET_A, i);
  }
  expect(addToWatchlist(WALLET_A, 1)).toBe(true);
});

// ── Wallet isolation (switch) ─────────────────────────────────────────────────

test("two wallets maintain independent lists", () => {
  addToWatchlist(WALLET_A, 100);
  addToWatchlist(WALLET_B, 200);

  expect(getWatchlist(WALLET_A)).toEqual([100]);
  expect(getWatchlist(WALLET_B)).toEqual([200]);
});

test("clearing one wallet does not affect another", () => {
  addToWatchlist(WALLET_A, 1);
  addToWatchlist(WALLET_B, 2);
  clearWatchlist(WALLET_A);

  expect(getWatchlist(WALLET_A)).toEqual([]);
  expect(getWatchlist(WALLET_B)).toEqual([2]);
});

test("wallet address case is normalised (reconnect with different casing)", () => {
  addToWatchlist("GABC", 5);
  expect(isWatched("GABC", 5)).toBe(true);
  // walletScope lowercases, so both references resolve to the same list
  expect(getWatchlist("gabc")).toEqual([5]);
});

// ── Partial unavailability ────────────────────────────────────────────────────

test("stored IDs for unavailable markets are preserved on read", () => {
  // Simulate three markets saved; one later disappears from the pool list.
  addToWatchlist(WALLET_A, 1);
  addToWatchlist(WALLET_A, 2);
  addToWatchlist(WALLET_A, 3);

  // The unavailable market (ID=2) is still in the raw list so it can reappear.
  const list = getWatchlist(WALLET_A);
  expect(list).toContain(2);

  // After removal of the unavailable ID the remaining two are unaffected.
  removeFromWatchlist(WALLET_A, 2);
  expect(getWatchlist(WALLET_A)).toEqual([1, 3]);
});
