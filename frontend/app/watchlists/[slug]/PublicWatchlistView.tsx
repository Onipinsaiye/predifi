"use client";

/**
 * Public watchlist view (Issue #280).
 *
 * Renders a watchlist shared via its opaque slug. The backend returns only
 * the name, description, and market IDs — never the owner address.
 *
 * Access revocation: when the owner revokes the share link, the backend
 * returns 404 and this component shows a "not available" message.
 */

import useSWR from "swr";
import { fetchPublicWatchlist, publicWatchlistApiUrl } from "@/lib/api/watchlist";

interface PublicWatchlistViewProps {
  slug: string;
}

export function PublicWatchlistView({ slug }: PublicWatchlistViewProps) {
  const { data, isLoading, error } = useSWR(
    publicWatchlistApiUrl(slug),
    () => fetchPublicWatchlist(slug),
    { revalidateOnFocus: false },
  );

  if (isLoading) {
    return (
      <div className="space-y-3 animate-pulse" aria-busy="true" aria-label="Loading watchlist">
        <div className="h-5 w-48 bg-zinc-800 rounded" />
        <div className="h-4 w-64 bg-zinc-800 rounded" />
        <div className="space-y-2 mt-4">
          {Array.from({ length: 4 }).map((_, i) => (
            <div key={i} className="h-10 w-full bg-zinc-900/50 rounded-lg" />
          ))}
        </div>
      </div>
    );
  }

  if (error || data === null) {
    return (
      <div
        role="status"
        className="rounded-xl border border-zinc-800 bg-zinc-900/50 p-8 text-center space-y-3"
      >
        <p className="text-white font-semibold">Watchlist not available</p>
        <p className="text-zinc-500 text-sm">
          This watchlist may have been made private or the link is no longer valid.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {/* Watchlist header */}
      <div>
        <h2 className="text-xl font-semibold text-white">{data.name}</h2>
        {data.description && (
          <p className="text-sm text-zinc-400 mt-1">{data.description}</p>
        )}
        <p className="text-xs text-zinc-600 mt-1">
          {data.market_ids.length} market{data.market_ids.length !== 1 ? "s" : ""}
        </p>
      </div>

      {/* Markets */}
      {data.market_ids.length === 0 ? (
        <p className="text-sm text-zinc-500">This watchlist is empty.</p>
      ) : (
        <ul className="space-y-2" aria-label="Markets in this watchlist">
          {data.market_ids.map((poolId) => (
            <li key={poolId}>
              <a
                href={`/user/pool-market/${poolId}`}
                className="flex items-center justify-between rounded-xl bg-zinc-900/50 hover:bg-zinc-900/80 transition-colors px-4 py-3"
                aria-label={`View market ${poolId}`}
              >
                <span className="text-sm text-zinc-300 hover:text-white">
                  Pool #{poolId}
                </span>
                <span className="text-xs text-zinc-600">→</span>
              </a>
            </li>
          ))}
        </ul>
      )}

      <p className="text-[10px] text-zinc-700 pt-2">
        This watchlist was shared publicly by its creator. Market data is not provided here — click
        a pool to view details on PrediFi.
      </p>
    </div>
  );
}
