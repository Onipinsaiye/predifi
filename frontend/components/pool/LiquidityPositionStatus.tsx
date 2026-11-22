"use client";

/**
 * Liquidity-provider position status (issue #273).
 *
 * Renders the LP-specific fields — deposited value, share fraction, accrued
 * fees, lifecycle lock and last indexed checkpoint — and gates withdrawal
 * actions on verifiable eligibility.  Every reason for a disabled action is
 * rendered as visible text inside a polite live region so it is reachable by
 * assistive technology, not hidden in a tooltip.
 */

import { useId } from "react";
import { formatStake } from "@/lib/stakeFilters";
import { FreshnessIndicator } from "@/components/common/FreshnessIndicator";
import {
  deriveLiquidityPositionStatus,
  type LiquidityPosition,
  type LiquidityPositionStatus,
} from "@/lib/liquidity/position";
import { cn } from "@/lib/utils";

export interface LiquidityPositionStatusProps {
  position: LiquidityPosition;
  /** Token symbol used when formatting base-unit amounts. */
  token?: string;
  /** Override for the staleness threshold, primarily for tests. */
  staleThresholdMs?: number;
  /** Current time override, primarily for tests. */
  now?: number;
  onWithdraw?: () => void;
  className?: string;
}

function checkpointLabel(value: string | number | null | undefined): string {
  if (value == null) return "Unavailable";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toISOString();
}

export function LiquidityPositionStatus({
  position,
  token = "XLM",
  staleThresholdMs,
  now,
  onWithdraw,
  className,
}: LiquidityPositionStatusProps) {
  const explanationId = useId();
  const status: LiquidityPositionStatus = deriveLiquidityPositionStatus(
    position,
    now ?? Date.now(),
    staleThresholdMs,
  );

  const rows: Array<{ label: string; value: string }> = [
    {
      label: "Deposited value",
      value: formatStake(position.depositedValue.toString(), token),
    },
    { label: "Share of pool", value: status.shareFraction },
    {
      label: "Accrued fees",
      value: formatStake(position.accruedFees.toString(), token),
    },
    {
      label: "Lifecycle",
      value: status.canWithdraw
        ? "Withdrawals open"
        : `Locked (${status.lifecycleState})`,
    },
  ];

  return (
    <section
      aria-label="Liquidity position status"
      data-eligibility={status.eligibility}
      className={cn("rounded-lg border border-zinc-800 p-4 text-sm", className)}
    >
      <div className="mb-3 flex items-center justify-between gap-2">
        <h2 className="font-medium text-zinc-100">Liquidity position</h2>
        <FreshnessIndicator
          indexedAt={position.lastIndexedAt}
          staleThresholdMs={staleThresholdMs}
        />
      </div>

      <dl className="grid grid-cols-2 gap-x-4 gap-y-2">
        {rows.map((row) => (
          <div key={row.label}>
            <dt className="text-xs text-zinc-500">{row.label}</dt>
            <dd className="text-zinc-100">{row.value}</dd>
          </div>
        ))}
        <div>
          <dt className="text-xs text-zinc-500">Last indexed checkpoint</dt>
          <dd className="text-zinc-100">
            {position.lastIndexedAt == null ? (
              "Unavailable"
            ) : (
              <time dateTime={checkpointLabel(position.lastIndexedAt)}>
                {checkpointLabel(position.lastIndexedAt)}
              </time>
            )}
          </dd>
        </div>
      </dl>

      <div
        role="status"
        aria-live="polite"
        aria-label="Withdrawal eligibility"
        className="mt-4"
      >
        <ul id={explanationId} className="space-y-1 text-xs text-zinc-400">
          {status.reasons.map((reason) => (
            <li key={reason}>{reason}</li>
          ))}
        </ul>
      </div>

      <button
        type="button"
        disabled={!status.canWithdraw}
        aria-disabled={!status.canWithdraw}
        aria-describedby={explanationId}
        onClick={status.canWithdraw ? onWithdraw : undefined}
        className={cn(
          "mt-4 rounded-md px-3 py-1.5 text-sm font-medium",
          status.canWithdraw
            ? "bg-emerald-600 text-white hover:bg-emerald-500"
            : "cursor-not-allowed bg-zinc-800 text-zinc-500",
        )}
      >
        Withdraw liquidity
      </button>
    </section>
  );
}

export default LiquidityPositionStatus;
