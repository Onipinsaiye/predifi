"use client";

import { Clock, AlertTriangle, HelpCircle } from "lucide-react";
import { cn } from "@/lib/utils";
import { getFreshness, formatAge, DEFAULT_STALE_THRESHOLD_MS } from "@/lib/freshness";

export interface FreshnessIndicatorProps {
  /** ISO timestamp (or epoch ms) the indexer last reported this data at. */
  indexedAt?: string | number | null;
  staleThresholdMs?: number;
  className?: string;
}

/**
 * Unobtrusive indicator of how fresh indexer-mirrored market data is.
 *
 * - `fresh`: data is within the staleness bound — rendered quietly so it
 *   doesn't compete with primary content.
 * - `stale`: indexing has lagged past the bound — called out so users don't
 *   mistake a behind-ledger value for current.
 * - `unavailable`: no freshness timestamp was reported at all (indexer
 *   unreachable, or not yet wired up) — shown distinctly from "stale" since
 *   claiming a specific lag here would be a guess.
 */
export function FreshnessIndicator({
  indexedAt,
  staleThresholdMs = DEFAULT_STALE_THRESHOLD_MS,
  className,
}: FreshnessIndicatorProps) {
  const { state, ageMs } = getFreshness(indexedAt, Date.now(), staleThresholdMs);

  if (state === "unavailable") {
    return (
      <span
        className={cn(
          "inline-flex items-center gap-1 text-[10px] text-zinc-500",
          className,
        )}
        title="Indexer freshness is unavailable right now"
      >
        <HelpCircle className="h-3 w-3" />
        Freshness unknown
      </span>
    );
  }

  if (state === "stale") {
    return (
      <span
        role="status"
        className={cn(
          "inline-flex items-center gap-1 rounded-full bg-amber-500/10 px-2 py-0.5 text-[10px] font-medium text-amber-400",
          className,
        )}
        title={`Last indexed ${ageMs != null ? formatAge(ageMs) : "a while ago"} — this data may be behind the latest ledger`}
      >
        <AlertTriangle className="h-3 w-3" />
        Behind ledger{ageMs != null ? ` · ${formatAge(ageMs)}` : ""}
      </span>
    );
  }

  return (
    <span
      className={cn("inline-flex items-center gap-1 text-[10px] text-zinc-500", className)}
      title={`Last indexed ${ageMs != null ? formatAge(ageMs) : "just now"}`}
    >
      <Clock className="h-3 w-3" />
      Updated {ageMs != null ? formatAge(ageMs) : "just now"}
    </span>
  );
}
