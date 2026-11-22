"use client";

/**
 * WatchlistButton (#285)
 *
 * A toggleable bookmark icon that adds or removes a single market from the
 * wallet-owned watchlist. Renders nothing when no wallet is connected.
 */

import { Bookmark } from "lucide-react";
import { cn } from "@/lib/utils";
import { useWatchlist } from "@/lib/hooks/useWatchlist";
import { useWalletAddress } from "@/lib/context";

interface WatchlistButtonProps {
  marketId: number;
  /** Optional CSS class for the button element. */
  className?: string;
}

export function WatchlistButton({ marketId, className }: WatchlistButtonProps) {
  const address = useWalletAddress();
  const { toggle, watched, isFull } = useWatchlist(address);

  if (!address) return null;

  const active = watched(marketId);
  const disabled = !active && isFull;

  function handleClick(e: React.MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    toggle(marketId);
  }

  return (
    <button
      type="button"
      aria-label={active ? "Remove from watchlist" : "Add to watchlist"}
      aria-pressed={active}
      title={disabled ? "Watchlist is full (50 markets)" : undefined}
      disabled={disabled}
      onClick={handleClick}
      className={cn(
        "flex h-8 w-8 items-center justify-center rounded-full border transition-colors",
        active
          ? "border-[#37B7C3] bg-[#37B7C3]/10 text-[#37B7C3] hover:bg-[#37B7C3]/20"
          : "border-zinc-700 bg-zinc-900 text-zinc-400 hover:border-zinc-500 hover:text-zinc-200",
        disabled && "cursor-not-allowed opacity-40",
        className,
      )}
    >
      <Bookmark
        className="h-4 w-4"
        fill={active ? "currentColor" : "none"}
        aria-hidden="true"
      />
    </button>
  );
}
