"use client";

import { useCallback, useMemo, useState } from "react";
import {
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
  Label,
  ReferenceDot,
  ReferenceLine,
  Cell,
} from "recharts";
import { Clock, RefreshCw } from "lucide-react";
import { cn } from "@/lib/utils";
import { formatUtcDateTime } from "@/lib/date";

/** A single price sample with an explicit source timestamp. */
export interface PriceSample {
  /** Unix timestamp (seconds) when the sample was taken. */
  t: number;
  /** Price value, or null when the sample is missing (gap). */
  price: number | null;
  /** Source that produced the sample (e.g. "indexer", "oracle"). */
  source: string;
}

/** Selectable chart time window in seconds. */
export type PriceWindow = "1h" | "24h" | "7d" | "30d";

const WINDOW_SECONDS: Record<PriceWindow, number> = {
  "1h": 3_600,
  "24h": 86_400,
  "7d": 604_800,
  "30d": 2_592_000,
};

const WINDOW_LABELS: Record<PriceWindow, string> = {
  "1h": "1 hour",
  "24h": "24 hours",
  "7d": "7 days",
  "30d": "30 days",
};

/** Default window when none is provided. */
const DEFAULT_WINDOW: PriceWindow = "24h";

/**
 * Auditable historical price chart.
 *
 * Renders selectable time windows, marks missing-data gaps explicitly, and
 * labels the sampled source and update time so consumers can verify where
 * each point came from rather than assuming continuous indexer coverage.
 */
export interface PriceChartProps {
  /** Raw price samples, unordered and possibly containing gaps. */
  samples: PriceSample[];
  /** Currently selected window. Defaults to "24h". */
  window?: PriceWindow;
  /** Called when the user selects a different window. */
  onWindowChange?: (window: PriceWindow) => void;
  /** Label shown next to the source attribution. */
  sourceLabel?: string;
  /** Accessibility label for the chart. */
  ariaLabel?: string;
  /** Optional height override in CSS units. Defaults to container height. */
  height?: number;
}

function formatWindowTime(seconds: number): string {
  if (seconds >= 86_400) {
    return `${seconds / 86_400} days`;
  }
  if (seconds >= 3_600) {
    return `${seconds / 3_600} hours`;
  }
  return `${seconds / 60} minutes`;
}

/** Filter samples to the selected window relative to the newest sample. */
function samplesInWindow(
  samples: readonly PriceSample[],
  window: PriceWindow,
): PriceSample[] {
  const span = WINDOW_SECONDS[window];
  if (samples.length === 0) return [];
  const newest = Math.max(...samples.map((s) => s.t));
  const cutoff = newest - span;
  return samples.filter((s) => s.t >= cutoff).sort((a, b) => a.t - b.t);
}

/** Detect gaps: consecutive samples more than the expected step apart. */
function gapIntervals(
  samples: readonly PriceSample[],
  window: PriceWindow,
): { start: number; end: number }[] {
  const sorted = [...samples].sort((a, b) => a.t - b.t);
  const span = WINDOW_SECONDS[window];
  // Expected step: 1 minute for 1h/24h, 1 hour for 7d/30d
  const expectedStep = window === "1h" || window === "24h" ? 60 : 3_600;
  const gaps: { start: number; end: number }[] = [];
  for (let i = 1; i < sorted.length; i++) {
    const prev = sorted[i - 1];
    const curr = sorted[i];
    if (prev.price === null || curr.price === null) continue;
    const delta = curr.t - prev.t;
    if (delta > expectedStep * 2) {
      gaps.push({ start: prev.t, end: curr.t });
    }
  }
  return gaps;
}

export function PriceChart({
  samples,
  window: windowProp = DEFAULT_WINDOW,
  onWindowChange,
  sourceLabel = "indexer",
  ariaLabel = "Historical price chart",
  height = 260,
}: PriceChartProps) {
  const [window, setWindow] = useState<PriceWindow>(windowProp);
  const filtered = useMemo(() => samplesInWindow(samples, window), [samples, window]);
  const newest = useMemo(
    () => (filtered.length > 0 ? Math.max(...filtered.map((s) => s.t)) : 0),
    [filtered],
  );
  const latestSample = useMemo(
    () =>
      filtered.length > 0
        ? filtered.reduce((a, b) => (a.t > b.t ? a : b))
        : null,
    [filtered],
  );
  const gaps = useMemo(() => gapIntervals(filtered, window), [filtered, window]);
  const hasData = filtered.some((s) => s.price !== null);

  const handleWindowChange = useCallback(
    (next: PriceWindow) => {
      setWindow(next);
      onWindowChange?.(next);
    },
    [onWindowChange],
  );

  return (
    <section
      className="rounded-2xl border border-white/10 bg-[#121212] p-4"
      aria-label={ariaLabel}
    >
      <div className="flex flex-col gap-3">
        {/* Window selector + source attribution */}
        <div
          className="flex flex-wrap items-center justify-between gap-2 text-xs"
          aria-label="Chart time window and data source"
        >
          <div className="flex items-center gap-2">
            <span className="text-zinc-500">Window</span>
            <div
              role="radiogroup"
              aria-label="Select chart time window"
              className="inline-flex rounded-lg border border-white/10 bg-black/40 p-1"
            >
              {Object.entries(WINDOW_LABELS).map(([key, label]) => {
                const w = key as PriceWindow;
                const active = w === window;
                return (
                  <button
                    key={w}
                    type="button"
                    role="radio"
                    aria-checked={active}
                    onClick={() => handleWindowChange(w)}
                    className={cn(
                      "rounded-md px-2.5 py-1 text-xs transition-colors",
                      active
                        ? "bg-[#37B7C3] text-black font-medium"
                        : "text-zinc-400 hover:text-white hover:bg-white/5",
                    )}
                  >
                    {label}
                  </button>
                );
              })}
            </div>
          </div>
          <div className="flex items-center gap-1.5 text-[10px] text-zinc-500">
            <Clock className="h-3 w-3 shrink-0" aria-hidden="true" />
            <span>
              Source: <span className="text-zinc-300">{sourceLabel}</span>{" "}
              · updated {formatUtcDateTime(newest * 1000)}
            </span>
            <button
              type="button"
              onClick={() => {
                // no-op refresh hook; consumers can wire a re-fetch
              }}
              className="rounded p-1 text-zinc-500 hover:text-white transition-colors"
              aria-label="Refresh chart data"
            >
              <RefreshCw className="h-3.5 w-3.5 shrink-0" />
            </button>
          </div>
        </div>

        {/* Chart */}
        <div className={`h-[${height}px] w-full`}>
          {!hasData ? (
            <div className="flex h-full items-center justify-center text-xs text-zinc-500">
              No price data available for the selected window.
            </div>
          ) : (
            <ResponsiveContainer width="100%" height={height}>
              <LineChart
                data={filtered}
                margin={{ top: 20, right: 30, bottom: 20, left: 10 }}
              >
                <CartesianGrid strokeDasharray="3 3" stroke="white/5" vertical={false} />
                <XAxis
                  dataKey="t"
                  type="number"
                  tickFormatter={(tick) => {
                    const d = new Date(tick * 1000);
                    return d.toLocaleTimeString(undefined, {
                      hour: "2-digit",
                      minute: "2-digit",
                    });
                  }}
                  tick={{ fontSize: 10, fill: "zinc-500" }}
                  axisLine={false}
                  tickLine={false}
                />
                <YAxis
                  tick={{ fontSize: 10, fill: "zinc-500" }}
                  axisLine={false}
                  tickLine={false}
                  width={40}
                />
                <Tooltip
                  content={({ active, payload }: any) => {
                    if (active && payload && payload.length > 0) {
                      const point = payload[0].payload as PriceSample;
                      const time = new Date(point.t * 1000).toLocaleString();
                      return (
                        <div className="bg-[#1a1a1a] border border-white/10 p-2 text-xs">
                          <p className="text-zinc-300">{time}</p>
                          <p className="text-white">
                            {point.price === null
                              ? "No data (gap)"
                              : point.price.toFixed(4)}
                          </p>
                          <p className="text-[10px] text-zinc-500">
                            Source: {point.source}
                          </p>
                        </div>
                      );
                    }
                    return null;
                  }}
                />
                {/* Gap markers: vertical reference lines where data is missing */}
                {gaps.map((gap, i) => (
                  <ReferenceLine
                    key={`gap-${i}`}
                    x={gap.start + (gap.end - gap.start) / 2}
                    stroke="red/40"
                    strokeDasharray="4 4"
                  />
                ))}
                <Line
                  type="monotone"
                  dataKey="price"
                  stroke="#37B7C3"
                  strokeWidth={2}
                  dot={false}
                  isActive={false}
                >
                  {filtered.map((sample, i) => (
                    <Cell
                      key={`cell-${i}`}
                      fill={
                        sample.price === null ? "transparent" : "#37B7C3"
                      }
                    />
                  ))}
                </Line>
                <Label
                  value="Price"
                  position="insideLeft"
                  offset={-8}
                  fill="#37B7C3"
                  textAnchor="start"
                  fontSize={10}
                />
              </LineChart>
            </ResponsiveContainer>
          )}
        </div>

        {/* Gap notice */}
        {gaps.length > 0 && (
          <div
            role="note"
            className="flex items-start gap-1.5 rounded-lg border border-red-500/20 bg-red-500/10 p-2 text-xs text-red-300"
          >
            <span className="shrink-0" aria-hidden="true">
              ⚠
            </span>
            <div>
              <p className="font-medium">Missing data detected</p>
              <p>
                {gaps.length} gap(s) in the selected window — the chart may not
                reflect continuous trading.
              </p>
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
