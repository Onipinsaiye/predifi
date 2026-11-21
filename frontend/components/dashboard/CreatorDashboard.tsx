"use client";

/**
 * Creator dashboard for authored markets (Issue #279).
 *
 * Displays:
 * - Aggregate creator stats (pools created, reward-eligible pools, total volume).
 * - Tabbed list of the creator's authored markets filtered by state
 *   (active / closed / settled).
 * - Per-pool summary cards with link to full market detail.
 * - Empty state when the creator has no pools yet.
 *
 * Data sources:
 * - Stats:  GET /api/v1/creators/:address/stats
 * - Pools:  GET /api/v1/pools?creator=:address
 *
 * Both are fetched client-side via SWR so the page stays fast and updates
 * when the creator resolves or creates a pool without a full page reload.
 */

import { useState, useMemo } from "react";
import useSWR from "swr";
import {
  Card,
  CardContent,
  EmptyState,
  ErrorState,
  Skeleton,
} from "@/components/ui";
import { cn } from "@/lib/utils";
import {
  fetchCreatorStats,
  fetchCreatorPools,
  creatorStatsUrl,
  creatorPoolsUrl,
} from "@/lib/api/creator";
import type { Pool } from "@/lib/api/pools";

// ─── Types ────────────────────────────────────────────────────────────────────

type PoolStatusTab = "active" | "closed" | "settled";

const STATUS_TABS: { value: PoolStatusTab; label: string }[] = [
  { value: "active", label: "Active" },
  { value: "closed", label: "Closed" },
  { value: "settled", label: "Settled" },
];

// ─── Sub-components ───────────────────────────────────────────────────────────

function StatCard({
  label,
  value,
  loading,
}: {
  label: string;
  value: string | number;
  loading: boolean;
}) {
  return (
    <Card className="bg-[#121212] border-zinc-800">
      <CardContent className="pt-6">
        <p className="text-xs text-zinc-500 uppercase tracking-wider mb-1">{label}</p>
        {loading ? (
          <Skeleton className="h-7 w-24 mt-1" />
        ) : (
          <p className="text-2xl font-bold text-white">{value}</p>
        )}
      </CardContent>
    </Card>
  );
}

function PoolCard({ pool }: { pool: Pool }) {
  const endDate = new Date(pool.end_time * 1000);
  const isExpired = endDate < new Date();

  return (
    <a
      href={`/user/pool-market/${pool.pool_id}`}
      className="block rounded-xl bg-zinc-900/50 hover:bg-zinc-900/80 transition-colors p-4"
      aria-label={`View market: ${pool.name}`}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 space-y-1.5">
          <p className="truncate text-sm font-medium text-white">{pool.name}</p>
          <div className="flex items-center gap-2 text-xs text-zinc-500 flex-wrap">
            <span className="rounded bg-zinc-800 px-1.5 py-0.5 font-medium text-zinc-400">
              {pool.category}
            </span>
            <span>
              {pool.total_stake.toLocaleString()} staked
            </span>
            {pool.result !== null && (
              <span className="text-zinc-600">
                Outcome: {pool.result}
              </span>
            )}
          </div>
          {pool.tags.length > 0 && (
            <p className="text-[10px] text-zinc-600 truncate">
              {pool.tags.map((t) => `#${t}`).join(" ")}
            </p>
          )}
          <p className="text-[10px] text-zinc-600">
            {isExpired ? "Ended" : "Ends"}{" "}
            {endDate.toLocaleDateString(undefined, {
              year: "numeric",
              month: "short",
              day: "numeric",
            })}
          </p>
        </div>
        <span
          className={cn(
            "shrink-0 rounded-full px-3 py-1 text-xs font-medium capitalize",
            pool.state === "active" && "bg-emerald-400/10 text-emerald-400",
            pool.state === "closed" && "bg-yellow-400/10 text-yellow-400",
            pool.state === "settled" && "bg-blue-400/10 text-blue-400",
          )}
        >
          {pool.state}
        </span>
      </div>
    </a>
  );
}

// ─── Main component ───────────────────────────────────────────────────────────

interface CreatorDashboardProps {
  /** Stellar address of the creator whose dashboard to display. */
  address: string;
  /** Optional bearer token for authenticated requests. */
  bearerToken?: string;
}

export function CreatorDashboard({ address, bearerToken }: CreatorDashboardProps) {
  const [activeTab, setActiveTab] = useState<PoolStatusTab>("active");
  const [page, setPage] = useState(0);
  const PAGE_SIZE = 20;

  // ─── Creator stats ──────────────────────────────────────────────────────────
  const {
    data: stats,
    isLoading: statsLoading,
    error: statsError,
  } = useSWR(creatorStatsUrl(address), () =>
    fetchCreatorStats(address, bearerToken),
  );

  // ─── Creator pools ──────────────────────────────────────────────────────────
  const poolsCacheKey = creatorPoolsUrl(address, {
    status: activeTab,
    limit: PAGE_SIZE,
    offset: page * PAGE_SIZE,
  });

  const {
    data: poolsData,
    isLoading: poolsLoading,
    error: poolsError,
    mutate: refreshPools,
  } = useSWR(poolsCacheKey, () =>
    fetchCreatorPools(address, {
      status: activeTab,
      limit: PAGE_SIZE,
      offset: page * PAGE_SIZE,
    }),
  );

  const pools: Pool[] = poolsData?.pools ?? [];
  const total = poolsData?.total ?? 0;
  const totalPages = Math.ceil(total / PAGE_SIZE);

  // ─── Tab change ─────────────────────────────────────────────────────────────
  function handleTabChange(tab: PoolStatusTab) {
    setActiveTab(tab);
    setPage(0);
  }

  // ─── Stats values ───────────────────────────────────────────────────────────
  const statsCards = useMemo(
    () => [
      {
        label: "Markets created",
        value: stats?.pools_created?.toLocaleString() ?? "—",
      },
      {
        label: "Reward-eligible",
        value: stats?.pools_reward_eligible?.toLocaleString() ?? "—",
      },
      {
        label: "Total volume",
        value: stats ? stats.total_volume.toLocaleString() : "—",
      },
    ],
    [stats],
  );

  // ─── Render ─────────────────────────────────────────────────────────────────
  return (
    <div className="space-y-8">
      {/* Stats row */}
      {statsError ? (
        <p className="text-sm text-zinc-500" role="status">
          Could not load creator stats.
        </p>
      ) : (
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
          {statsCards.map((card) => (
            <StatCard
              key={card.label}
              label={card.label}
              value={card.value}
              loading={statsLoading}
            />
          ))}
        </div>
      )}

      {/* Markets section */}
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <h2 className="text-lg font-semibold text-white">Your markets</h2>
          {/* Status tabs */}
          <div className="flex gap-1" role="tablist" aria-label="Market status filter">
            {STATUS_TABS.map((tab) => (
              <button
                key={tab.value}
                type="button"
                role="tab"
                aria-selected={activeTab === tab.value}
                onClick={() => handleTabChange(tab.value)}
                className={cn(
                  "rounded px-3 py-1.5 text-xs font-medium border transition-colors",
                  activeTab === tab.value
                    ? "bg-zinc-700/50 border-zinc-600 text-white"
                    : "border-zinc-800 text-zinc-500 hover:text-zinc-300",
                )}
              >
                {tab.label}
              </button>
            ))}
          </div>
        </div>

        {/* Pool count */}
        {!poolsLoading && !poolsError && (
          <p className="text-xs text-zinc-500" role="status" aria-live="polite">
            {total} {activeTab} market{total !== 1 ? "s" : ""}
          </p>
        )}

        {/* Pool list */}
        {poolsLoading ? (
          <div className="space-y-3" aria-label="Loading markets" role="status">
            {Array.from({ length: 4 }).map((_, i) => (
              <div key={i} className="rounded-xl bg-zinc-900/50 p-4">
                <div className="space-y-2">
                  <Skeleton className="h-4 w-56" />
                  <Skeleton className="h-3 w-36" />
                </div>
              </div>
            ))}
          </div>
        ) : poolsError ? (
          <ErrorState
            heading="Couldn't load your markets"
            description="The pool service didn't respond. Check your connection and try again."
            onRetry={() => refreshPools()}
          />
        ) : pools.length === 0 ? (
          <EmptyState
            heading={`No ${activeTab} markets yet`}
            description={
              activeTab === "active"
                ? "Create your first prediction market to see it here."
                : `You haven't had any ${activeTab} markets yet.`
            }
          />
        ) : (
          <div className="space-y-3" role="list" aria-label={`${activeTab} markets`}>
            {pools.map((pool) => (
              <div key={pool.pool_id} role="listitem">
                <PoolCard pool={pool} />
              </div>
            ))}
          </div>
        )}

        {/* Pagination */}
        {totalPages > 1 && (
          <div className="flex items-center justify-center gap-2">
            <button
              type="button"
              onClick={() => setPage((p) => Math.max(0, p - 1))}
              disabled={page === 0}
              className="rounded-lg px-3 py-1.5 text-sm font-medium border border-zinc-800 text-zinc-400 hover:text-zinc-300 disabled:opacity-40 disabled:cursor-not-allowed"
            >
              Previous
            </button>
            <span className="text-sm text-zinc-500">
              Page {page + 1} of {totalPages}
            </span>
            <button
              type="button"
              onClick={() => setPage((p) => Math.min(totalPages - 1, p + 1))}
              disabled={page >= totalPages - 1}
              className="rounded-lg px-3 py-1.5 text-sm font-medium border border-zinc-800 text-zinc-400 hover:text-zinc-300 disabled:opacity-40 disabled:cursor-not-allowed"
            >
              Next
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
