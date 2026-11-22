"use client";

/**
 * WatchlistPanel (#285)
 *
 * Shows all markets in the connected wallet's watchlist. Markets that are no
 * longer in the pool list (deleted/unavailable) are silently skipped so the
 * remaining list is unaffected. The panel is empty-state aware and surfaces
 * the fill limit to the user.
 */

import { Bookmark, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useWatchlist } from "@/lib/hooks/useWatchlist";
import { useWalletAddress } from "@/lib/context";
import { MAX_WATCHLIST_SIZE } from "@/lib/watchlist";
import type { Pool } from "@/lib/api/pools";

interface WatchlistPanelProps {
  /** All known pools — the panel filters these down to watchlisted ones. */
  pools: Pool[];
  className?: string;
}

export function WatchlistPanel({ pools, className }: WatchlistPanelProps) {
  const address = useWalletAddress();
  const { ids, remove, clear } = useWatchlist(address);

  if (!address) {
    return (
      <div className={cn("rounded-xl border border-zinc-800 bg-zinc-900 p-6 text-center", className)}>
        <Bookmark className="mx-auto mb-3 h-8 w-8 text-zinc-600" aria-hidden="true" />
        <p className="text-sm text-zinc-400">Connect a wallet to use watchlists.</p>
      </div>
    );
  }

  // Keep only markets present in `pools` to handle deleted/unavailable entries
  // without corrupting the stored list.
  const poolMap = new Map(pools.map((p) => [p.pool_id, p]));
  const watchedPools = ids.map((id) => poolMap.get(id)).filter(Boolean) as Pool[];

  return (
    <div className={cn("rounded-xl border border-zinc-800 bg-zinc-900", className)}>
      <div className="flex items-center justify-between px-4 py-3 border-b border-zinc-800">
        <div className="flex items-center gap-2">
          <Bookmark className="h-4 w-4 text-[#37B7C3]" aria-hidden="true" />
          <p className="text-sm font-semibold text-white">
            Watchlist
          </p>
          <span className="text-xs text-zinc-500">
            {ids.length}/{MAX_WATCHLIST_SIZE}
          </span>
        </div>
        {ids.length > 0 && (
          <button
            type="button"
            onClick={clear}
            className="text-xs text-zinc-500 hover:text-red-400 transition-colors"
          >
            Clear all
          </button>
        )}
      </div>

      {ids.length === 0 ? (
        <div className="px-4 py-8 text-center">
          <Bookmark className="mx-auto mb-3 h-8 w-8 text-zinc-700" aria-hidden="true" />
          <p className="text-sm text-zinc-500">No markets in your watchlist yet.</p>
          <p className="text-xs text-zinc-600 mt-1">
            Click the bookmark icon on any market to add it here.
          </p>
        </div>
      ) : (
        <ul>
          {ids.map((id) => {
            const pool = poolMap.get(id);
            return (
              <li
                key={id}
                className="flex items-center justify-between gap-3 px-4 py-3 border-b border-zinc-800/50 last:border-0 hover:bg-white/[0.02] transition-colors"
              >
                {pool ? (
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium text-white truncate">{pool.name}</p>
                    <p className="text-xs text-zinc-500 mt-0.5">
                      <span className="capitalize">{pool.state}</span>
                      {pool.category ? ` · ${pool.category}` : ""}
                    </p>
                  </div>
                ) : (
                  <div className="min-w-0 flex-1">
                    <p className="text-sm text-zinc-500 italic">Market #{id} unavailable</p>
                  </div>
                )}
                <button
                  type="button"
                  onClick={() => remove(id)}
                  aria-label={`Remove market ${pool?.name ?? id} from watchlist`}
                  className="flex-shrink-0 rounded-full p-1 text-zinc-600 hover:text-red-400 transition-colors"
                >
                  <X className="h-3.5 w-3.5" aria-hidden="true" />
                </button>
              </li>
            );
          })}
        </ul>
      )}

      {ids.length >= MAX_WATCHLIST_SIZE && (
        <p className="px-4 py-2 text-center text-xs text-amber-400 border-t border-zinc-800">
          Watchlist is full. Remove a market to add another.
        </p>
      )}
    </div>
  );
}
