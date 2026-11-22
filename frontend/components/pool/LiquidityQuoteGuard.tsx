"use client";

/**
 * Quote-bound guard for liquidity actions (issue #274).
 *
 * The user must choose a maximum price impact (or a minimum output) before a
 * deposit/withdrawal can be simulated or submitted.  The same verified bound
 * object is handed to both paths so the value signed by the user is exactly
 * the value enforced on-chain.  Any material change — network, pool snapshot
 * or amount — invalidates the quote and forces a refresh.
 */

import { useId, useMemo, useState } from "react";
import {
  buildQuoteBound,
  computeMinOutput,
  simulateWithBound,
  toBoundCallArguments,
  type BoundCallArguments,
  type BoundCheckResult,
  type LiquidityQuote,
  type QuoteBound,
} from "@/lib/liquidity/quoteBound";
import { formatStake } from "@/lib/stakeFilters";
import { cn } from "@/lib/utils";

export interface LiquidityQuoteContext {
  poolId: string;
  amountIn: bigint;
  networkId: string;
  stateCheckpoint: string;
}

export interface LiquidityQuoteGuardProps {
  quote: LiquidityQuote;
  context: LiquidityQuoteContext;
  now?: number;
  maxAgeMs?: number;
  onRefreshQuote?: () => void;
  onSimulate?: (args: BoundCallArguments) => void;
  onSubmit?: (args: BoundCallArguments) => void;
  className?: string;
}

const PRESETS = [10, 50, 100, 300] as const;

export function LiquidityQuoteGuard({
  quote,
  context,
  now,
  maxAgeMs,
  onRefreshQuote,
  onSimulate,
  onSubmit,
  className,
}: LiquidityQuoteGuardProps) {
  const inputId = useId();
  const messageId = useId();
  const [maxSlippageInput, setMaxSlippageInput] = useState<string>("");

  const parsed = maxSlippageInput === "" ? null : Number(maxSlippageInput);
  const maxSlippageBps =
    parsed != null && Number.isInteger(parsed) && parsed >= 0 && parsed <= 10_000
      ? parsed
      : null;

  // The bound is derived solely from the quote and the user's visible choice.
  const bound: QuoteBound | null = useMemo(
    () =>
      maxSlippageBps == null
        ? null
        : buildQuoteBound(quote, { maxSlippageBps }),
    [quote, maxSlippageBps],
  );

  const check: BoundCheckResult = useMemo(
    () => simulateWithBound(bound, context, now, maxAgeMs),
    [bound, context, now, maxAgeMs],
  );

  const needsRefresh = !check.ok && check.reason !== "NO_BOUND" && check.reason !== "UNENFORCEABLE_BOUND";
  const expectedMinOutput =
    maxSlippageBps == null ? null : computeMinOutput(quote.expectedOut, maxSlippageBps);

  const submit = () => {
    if (!check.ok) return;
    // Both paths receive the very same immutable argument object.
    onSimulate?.(check.arguments);
    onSubmit?.(check.arguments);
  };

  return (
    <section
      aria-label="Quote slippage bound"
      data-bound-status={check.ok ? "enforced" : check.reason}
      className={cn("rounded-lg border border-zinc-800 p-4 text-sm", className)}
    >
      <h2 className="mb-3 font-medium text-zinc-100">Quote &amp; slippage</h2>

      <dl className="grid grid-cols-2 gap-2">
        <div>
          <dt className="text-xs text-zinc-500">Expected output</dt>
          <dd className="text-zinc-100">{formatStake(quote.expectedOut.toString(), "XLM")}</dd>
        </div>
        <div>
          <dt className="text-xs text-zinc-500">Price impact</dt>
          <dd className="text-zinc-100">{(quote.priceImpactBps / 100).toFixed(2)}%</dd>
        </div>
      </dl>

      <div className="mt-4">
        <label htmlFor={inputId} className="block text-xs text-zinc-400">
          Maximum price impact (basis points)
        </label>
        <input
          id={inputId}
          type="number"
          min={0}
          max={10_000}
          step={1}
          value={maxSlippageInput}
          onChange={(event) => setMaxSlippageInput(event.target.value)}
          aria-describedby={messageId}
          aria-invalid={maxSlippageBps == null && maxSlippageInput !== ""}
          className="mt-1 w-full rounded-md border border-zinc-700 bg-zinc-900 px-2 py-1 text-zinc-100"
        />
        {expectedMinOutput != null && (
          <p className="mt-1 text-xs text-zinc-500">
            Minimum output: {formatStake(expectedMinOutput.toString(), "XLM")}
          </p>
        )}
        <div className="mt-2 flex gap-2" aria-label="Common slippage limits">
          {PRESETS.map((preset) => (
            <button
              key={preset}
              type="button"
              aria-pressed={maxSlippageBps === preset}
              onClick={() => setMaxSlippageInput(String(preset))}
              className={cn(
                "rounded px-2 py-1 text-xs",
                maxSlippageBps === preset
                  ? "bg-emerald-600 text-white"
                  : "bg-zinc-800 text-zinc-300",
              )}
            >
              {(preset / 100).toFixed(preset % 100 === 0 ? 0 : 1)}%
            </button>
          ))}
        </div>
      </div>

      <p
        id={messageId}
        role={check.ok ? "status" : "alert"}
        aria-live={check.ok ? "polite" : "assertive"}
        className={cn("mt-3 text-xs", check.ok ? "text-emerald-400" : "text-amber-400")}
      >
        {check.ok ? "Bound enforced for simulation and submission." : check.message}
      </p>

      {needsRefresh && (
        <button
          type="button"
          onClick={onRefreshQuote}
          className="mt-2 rounded-md bg-zinc-700 px-3 py-1.5 text-xs text-white"
        >
          Refresh quote
        </button>
      )}

      <button
        type="button"
        disabled={!check.ok}
        aria-disabled={!check.ok}
        onClick={submit}
        className={cn(
          "mt-4 w-full rounded-md px-3 py-2 text-sm font-medium",
          check.ok
            ? "bg-emerald-600 text-white hover:bg-emerald-500"
            : "cursor-not-allowed bg-zinc-800 text-zinc-500",
        )}
      >
        Review liquidity action
      </button>
    </section>
  );
}

export default LiquidityQuoteGuard;
