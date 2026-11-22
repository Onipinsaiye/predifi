"use client";

import { useId } from "react";
import {
  buildChartAlternative,
  type ChartAlternativeInput,
} from "@/lib/chartAccessibility";

interface ChartAlternativeProps extends ChartAlternativeInput {
  className?: string;
}

/**
 * Non-visual alternative to a chart: a short summary that names the chart's
 * units and range, plus a navigable data table with the same values.
 *
 * Render the visual chart inside `aria-hidden` so assistive tech reads this
 * once instead of announcing the chart's SVG and tooltips as well.
 */
export function ChartAlternative({ className, ...input }: ChartAlternativeProps) {
  const id = useId();
  const { summary, rows, valueHeader } = buildChartAlternative(input);

  return (
    <div className={className} data-testid="chart-alternative">
      <p id={`${id}-summary`} className="sr-only">
        {summary}
      </p>
      <details className="text-xs text-zinc-500">
        <summary className="cursor-pointer select-none hover:text-zinc-300">
          View data table
        </summary>
        <table aria-describedby={`${id}-summary`} className="mt-2 w-full text-left">
          <caption className="sr-only">
            {input.title}
            {input.rangeLabel ? `, ${input.rangeLabel}` : ""}
          </caption>
          <thead>
            <tr>
              <th scope="col" className="py-1 pr-4 font-medium">
                {input.showShare ? "Category" : "Period"}
              </th>
              <th scope="col" className="py-1 pr-4 font-medium">
                {valueHeader}
              </th>
              {input.showShare && (
                <th scope="col" className="py-1 font-medium">
                  Share
                </th>
              )}
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.label}>
                <th scope="row" className="py-0.5 pr-4 font-normal text-zinc-400">
                  {row.label}
                </th>
                <td className="py-0.5 pr-4 font-mono text-zinc-300">{row.formatted}</td>
                {input.showShare && (
                  <td className="py-0.5 font-mono text-zinc-300">{row.share}</td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </div>
  );
}
