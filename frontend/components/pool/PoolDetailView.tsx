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
import { ArrowLeft, Radio, Users, Coins, Info } from "lucide-react";
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

        <p className="text-[11px] text-zinc-600">
          Creator {pool.creator.slice(0, 6)}…{pool.creator.slice(-4)} · token{" "}
          {pool.token.slice(0, 6)}…
          {demo && " · illustrative only"}
        </p>
      </div>
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
