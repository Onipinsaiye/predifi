/**
 * Builds the non-visual alternative for a chart: one concise summary sentence
 * plus the exact rows that were plotted. Both come from the same input the
 * chart renders, so the two can never drift apart.
 */

import { formatChartValue } from "@/lib/stakeFilters";

export interface ChartDatum {
  label: string;
  value: number;
}

export interface ChartAlternativeInput {
  title: string;
  /** Unit of the plotted values, e.g. "USD" or "predictions". */
  unit: string;
  /** Human-readable span covered, e.g. "January to December". */
  rangeLabel?: string;
  data: ChartDatum[];
  /** Defaults to compact USD (the chart's tooltip format). */
  formatValue?: (value: number) => string;
  /** Include each row's share of the total (for distribution charts). */
  showShare?: boolean;
}

export interface ChartAlternativeRow {
  label: string;
  value: number;
  formatted: string;
  share?: string;
}

export interface ChartAlternative {
  summary: string;
  rows: ChartAlternativeRow[];
  valueHeader: string;
}

export function buildChartAlternative(input: ChartAlternativeInput): ChartAlternative {
  const format = input.formatValue ?? formatChartValue;
  const total = input.data.reduce((sum, d) => sum + d.value, 0);
  const valueHeader = `${input.title} (${input.unit})`;

  const rows: ChartAlternativeRow[] = input.data.map((d) => ({
    label: d.label,
    value: d.value,
    formatted: format(d.value),
    ...(input.showShare
      ? { share: total > 0 ? `${Math.round((d.value / total) * 100)}%` : "0%" }
      : {}),
  }));

  const range = input.rangeLabel ? `, ${input.rangeLabel}` : "";
  if (input.data.length === 0) {
    return {
      summary: `${input.title}${range}: no data available.`,
      rows,
      valueHeader,
    };
  }

  let summary: string;
  if (input.showShare) {
    const parts = rows.map((r) => `${r.label} ${r.formatted} (${r.share})`);
    summary = `${input.title}${range}, in ${input.unit}: ${parts.join(", ")}. Total ${format(total)}.`;
  } else {
    const max = input.data.reduce((a, b) => (b.value > a.value ? b : a));
    const min = input.data.reduce((a, b) => (b.value < a.value ? b : a));
    summary =
      `${input.title}${range}, in ${input.unit}: ${input.data.length} data points. ` +
      `Highest ${max.label} at ${format(max.value)}; lowest ${min.label} at ${format(min.value)}; ` +
      `total ${format(total)}.`;
  }

  return { summary, rows, valueHeader };
}
