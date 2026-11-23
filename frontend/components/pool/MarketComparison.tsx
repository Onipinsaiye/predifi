"use client";

import { useCallback, useMemo, useState } from "react";
import { X, Maximize2, Minimize2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { formatUtcDateTime } from "@/lib/date";

/** Maximum number of markets allowed in a comparison set. */
export const MAX_COMPARISON_MARKETS = 4;

/** A single market entry that can be compared side by side. */
export interface ComparisonMarket {
  /** Market / pool identifier. */
  id: string | number;
  /** Human-readable market question. */
  question: string;
  /** Outcome options, e.g. ["Yes", "No"]. */
  outcomes: string[];
  /** Close time as Unix timestamp (seconds). */
  closeTime: number;
  /** Data source label (e.g. "indexer", "chain"). */
  source: string;
  /** When the indexer last mirrored this market, ISO timestamp. */
  indexedAt?: string | null;
  /** Latest indexed value per outcome, keyed by outcome index. */
  latestValues?: Record<number, number>;
  /** URL to the market detail page. */
  url?: string;
}

/** Comparison set state. */
export interface ComparisonSet {
  ids: (string | number)[];
  add: (market: ComparisonMarket) => void;
  remove: (id: string | number) => void;
  toggle: (market: ComparisonMarket) => void;
  clear: () => void;
  isFull: boolean;
  contains: (id: string | number) => boolean;
}

export interface MarketComparisonProps {
  /** Markets currently available for selection. */
  markets: ComparisonMarket[];
  /** Currently selected comparison set. */
  selected: ComparisonSet;
  /** Accessibility label. */
  ariaLabel?: string;
  /** Whether the comparison panel is open. Defaults to true. */
  open?: boolean;
  /** Called when the panel should close. */
  onClose?: () => void;
}

/** Format a close time for display alongside a countdown. */
function formatCloseTime(closeTime: number): string {
  return formatUtcDateTime(closeTime * 1000);
}

/** Determine if a market is stale (not recently indexed). */
function isStale(indexedAt?: string | null): boolean {
  if (!indexedAt) return true;
  const indexedMs = new Date(indexedAt).getTime();
  if (Number.isNaN(indexedMs)) return true;
  const oneHourAgo = Date.now() - 60 * 60 * 1000;
  return indexedMs < oneHourAgo;
}

export function MarketComparison({
  markets,
  selected,
  ariaLabel = "Compare selected markets side by side",
  open = true,
  onClose,
}: MarketComparisonProps) {
  const [expanded, setExpanded] = useState(false);
  const [sortBy, setSortBy] = useState<"closeTime" | "question">("closeTime");

  const marketsById = useMemo(() => {
    const map = new Map<string | number, ComparisonMarket>();
    for (const m of markets) map.set(m.id, m);
    return map;
  }, [markets]);

  const comparison = useMemo(() => {
    return selected.ids
      .map((id) => marketsById.get(id))
      .filter((m): m is ComparisonMarket => m !== undefined)
      .sort((a, b) => {
        if (sortBy === "closeTime") return a.closeTime - b.closeTime;
        return a.question.localeCompare(b.question);
      });
  }, [selected.ids, marketsById, sortBy]);

  const canAdd = useMemo(
    () => markets.length > comparison.length && !selected.isFull,
    [markets.length, comparison.length, selected.isFull],
  );

  const handleAdd = useCallback(
    (market: ComparisonMarket) => {
      selected.add(market);
    },
    [selected],
  );

  const handleRemove = useCallback(
    (id: string | number) => {
      selected.remove(id);
    },
    [selected],
  );

  const handleToggle = useCallback(
    (market: ComparisonMarket) => {
      selected.toggle(market);
    },
    [selected],
  );

  if (!open) return null;

  return (
    <section
      className={cn(
        "fixed inset-0 z-50 flex flex-col",
        expanded ? "w-full h-full" : "w-[420px] h-full",
      )}
      aria-label={ariaLabel}
    >
      {/* Header */}
      <div className="flex items-center justify-between gap-2 px-4 py-3 border-b border-white/10 bg-[#0a0a0a]/80 backdrop-blur z-10">
        <h2 className="text-sm font-semibold text-white">
          {comparison.length === 0 ? "Compare markets" : "Market comparison"}
        </h2>
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={() => setExpanded((e) => !e)}
            className="rounded p-1.5 text-zinc-400 hover:text-white transition-colors"
            aria-label={expanded ? "Collapse comparison panel" : "Expand comparison panel"}
          >
            {expanded ? (
              <Minimize2 className="h-4 w-4" />
            ) : (
              <Maximize2 className="h-4 w-4" />
            )}
          </button>
          <button
            type="button"
            onClick={onClose}
            className="rounded p-1.5 text-zinc-400 hover:text-red-400 transition-colors"
            aria-label="Close comparison panel"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      </div>

      {/* Body */}
      <div
        className={cn(
          "flex-1 overflow-y-auto p-4",
          expanded ? "max-h-full" : "max-h-[60vh]",
        )}
      >
        {/* Empty state */}
        {comparison.length === 0 && (
          <div className="flex flex-col items-center justify-center h-full text-center">
            <p className="text-zinc-500 text-sm">
              No markets selected. Add up to {MAX_COMPARISON_MARKETS} markets
              to compare side by side.
            </p>
          </div>
        )}

        {/* Comparison table (wide screens) */}
        {comparison.length > 0 && (
          <>
            <div className="mb-3 flex items-center justify-between">
              <span className="text-[10px] uppercase tracking-wider text-zinc-500">
                {comparison.length} market{comparison.length !== 1 ? "s" : ""}
                {selected.isFull ? " — limit reached" : ""}
              </span>
              <div className="flex items-center gap-1">
                <span className="text-xs text-zinc-500">Sort by</span>
                <button
                  type="button"
                  onClick={() => setSortBy("closeTime")}
                  className={cn(
                    "text-xs px-2 py-0.5 rounded transition-colors",
                    sortBy === "closeTime"
                      ? "bg-[#37B7C3]/10 text-[#37B7C3]"
                      : "text-zinc-400 hover:bg-white/5",
                  )}
                >
                  Close time
                </button>
                <button
                  type="button"
                  onClick={() => setSortBy("question")}
                  className={cn(
                    "text-xs px-2 py-0.5 rounded transition-colors",
                    sortBy === "question"
                      ? "bg-[#37B7C3]/10 text-[#37B7C3]"
                      : "text-zinc-400 hover:bg-white/5",
                  )}
                >
                  Question
                </button>
              </div>
            </div>

            {/* Non-table narrow-screen layout (cards) */}
            <div className="space-y-3" role="region" aria-label="Market comparison list">
              {comparison.map((market) => (
                <ComparisonCard
                  key={String(market.id)}
                  market={market}
                  isSelected={selected.contains(market.id)}
                  onRemove={() => handleRemove(market.id)}
                  onToggle={() => handleToggle(market)}
                  canRemove={comparison.length > 1 || !selected.contains(market.id)}
                />
              ))}
            </div>

            {/* Responsive table for wide screens */}
            <div className="hidden lg:block mt-4">
              <ComparisonTable
                markets={comparison}
                selected={selected}
                onRemove={handleRemove}
                onToggle={handleToggle}
              />
            </div>
          </>
        )}
      </div>

      {/* Footer: available markets to add */}
      {comparison.length > 0 && canAdd && (
        <div className="border-t border-white/10 p-3 bg-[#0a0a0a]/40">
          <p className="text-[10px] text-zinc-500 mb-2">
            Add more markets ({comparison.length}/{MAX_COMPARISON_MARKETS})
          </p>
          <div className="flex flex-wrap gap-2">
            {markets
              .filter((m) => !selected.contains(m.id))
              .slice(0, 8)
              .map((market) => (
                <button
                  key={String(market.id)}
                  type="button"
                  onClick={() => handleAdd(market)}
                  disabled={!canAdd}
                  className={cn(
                    "flex items-center gap-2 rounded-lg border border-white/10 bg-black/40 px-2 py-1.5 text-xs transition-colors",
                    !canAdd
                      ? "opacity-50 cursor-not-allowed"
                      : "text-zinc-300 hover:text-white hover:bg-white/5 disabled:cursor-not-allowed",
                  )}
                >
                  {market.question.length > 28
                    ? `${market.question.slice(0, 28)}…`
                    : market.question}
                </button>
              ))}
          </div>
        </div>
      )}
    </section>
  );
}

/** Single market card for narrow-screen layout. */
function ComparisonCard({
  market,
  isSelected,
  onRemove,
  onToggle,
  canRemove = true,
}: {
  market: ComparisonMarket;
  isSelected: boolean;
  onRemove: () => void;
  onToggle: () => void;
  canRemove?: boolean;
}) {
  const stale = isStale(market.indexedAt);
  const closeLabel = formatCloseTime(market.closeTime);

  return (
    <div
      className={cn(
        "rounded-xl border border-white/10 bg-[#121212] p-3",
        isSelected ? "border-[#37B7C3]/50 bg-[#121212]" : "",
      )}
    >
      <div className="flex items-start justify-between gap-2 mb-1">
        <button
          type="button"
          onClick={onToggle}
          className="flex items-center gap-1.5 text-sm font-medium text-white hover:text-[#37B7C3] truncate"
          aria-pressed={isSelected}
        >
          {isSelected ? "✓ " : ""}
          {market.question}
        </button>
        {canRemove && (
          <button
            type="button"
            onClick={onRemove}
            className="rounded p-1 text-zinc-500 hover:text-red-400 transition-colors"
            aria-label={`Remove ${market.question} from comparison`}
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
      </div>
      <dl className="space-y-1 text-xs">
        <div className="flex items-center gap-2">
          <dt className="w-16 text-zinc-500">Outcomes</dt>
          <dd className="flex flex-wrap gap-1 text-zinc-300">
            {market.outcomes.map((o) => (
              <span
                key={o}
                className="rounded bg-white/5 px-1.5 py-0.5"
              >
                {o}
              </span>
            ))}
          </dd>
        </div>
        <div className="flex items-center gap-2">
          <dt className="w-16 text-zinc-500">Closes</dt>
          <dd className="text-zinc-300">{closeLabel}</dd>
        </div>
        <div className="flex items-center gap-2">
          <dt className="w-16 text-zinc-500">Source</dt>
          <dd className="text-zinc-300 capitalize">{market.source}</dd>
        </div>
        <div className="flex items-center gap-2">
          <dt className="w-16 text-zinc-500">Indexed</dt>
          <dd className={cn("text-zinc-300", stale && "text-red-400")}>
            {stale ? "Stale" : market.indexedAt ?? "—"}
          </dd>
        </div>
        {market.latestValues && (
          <div className="flex items-center gap-2">
            <dt className="w-16 text-zinc-500">Latest values</dt>
            <dd className="flex flex-wrap gap-1 text-zinc-300">
              {Object.entries(market.latestValues).map(([outcome, value]) => (
                <span key={outcome} className="text-[#37B7C3]">
                  {outcome}: {value.toFixed(4)}
                </span>
              ))}
            </dd>
          </div>
        )}
      </dl>
    </div>
  );
}

/** Side-by-side table for wide screens. */
function ComparisonTable({
  markets,
  selected,
  onRemove,
  onToggle,
}: {
  markets: ComparisonMarket[];
  selected: ComparisonSet;
  onRemove: (id: string | number) => void;
  onToggle: (market: ComparisonMarket) => void;
}) {
  const allOutcomes = useMemo(() => {
    const set = new Set<number>();
    for (const m of markets) {
      for (let i = 0; i < m.outcomes.length; i++) set.add(i);
    }
    return [...set];
  }, [markets]);

  return (
    <div className="rounded-xl border border-white/10 bg-[#121212] overflow-hidden">
      <table className="w-full text-left border-collapse text-xs">
        <thead>
          <tr className="border-b border-white/10 bg-[#0a0a0a]/50">
            <th className="px-3 py-2 text-zinc-500 font-medium">Market</th>
            {allOutcomes.map((o) => (
              <th
                key={o}
                className="px-3 py-2 text-zinc-500 font-medium text-center"
              >
                Outcome {o}
              </th>
            ))}
            <th className="px-3 py-2 text-zinc-500 font-medium text-center">
              Closes
            </th>
            <th className="px-3 py-2 text-zinc-500 font-medium text-center">
              Source
            </th>
            <th className="px-3 py-2 text-zinc-500 font-medium text-center">
              Indexed
            </th>
          </tr>
        </thead>
        <tbody className="divide-y divide-white/5">
          {markets.map((market) => {
            const stale = isStale(market.indexedAt);
            const latest = market.latestValues ?? {};
            return (
              <tr key={String(market.id)} className="hover:bg-white/3 transition-colors">
                <td className="px-3 py-2">
                  <button
                    type="button"
                    onClick={() => onToggle(market)}
                    className="flex items-center gap-1.5 text-sm font-medium text-white truncate hover:text-[#37B7C3]"
                    aria-pressed={selected.contains(market.id)}
                  >
                    {selected.contains(market.id) ? "✓ " : ""}
                    {market.question}
                  </button>
                  {selected.contains(market.id) && (
                    <button
                      type="button"
                      onClick={() => onRemove(market.id)}
                      className="ml-1.5 rounded p-1 text-zinc-500 hover:text-red-400 transition-colors"
                      aria-label={`Remove ${market.question} from comparison`}
                    >
                      <X className="h-3 w-3" />
                    </button>
                  )}
                </td>
                {allOutcomes.map((o) => (
                  <td
                    key={o}
                    className="px-3 py-2 text-center"
                  >
                    {latest[o] !== undefined ? (
                      <span className={cn(
                        "font-mono text-[#37B7C3]",
                        latest[o] === 0 ? "text-zinc-500" : "",
                      )}>
                        {latest[o].toFixed(4)}
                      </span>
                    ) : (
                      <span className="text-zinc-500">—</span>
                    )}
                  </td>
                ))}
                <td className="px-3 py-2 text-center text-zinc-300">
                  {formatCloseTime(market.closeTime)}
                </td>
                <td className="px-3 py-2 text-center text-zinc-300 capitalize">
                  {market.source}
                </td>
                <td className="px-3 py-2 text-center">
                  <span className={cn("text-xs", stale && "text-red-400")}>
                    {stale ? "Stale" : market.indexedAt ?? "—"}
                  </span>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
