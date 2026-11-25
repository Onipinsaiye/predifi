"use client";

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import useSWR from "swr";
import Link from "next/link";
import {
  ArrowLeft,
  Radio,
  Users,
  Coins,
  Info,
  AlertTriangle,
  Flag,
} from "lucide-react";
import {
  fetchPoolDetail,
  poolDetailUrl,
  type PoolDetail,
  type PoolLiveEvent,
} from "@/lib/api/pools";
import { usePoolWebSocket } from "@/lib/hooks/usePoolWebSocket";
import { AnimatedNumber } from "@/components/pool/AnimatedNumber";
import { CountdownTimer } from "@/components/pool/CountdownTimer";
import { FreshnessIndicator } from "@/components/common/FreshnessIndicator";
import { cn } from "@/lib/utils";
import {
  getLifecycleExplanation,
  type LifecycleExplanation,
} from "@/lib/i18n/lifecycleCatalog";
import { ContentReportPanel } from "@/components/moderation/ContentReportPanel";

export interface PoolDetailViewProps {
  poolId: number;
  /** Optional JWT for authenticated WS (local may be permissive). */
  wsToken?: string | null;
  /**
   * Read-only demo mode. Uses static fixtures, never opens a WebSocket,
   * never issues API requests, and marks every value as illustrative.
   */
  demo?: boolean;
}

/** Static, clearly illustrative fixture used only in demo mode. */
const DEMO_POOL: PoolDetail = {
  pool_id: 0,
  name: "Demo pool — sample market",
  category: "demo",
  state: "open",
  creator: "0xDEMO000000000000000000000000000000000000",
  token: "0xDEMO000000000000000000000000000000000000",
  total_stake: 1250,
  prediction_count: 42,
  end_time: new Date(Date.now() + 1000 * 60 * 60 * 24).toISOString(),
  indexed_at: new Date().toISOString(),
  odds: [
    { outcome: 0, stake: 800, odds: 1.56 },
    { outcome: 1, stake: 450, odds: 2.78 },
  ],
};

function recomputeOdds(
  odds: PoolDetail["odds"],
  totalStake: number,
): PoolDetail["odds"] {
  if (totalStake <= 0) {
    return odds.map((o) => ({ ...o, odds: 0 }));
  }
  return odds.map((o) => ({
    ...o,
    odds: o.stake > 0 ? totalStake / o.stake : 0,
  }));
}

/**
 * Pool detail surface with live WebSocket updates for stakes, counts, odds,
 * and a closing countdown. Value changes animate for feedback.
 */
export function PoolDetailView({
  poolId,
  wsToken,
  demo = false,
}: PoolDetailViewProps) {
  const key = demo ? null : poolDetailUrl(poolId);
  const { data, error, isLoading, mutate } = useSWR(key, fetchPoolDetail);
  const [live, setLive] = useState<PoolDetail | null>(null);
  const [predictionCount, setPredictionCount] = useState(0);

  const pool = demo ? DEMO_POOL : live ?? data;

  const onEvent = useCallback(
    (event: PoolLiveEvent) => {
      if (demo) return;
      if (event.type !== "prediction_placed") return;
      const amount = Number(event.amount ?? 0);
      const outcome = Number(event.outcome ?? 0);

      setPredictionCount((c) => c + 1);
      setLive((prev) => {
        const base = prev ?? data;
        if (!base) return prev;

        const nextOdds = base.odds.map((o) =>
          o.outcome === outcome ? { ...o, stake: o.stake + amount } : { ...o },
        );
        // Ensure outcome row exists if API returned sparse odds
        if (!nextOdds.some((o) => o.outcome === outcome)) {
          nextOdds.push({ outcome, stake: amount, odds: 0 });
          nextOdds.sort((a, b) => a.outcome - b.outcome);
        }

        const total_stake = base.total_stake + amount;
        return {
          ...base,
          total_stake,
          odds: recomputeOdds(nextOdds, total_stake),
          prediction_count: (base.prediction_count ?? predictionCount) + 1,
        };
      });

      // Soft revalidate so server odds stay authoritative
      void mutate();
    },
    [data, demo, mutate, predictionCount],
  );

  const { status } = usePoolWebSocket({
    poolId,
    token: wsToken,
    onEvent,
    enabled: !demo && (Boolean(data) || Boolean(live)),
  });

  const maxStake = useMemo(
    () => Math.max(1, ...(pool?.odds.map((o) => o.stake) ?? [1])),
    [pool],
  );

  // Trade dialog state: the confirmation must communicate post-trade exposure
  // before any wallet prompt is opened.
  const [tradeOpen, setTradeOpen] = useState(false);
  const [tradeOutcome, setTradeOutcome] = useState<number | null>(null);
  const [tradeAmount, setTradeAmount] = useState("");
  const [availableBalance, setAvailableBalance] = useState<number | null>(null);
  const [slippageBps, setSlippageBps] = useState<number>(50);
  const [simulation, setSimulation] = useState<TradeSimulation | null>(null);
  const [simulating, setSimulating] = useState(false);
  const [submitted, setSubmitted] = useState(false);

  const openTrade = useCallback((outcome: number) => {
    setTradeOutcome(outcome);
    setTradeAmount("");
    setSimulation(null);
    setSubmitted(false);
    setTradeOpen(true);
  }, []);

  const closeTrade = useCallback(() => {
    setTradeOpen(false);
    setSimulation(null);
    setSimulating(false);
    setSubmitted(false);
  }, []);

  const parsedAmount = useMemo(() => {
    const n = Number(tradeAmount);
    return Number.isFinite(n) && n > 0 ? n : 0;
  }, [tradeAmount]);

  // Fingerprint of the current inputs. A preview may only be submitted while
  // its fingerprint matches the live inputs, so stale previews are rejected.
  const previewFingerprint = useMemo(
    () =>
      tradeOutcome === null
        ? ""
        : `${pool?.pool_id ?? poolId}:${tradeOutcome}:${parsedAmount}:${slippageBps}`,
    [pool?.pool_id, poolId, tradeOutcome, parsedAmount, slippageBps],
  );

  // Invalidate any previous simulation whenever inputs change.
  useEffect(() => {
    setSimulation(null);
  }, [previewFingerprint]);

  const runSimulation = useCallback(async () => {
    if (tradeOutcome === null || parsedAmount <= 0) return;
    const fingerprint = previewFingerprint;
    setSimulating(true);
    try {
      const result = await simulateTrade({
        poolId: pool?.pool_id ?? poolId,
        outcome: tradeOutcome,
        amount: parsedAmount,
        availableBalance,
        slippageBps,
        odds: pool?.odds ?? [],
        totalStake: pool?.total_stake ?? 0,
      });
      // Drop the result if inputs changed while simulating.
      setSimulation((prev) =>
        fingerprint === previewFingerprintRef.current
          ? result
          : prev,
      );
    } finally {
      setSimulating(false);
    }
  }, [
    tradeOutcome,
    parsedAmount,
    previewFingerprint,
    pool?.pool_id,
    pool?.odds,
    pool?.total_stake,
    poolId,
    availableBalance,
    slippageBps,
  ]);

  const previewFingerprintRef = useRef(previewFingerprint);
  useEffect(() => {
    previewFingerprintRef.current = previewFingerprint;
  }, [previewFingerprint]);

  const canSubmit =
    !submitted &&
    !simulating &&
    simulation !== null &&
    simulation.status === "success" &&
    simulation.fingerprint === previewFingerprint &&
    parsedAmount > 0;

  const handleConfirm = useCallback(() => {
    if (!canSubmit) return;
    // Wallet prompt would open here; the preview above is final for these inputs.
    setSubmitted(true);
  }, [canSubmit]);

  if (!demo && isLoading && !pool) {
    return (
      <div className="mx-auto max-w-3xl space-y-4 p-6">
        <div className="h-8 w-48 animate-pulse rounded bg-white/5" />
        <div className="h-24 animate-pulse rounded-2xl bg-white/5" />
        <div className="h-40 animate-pulse rounded-2xl bg-white/5" />
      </div>
    );
  }

  if (!demo && (error || !pool)) {
    return (
      <div className="mx-auto max-w-3xl p-6 text-zinc-400">
        <p>Could not load pool #{poolId}.</p>
        <Link href="/user/pool-market" className="mt-3 inline-block text-[#37B7C3]">
          Back to market
        </Link>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-[#0A0A0A] p-6 lg:p-8">
      <div className="mx-auto max-w-3xl space-y-6">
        {demo && <DemoBanner />}
        <div className="flex items-start justify-between gap-4">
          <div className="space-y-2">
            <Link
              href="/user/pool-market"
              className="inline-flex items-center gap-1 text-xs text-zinc-500 transition hover:text-white"
            >
              <ArrowLeft className="h-3.5 w-3.5" />
              Pool market
            </Link>
            <h1 className="text-2xl font-bold text-white sm:text-3xl">
              {pool.name || `Pool #${pool.pool_id}`}
              {demo && <IllustrativeTag />}
            </h1>
            <div className="flex flex-wrap items-center gap-2 text-xs text-zinc-500">
              <span className="rounded bg-white/5 px-2 py-0.5 text-zinc-300">
                {pool.category}
              </span>
              <LifecycleStatus state={pool.state} />
              <LiveBadge status={demo ? "demo" : status} />
              <FreshnessIndicator indexedAt={pool.indexed_at} />
            </div>
          </div>
          <CountdownTimer endTime={pool.end_time} />
        </div>

        <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
          <Metric
            icon={<Coins className="h-4 w-4 text-[#37B7C3]" />}
            label="Total stake"
            demo={demo}
            value={
              <AnimatedNumber
                value={pool.total_stake}
                className="text-xl font-semibold text-white"
              />
            }
          />
          <Metric
            icon={<Users className="h-4 w-4 text-[#37B7C3]" />}
            label="Live predictions"
            demo={demo}
            value={
              <AnimatedNumber
                value={pool.prediction_count ?? predictionCount}
                className="text-xl font-semibold text-white"
              />
            }
          />
          <Metric
            icon={<Radio className="h-4 w-4 text-[#37B7C3]" />}
            label="Outcomes"
            demo={demo}
            value={
              <span className="text-xl font-semibold text-white">
                {pool.odds.length || "—"}
              </span>
            }
          />
        </div>

        <section className="space-y-3 rounded-2xl border border-white/10 bg-[#121212] p-5">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-semibold text-white">
              {demo ? "Illustrative odds" : "Live odds"}
            </h2>
            <p className="text-[10px] text-zinc-500">
              {demo
                ? "Sample values — not live market data"
                : "Updates stream over WebSocket"}
            </p>
          </div>
          <ul className="space-y-3">
            {pool.odds.map((o) => {
              const width = Math.max(4, (o.stake / maxStake) * 100);
              return (
                <li key={o.outcome} className="space-y-1.5">
                  <div className="flex items-center justify-between text-xs">
                    <span className="text-zinc-300">Outcome {o.outcome}</span>
                    <span className="flex items-center gap-3 text-zinc-400">
                      <span>
                        stake{" "}
                        <AnimatedNumber
                          value={o.stake}
                          className="text-white"
                        />
                      </span>
                      <span>
                        odds{" "}
                        <AnimatedNumber
                          value={o.odds}
                          decimals={2}
                          suffix="x"
                          className="text-[#37B7C3]"
                        />
                      </span>
                      <button
                        type="button"
                        onClick={() => openTrade(o.outcome)}
                        disabled={demo}
                        className="rounded bg-[#37B7C3]/15 px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide text-[#37B7C3] transition hover:bg-[#37B7C3]/25 disabled:cursor-not-allowed disabled:opacity-40"
                      >
                        Trade
                      </button>
                    </span>
                  </div>
                  <div className="h-1.5 overflow-hidden rounded-full bg-zinc-800">
                    <div
                      className="h-full rounded-full bg-[#37B7C3] transition-[width] duration-500 ease-out"
                      style={{ width: `${width}%` }}
                    />
                  </div>
                </li>
              );
            })}
            {pool.odds.length === 0 && (
              <li className="text-sm text-zinc-500">No stakes yet — be first.</li>
            )}
          </ul>
        </section>

        {tradeOpen && tradeOutcome !== null && (
          <TradeDialog
            outcome={tradeOutcome}
            amount={tradeAmount}
            onAmountChange={setTradeAmount}
            availableBalance={availableBalance}
            onAvailableBalanceChange={setAvailableBalance}
            slippageBps={slippageBps}
            onSlippageChange={setSlippageBps}
            simulation={simulation}
            simulating={simulating}
            fingerprint={previewFingerprint}
            onSimulate={runSimulation}
            canSubmit={canSubmit}
            submitted={submitted}
            onConfirm={handleConfirm}
            onClose={closeTrade}
          />
        )}

        <ContentReportPanel
          contentType="pool"
          contentId={String(pool.pool_id ?? poolId)}
          contentLabel={pool.name || `Pool #${pool.pool_id ?? poolId}`}
          demo={demo}
        />

        <p className="text-[11px] text-zinc-600">
          Creator {pool.creator.slice(0, 6)}…{pool.creator.slice(-4)} · token{" "}
          {pool.token.slice(0, 6)}…
          {demo && " · illustrative only"}
        </p>
      </div>
    </div>
  );
}

export interface TradeSimulation {
  status: "success" | "failed";
  /** Fingerprint of the inputs this simulation was computed for. */
  fingerprint: string;
  /** Estimated position received (in outcome shares). */
  estimatedReceived: number;
  /** Fee deducted from the input amount. */
  fee: number;
  /** Balance remaining after amount + fee. */
  balanceAfterFees: number;
  /** Slippage bound applied to the estimate (in basis points). */
  slippageBps: number;
  /** Worst-case received position at the slippage bound. */
  minReceived: number;
  error?: string;
}

interface SimulateTradeArgs {
  poolId: number;
  outcome: number;
  amount: number;
  availableBalance: number | null;
  slippageBps: number;
  odds: PoolDetail["odds"];
  totalStake: number;
}

const TRADE_FEE_BPS = 30; // 0.30% protocol fee

/** Rounds to a fixed number of decimals without floating-point drift. */
export function roundTo(value: number, decimals: number): number {
  if (!Number.isFinite(value)) return 0;
  const factor = 10 ** decimals;
  return Math.round((value + Number.EPSILON) * factor) / factor;
}

/**
 * Deterministic local simulation used to preview post-trade exposure. In a
 * production build this would call the backend simulation endpoint; the
 * contract (status + fingerprint + estimates) is identical.
 */
export async function simulateTrade(
  args: SimulateTradeArgs,
): Promise<TradeSimulation> {
  const { amount, availableBalance, slippageBps, odds, outcome } = args;
  const fingerprint = `${args.poolId}:${outcome}:${amount}:${slippageBps}`;
  const fee = roundTo((amount * TRADE_FEE_BPS) / 10_000, 6);
  const balanceAfterFees = roundTo(
    (availableBalance ?? 0) - amount - fee,
    6,
  );

  if (availableBalance !== null && amount + fee > availableBalance) {
    return {
      status: "failed",
      fingerprint,
      estimatedReceived: 0,
      fee,
      balanceAfterFees,
      slippageBps,
      minReceived: 0,
      error: "Insufficient balance to cover amount plus fees.",
    };
  }

  const row = odds.find((o) => o.outcome === outcome);
  const price = row && row.odds > 0 ? row.odds : 0;
  if (price <= 0) {
    return {
      status: "failed",
      fingerprint,
      estimatedReceived: 0,
      fee,
      balanceAfterFees,
      slippageBps,
      minReceived: 0,
      error: "Simulation failed: no price available for this outcome.",
    };
  }

  const estimatedReceived = roundTo(amount / price, 6);
  const minReceived = roundTo(
    estimatedReceived * (1 - slippageBps / 10_000),
    6,
  );

  return {
    status: "success",
    fingerprint,
    estimatedReceived,
    fee,
    balanceAfterFees,
    slippageBps,
    minReceived,
  };
}

interface TradeDialogProps {
  outcome: number;
  amount: string;
  onAmountChange: (value: string) => void;
  availableBalance: number | null;
  onAvailableBalanceChange: (value: number | null) => void;
  slippageBps: number;
  onSlippageChange: (value: number) => void;
  simulation: TradeSimulation | null;
  simulating: boolean;
  fingerprint: string;
  onSimulate: () => void;
  canSubmit: boolean;
  submitted: boolean;
  onConfirm: () => void;
  onClose: () => void;
}

function TradeDialog({
  outcome,
  amount,
  onAmountChange,
  availableBalance,
  onAvailableBalanceChange,
  slippageBps,
  onSlippageChange,
  simulation,
  simulating,
  fingerprint,
  onSimulate,
  canSubmit,
  submitted,
  onConfirm,
  onClose,
}: TradeDialogProps) {
  const isFinal =
    simulation !== null &&
    simulation.status === "success" &&
    simulation.fingerprint === fingerprint;
  const insufficient =
    simulation !== null && simulation.status === "failed";

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={`Trade preview for outcome ${outcome}`}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
    >
      <div className="w-full max-w-md space-y-4 rounded-2xl border border-white/10 bg-[#121212] p-5">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-semibold text-white">
            Trade preview · Outcome {outcome}
          </h2>
          <button
            type="button"
            onClick={onClose}
            className="text-xs text-zinc-500 transition hover:text-white"
          >
            Close
          </button>
        </div>

        <label className="block space-y-1 text-xs text-zinc-400">
          <span>Input amount</span>
          <input
            type="number"
            min="0"
            step="any"
            inputMode="decimal"
            value={amount}
            onChange={(e) => onAmountChange(e.target.value)}
            className="w-full rounded-lg border border-white/10 bg-black/40 px-3 py-2 text-sm text-white outline-none focus:border-[#37B7C3]"
          />
        </label>

        <div className="grid grid-cols-2 gap-3">
          <label className="block space-y-1 text-xs text-zinc-400">
            <span>Available balance</span>
            <input
              type="number"
              min="0"
              step="any"
              inputMode="decimal"
              value={availableBalance ?? ""}
              onChange={(e) =>
                onAvailableBalanceChange(
                  e.target.value === "" ? null : Number(e.target.value),
                )
              }
              className="w-full rounded-lg border border-white/10 bg-black/40 px-3 py-2 text-sm text-white outline-none focus:border-[#37B7C3]"
            />
          </label>
          <label className="block space-y-1 text-xs text-zinc-400">
            <span>Slippage bound (bps)</span>
            <input
              type="number"
              min="0"
              step="1"
              inputMode="numeric"
              value={slippageBps}
              onChange={(e) => onSlippageChange(Number(e.target.value) || 0)}
              className="w-full rounded-lg border border-white/10 bg-black/40 px-3 py-2 text-sm text-white outline-none focus:border-[#37B7C3]"
            />
          </label>
        </div>

        <TradePreview
          amount={Number(amount) || 0}
          simulation={simulation}
          simulating={simulating}
          fingerprint={fingerprint}
          slippageBps={slippageBps}
          insufficient={insufficient}
        />

        <div className="flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={onSimulate}
            disabled={simulating || !(Number(amount) > 0)}
            className="rounded-lg border border-white/10 px-3 py-2 text-xs text-zinc-300 transition hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {simulating ? "Simulating…" : "Simulate"}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            disabled={!canSubmit}
            className="rounded-lg bg-[#37B7C3] px-3 py-2 text-xs font-semibold text-black transition hover:bg-[#37B7C3]/90 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {submitted ? "Submitted" : "Confirm & sign"}
          </button>
        </div>
        {!isFinal && !submitted && (
          <p className="text-[10px] text-amber-300">
            Estimates are non-final until simulation succeeds.
          </p>
        )}
      </div>
    </div>
  );
}

interface TradePreviewProps {
  amount: number;
  simulation: TradeSimulation | null;
  simulating: boolean;
  fingerprint: string;
  slippageBps: number;
  insufficient: boolean;
}

/**
 * Presentational preview of post-trade exposure. Every value is labelled as a
 * non-final estimate until a simulation matching the current inputs succeeds.
 */
export function TradePreview({
  amount,
  simulation,
  simulating,
  fingerprint,
  slippageBps,
  insufficient,
}: TradePreviewProps) {
  const matches =
    simulation !== null && simulation.fingerprint === fingerprint;
  const final = matches && simulation.status === "success";
  const estimateLabel = final ? "estimate" : "non-final estimate";

  const received = matches ? simulation.estimatedReceived : null;
  const balanceAfter = matches ? simulation.balanceAfterFees : null;
  const minReceived = matches ? simulation.minReceived : null;

  return (
    <div className="space-y-2 rounded-xl border border-white/10 bg-black/30 p-3 text-xs">
      <div className="flex items-center justify-between">
        <span className="text-zinc-400">Input amount</span>
        <span className="text-white">{roundTo(amount, 6)}</span>
      </div>
      <div className="flex items-center justify-between">
        <span className="text-zinc-400">
          Estimated received position{" "}
          <span className="text-[10px] text-amber-300">({estimateLabel})</span>
        </span>
        <span className="text-white">
          {received === null ? "—" : roundTo(received, 6)}
        </span>
      </div>
      <div className="flex items-center justify-between">
        <span className="text-zinc-400">
          Available balance after fees{" "}
          <span className="text-[10px] text-amber-300">({estimateLabel})</span>
        </span>
        <span className="text-white">
          {balanceAfter === null ? "—" : roundTo(balanceAfter, 6)}
        </span>
      </div>
      <div className="flex items-center justify-between">
        <span className="text-zinc-400">Slippage bound</span>
        <span className="text-white">
          {(slippageBps / 100).toFixed(2)}%
          {minReceived !== null && (
            <span className="ml-1 text-zinc-500">
              (min {roundTo(minReceived, 6)})
            </span>
          )}
        </span>
      </div>

      {simulating && (
        <p className="text-[10px] text-zinc-500">Simulating…</p>
      )}
      {insufficient && (
        <p
          role="alert"
          className="flex items-center gap-1 text-[10px] text-red-400"
        >
          <AlertTriangle className="h-3 w-3" aria-hidden="true" />
          {simulation?.error ?? "Insufficient balance."}
        </p>
      )}
      {!final && !simulating && !insufficient && (
        <p className="text-[10px] text-amber-300">
          Non-final: run simulation to confirm these estimates.
        </p>
      )}
    </div>
  );
}

function DemoBanner() {
  return (
    <div
      role="note"
      className="flex items-start gap-2 rounded-xl border border-amber-400/30 bg-amber-400/10 p-3 text-xs text-amber-200"
    >
      <Info className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
      <div className="space-y-1">
        <p className="font-semibold">Demo mode — illustrative data only</p>
        <p className="text-amber-200/80">
          All values below are sample fixtures, not live markets. No network
          requests are made and wallet signing is disabled.
        </p>
        <Link
          href="/user/pool-market"
          className="inline-block font-medium text-[#37B7C3] underline-offset-2 hover:underline"
        >
          Exit demo and view live data
        </Link>
      </div>
    </div>
  );
}

function IllustrativeTag() {
  return (
    <span className="ml-2 align-middle text-[10px] font-medium uppercase tracking-wide text-amber-300">
      (illustrative)
    </span>
  );
}

/**
 * Status label with contextual lifecycle explanation from the shared
 * catalog, and an accessible live-region announcement on status change.
 */
function LifecycleStatus({ state }: { state: string }) {
  const explanation: LifecycleExplanation = getLifecycleExplanation(state);
  const [announcement, setAnnouncement] = useState("");
  const previous = useRef<string | null>(null);

  useEffect(() => {
    if (previous.current !== null && previous.current !== state) {
      setAnnouncement(
        `${explanation.label}. ${explanation.description} Next: ${explanation.nextTransition} (source: ${explanation.source}).`,
      );
    }
    previous.current = state;
  }, [state, explanation]);

  return (
    <span className="inline-flex flex-wrap items-center gap-1.5">
      <span
        className="capitalize text-zinc-300"
        title={`${explanation.description} Next: ${explanation.nextTransition} (source: ${explanation.source})`}
      >
        {explanation.label}
      </span>
      <span className="text-zinc-600" aria-hidden="true">
        ·
      </span>
      <span className="text-zinc-500">{explanation.nextTransition}</span>
      <span className="text-zinc-600" aria-hidden="true">
        ·
      </span>
      <span className="text-zinc-600">source: {explanation.source}</span>
      <span role="status" aria-live="polite" className="sr-only">
        {announcement}
      </span>
    </span>
  );
}

function Metric({
  icon,
  label,
  value,
  demo = false,
}: {
  icon: ReactNode;
  label: string;
  value: ReactNode;
  demo?: boolean;
}) {
  return (
    <div className="rounded-2xl border border-white/10 bg-[#121212] p-4">
      <div className="mb-2 flex items-center gap-2 text-[11px] uppercase tracking-wide text-zinc-500">
        {icon}
        {label}
        {demo && (
          <span className="ml-auto text-[9px] normal-case tracking-normal text-amber-300">
            illustrative
          </span>
        )}
      </div>
      {value}
    </div>
  );
}

/**
 * Moderation notice: content moderation is a presentation-layer concern and
 * never mutates on-chain funds or market resolution.
 */
export function ModerationNotice() {
  return (
    <p
      role="note"
      className="flex items-start gap-1.5 text-[10px] text-zinc-500"
    >
      <Flag className="mt-0.5 h-3 w-3 shrink-0" aria-hidden="true" />
      <span>
        Reporting and moderation affect content visibility only. They never
        alter on-chain funds, stakes, or market resolution.
      </span>
    </p>
  );
}

function LiveBadge({ status }: { status: string }) {
  const live = status === "open";
  const demo = status === "demo";
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[10px] uppercase tracking-wide",
        demo
          ? "bg-amber-400/15 text-amber-300"
          : live
            ? "bg-[#37B7C3]/15 text-[#37B7C3]"
            : "bg-zinc-800 text-zinc-500",
      )}
    >
      <span
        className={cn(
          "h-1.5 w-1.5 rounded-full",
          demo
            ? "bg-amber-300"
            : live
              ? "animate-pulse bg-[#37B7C3]"
              : "bg-zinc-600",
        )}
      />
      {demo ? "Demo" : live ? "Live" : status}
    </span>
  );
}
