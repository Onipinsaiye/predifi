/**
 * Wallet-owned market watchlists (#285)
 *
 * A watchlist is a bounded set of canonical market IDs (pool_id numbers) tied
 * to a wallet address. All state lives in the browser (cookies → localStorage
 * fallback) so it is always available offline and synchronises automatically
 * on wallet switch because every read and write passes the wallet address as a
 * scope key.
 *
 * Design decisions:
 * - Identity is the wallet address. Switching wallets surfaces a completely
 *   independent list; the previous wallet's data is never shown or mutated.
 * - `MAX_WATCHLIST_SIZE` enforces a bounded list. Attempts to add a 51st item
 *   are ignored and the caller receives `false`.
 * - Duplicate IDs are silently dropped (idempotent add).
 * - Deleted or unavailable markets are handled by callers (e.g. the UI
 *   component) filtering the full pool list against watchlist IDs; a missing
 *   market in the pool list simply does not render, leaving the rest intact.
 *   The ID itself stays in the list so it reappears if the market becomes
 *   available again (e.g. after a temporary indexer outage).
 */

import {
  getPersistedValue,
  setPersistedValue,
  removePersistedValue,
} from "@/lib/persistentStorage";

export const MAX_WATCHLIST_SIZE = 50;
const WATCHLIST_KEY = "market-watchlist";

/** Build a stable scope string for a connected wallet address. */
function walletScope(address: string): string {
  return `wallet:${address.toLowerCase()}`;
}

/**
 * Read the current watchlist for `address`.
 * Returns an empty array when no data exists or the stored value is invalid.
 */
export function getWatchlist(address: string): number[] {
  const raw = getPersistedValue(WATCHLIST_KEY, walletScope(address));
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (
      Array.isArray(parsed) &&
      parsed.every((item) => typeof item === "number" && Number.isFinite(item) && item > 0)
    ) {
      return parsed as number[];
    }
  } catch {
    // Corrupted data — return empty list and let callers rebuild it.
  }
  return [];
}

/**
 * Persist the watchlist for `address`.
 * Returns `true` when storage succeeds.
 */
function saveWatchlist(address: string, ids: number[]): boolean {
  return setPersistedValue(WATCHLIST_KEY, JSON.stringify(ids), undefined, walletScope(address));
}

/**
 * Add `marketId` to the watchlist owned by `address`.
 *
 * - Duplicate IDs are ignored (idempotent).
 * - Returns `false` and does not mutate when the list is already at capacity.
 * - Returns `true` when the ID was added (or was already present).
 */
export function addToWatchlist(address: string, marketId: number): boolean {
  const list = getWatchlist(address);
  if (list.includes(marketId)) return true;
  if (list.length >= MAX_WATCHLIST_SIZE) return false;
  return saveWatchlist(address, [...list, marketId]);
}

/**
 * Remove `marketId` from the watchlist owned by `address`.
 * Idempotent — removing a missing ID is a no-op.
 */
export function removeFromWatchlist(address: string, marketId: number): void {
  const list = getWatchlist(address);
  if (!list.includes(marketId)) return;
  saveWatchlist(address, list.filter((id) => id !== marketId));
}

/**
 * Toggle `marketId` in the watchlist.
 * Returns `true` when the ID is now watched, `false` when it was removed or
 * when the add was rejected because the list is full.
 */
export function toggleWatchlist(address: string, marketId: number): boolean {
  if (isWatched(address, marketId)) {
    removeFromWatchlist(address, marketId);
    return false;
  }
  return addToWatchlist(address, marketId);
}

/** Returns `true` when `marketId` is in the watchlist for `address`. */
export function isWatched(address: string, marketId: number): boolean {
  return getWatchlist(address).includes(marketId);
}

/**
 * Clear every entry from the watchlist for `address`.
 * Useful when a wallet disconnects or on explicit user action.
 */
export function clearWatchlist(address: string): void {
  removePersistedValue(WATCHLIST_KEY, walletScope(address));
}
