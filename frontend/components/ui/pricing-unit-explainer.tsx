"use client";

import { useId, useState } from "react";
import { AlertTriangle, ChevronDown, ChevronUp, HelpCircle, Info } from "lucide-react";
import { cn } from "@/lib/utils";
import {
  PRICING_UNIT_KINDS,
  getPricingUnitExplanation,
  normalizeTokenSymbol,
  type FormattedPricingMetric,
  type PricingUnitKind,
} from "@/lib/probabilityUnits";

export interface PricingMetricValueProps {
  /** Formatted pricing metric produced by `@/lib/probabilityUnits`. */
  metric: FormattedPricingMetric;
  /** Quote token symbol (defaults to `"XLM"`). */
  token?: string;
  /** Optional custom short label (defaults to the canonical unit label). */
  label?: string;
  /** Show the unit explanation panel expanded initially. */
  defaultExpanded?: boolean;
  /** Additional className for the wrapper element. */
  className?: string;
}

/**
 * Renders a displayed probability, share price, odds multiplier, or stake
 * value with its explicit unit and an accessible inline explanation toggle.
 * Surfaces `unavailable` and `invalid` states explicitly with recovery hints.
 */
export function PricingMetricValue({
  metric,
  token,
  label,
  defaultExpanded = false,
  className,
}: PricingMetricValueProps) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const panelId = useId();
  const explanation = getPricingUnitExplanation(metric.kind, token);
  const displayLabel = label ?? explanation.label;

  return (
    <div className={cn("inline-flex flex-col gap-1", className)}>
      <div className="inline-flex flex-wrap items-center gap-1.5">
        <span className="text-xs text-zinc-400">{displayLabel}</span>
        <span
          aria-label={metric.ariaLabel}
          className={cn(
            "font-mono text-sm font-medium",
            metric.status === "ok" && "text-white",
            metric.status === "unavailable" && "text-zinc-400",
            metric.status === "invalid" && "text-amber-300",
          )}
        >
          {metric.display}
        </span>
        {metric.status === "invalid" && (
          <AlertTriangle
            className="h-3.5 w-3.5 text-amber-400 shrink-0"
            aria-hidden="true"
          />
        )}
        <button
          type="button"
          aria-expanded={expanded}
          aria-controls={expanded ? panelId : undefined}
          aria-label={`Explain ${displayLabel} (${metric.unitLabel})`}
          onClick={() => setExpanded((prev) => !prev)}
          className="inline-flex items-center gap-0.5 rounded px-1 py-0.5 text-[11px] text-[#7DE3EC] hover:bg-white/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#7DE3EC]"
        >
          <HelpCircle className="h-3 w-3 shrink-0" aria-hidden="true" />
          <span>Unit</span>
        </button>
      </div>

      {expanded && (
        <div
          id={panelId}
          className="rounded-lg border border-white/10 bg-[#141414] p-3 text-xs text-zinc-300 space-y-1.5"
        >
          <p className="font-medium text-white">
            {explanation.label} ({metric.unitLabel})
          </p>
          <p>{explanation.summary}</p>
          <p className="font-mono text-[11px] text-[#7DE3EC]">
            Formula: {explanation.formula}
          </p>
          <p className="text-[11px] text-zinc-400">Example: {explanation.example}</p>
          <p className="text-[11px] text-zinc-500">{explanation.authorityNote}</p>

          {metric.status !== "ok" && metric.reason && (
            <div
              role={metric.status === "invalid" ? "alert" : "status"}
              className={cn(
                "rounded-md border px-2.5 py-2 text-[11px] space-y-0.5",
                metric.status === "invalid"
                  ? "border-amber-500/40 bg-amber-500/10 text-amber-200"
                  : "border-zinc-700 bg-zinc-800/60 text-zinc-300",
              )}
            >
              <p className="font-medium">{metric.reason}</p>
              {metric.recoveryHint && <p>{metric.recoveryHint}</p>}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export interface PricingUnitsLegendProps {
  /** Quote token symbol shown in examples and price-unit copy (defaults to `"XLM"`). */
  token?: string;
  /** Optional subset of pricing units to explain. Defaults to all four units. */
  units?: readonly PricingUnitKind[];
  /** Whether the legend starts expanded. Defaults to `false` to keep market views compact. */
  defaultOpen?: boolean;
  /** Additional className for the container. */
  className?: string;
}

/**
 * Reusable, accessible legend explaining how displayed probabilities, unit
 * share prices, payout odds, and stake amounts relate on a market view.
 */
export function PricingUnitsLegend({
  token,
  units = PRICING_UNIT_KINDS,
  defaultOpen = false,
  className,
}: PricingUnitsLegendProps) {
  const [open, setOpen] = useState(defaultOpen);
  const contentId = useId();
  const symbol = normalizeTokenSymbol(token);
  const explanations = units.map((kind) => getPricingUnitExplanation(kind, symbol));

  return (
    <aside
      aria-label="Probability and price units guide"
      className={cn(
        "rounded-lg border border-zinc-800 bg-zinc-900/70 px-3.5 py-2.5 text-xs text-zinc-300",
        className,
      )}
    >
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-zinc-300">
          <Info className="h-3.5 w-3.5 text-[#7DE3EC] shrink-0" aria-hidden="true" />
          <span>
            Displayed probabilities (%) and unit prices ({symbol}/share) are derived from current
            pool stakes.
          </span>
        </div>
        <button
          type="button"
          aria-expanded={open}
          aria-controls={open ? contentId : undefined}
          onClick={() => setOpen((prev) => !prev)}
          className="inline-flex items-center gap-1 shrink-0 rounded px-2 py-1 text-xs font-medium text-[#7DE3EC] hover:bg-white/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#7DE3EC]"
        >
          <span>{open ? "Hide unit guide" : "How units work"}</span>
          {open ? (
            <ChevronUp className="h-3.5 w-3.5" aria-hidden="true" />
          ) : (
            <ChevronDown className="h-3.5 w-3.5" aria-hidden="true" />
          )}
        </button>
      </div>

      {open && (
        <div id={contentId} className="mt-3 border-t border-zinc-800 pt-3 space-y-3">
          <dl className="grid gap-2.5 sm:grid-cols-2">
            {explanations.map((entry) => (
              <div
                key={entry.kind}
                className="rounded-md border border-zinc-800 bg-[#0F0F0F] p-2.5 space-y-1"
              >
                <dt className="flex items-center justify-between gap-2 font-medium text-white">
                  <span>{entry.label}</span>
                  <span className="rounded bg-white/5 px-1.5 py-0.5 font-mono text-[10px] text-[#7DE3EC]">
                    {entry.unitSymbol}
                  </span>
                </dt>
                <dd className="space-y-1 text-[11px] text-zinc-400">
                  <p className="text-zinc-300">{entry.summary}</p>
                  <p className="font-mono text-[10px] text-zinc-400">
                    Formula: {entry.formula}
                  </p>
                  <p className="text-[10px] text-zinc-500">{entry.example}</p>
                </dd>
              </div>
            ))}
          </dl>
          <p className="text-[11px] text-zinc-400">
            All probabilities, unit prices, and odds shown in the UI are illustrative snapshots
            before protocol fees. Final payouts are calculated authoritatively on-chain in{" "}
            {symbol} base units when the market resolves.
          </p>
        </div>
      )}
    </aside>
  );
}
