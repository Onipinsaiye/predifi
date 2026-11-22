"use client";

/**
 * useWatchlist — React hook for the wallet-owned market watchlist (#285).
 *
 * Reads from and writes to `lib/watchlist.ts` (cookie/localStorage) scoped by
 * the connected wallet address. The hook re-renders callers only when the list
 * or the loading state changes, not on every wallet interaction.
 *
 * Reconnect and wallet-switch safety:
 * - When `address` changes the hook re-reads from storage, so the new wallet's
 *   list is always shown even after a mid-session switch.
 * - When `address` is `undefined` (disconnected) all mutations are no-ops and
 *   `ids` is an empty array.
 */

import { useState, useCallback, useEffect } from "react";
import {
  getWatchlist,
  addToWatchlist,
  removeFromWatchlist,
  toggleWatchlist,
  isWatched,
  clearWatchlist,
  MAX_WATCHLIST_SIZE,
} from "@/lib/watchlist";

export interface UseWatchlistResult {
  /** Ordered list of watched market IDs for the connected wallet. */
  ids: number[];
  /** True when no wallet is connected. */
  isDisconnected: boolean;
  /** True when the list has reached MAX_WATCHLIST_SIZE. */
  isFull: boolean;
  /** Add a market to the watchlist. Returns false when the list is full. */
  add: (marketId: number) => boolean;
  /** Remove a market from the watchlist. */
  remove: (marketId: number) => void;
  /** Toggle a market in/out of the watchlist. Returns the new watched state. */
  toggle: (marketId: number) => boolean;
  /** Returns true when `marketId` is in the watchlist. */
  watched: (marketId: number) => boolean;
  /** Clear all entries from the watchlist. */
  clear: () => void;
}

export function useWatchlist(address: string | undefined): UseWatchlistResult {
  const [ids, setIds] = useState<number[]>(() =>
    address ? getWatchlist(address) : [],
  );

  // Sync when the connected wallet changes.
  useEffect(() => {
    setIds(address ? getWatchlist(address) : []);
  }, [address]);

  const add = useCallback(
    (marketId: number): boolean => {
      if (!address) return false;
      const ok = addToWatchlist(address, marketId);
      if (ok) setIds(getWatchlist(address));
      return ok;
    },
    [address],
  );

  const remove = useCallback(
    (marketId: number): void => {
      if (!address) return;
      removeFromWatchlist(address, marketId);
      setIds(getWatchlist(address));
    },
    [address],
  );

  const toggle = useCallback(
    (marketId: number): boolean => {
      if (!address) return false;
      const result = toggleWatchlist(address, marketId);
      setIds(getWatchlist(address));
      return result;
    },
    [address],
  );

  const watched = useCallback(
    (marketId: number): boolean => {
      if (!address) return false;
      return isWatched(address, marketId);
    },
    [address],
  );

  const clear = useCallback((): void => {
    if (!address) return;
    clearWatchlist(address);
    setIds([]);
  }, [address]);

  return {
    ids,
    isDisconnected: !address,
    isFull: ids.length >= MAX_WATCHLIST_SIZE,
    add,
    remove,
    toggle,
    watched,
    clear,
  };
}
