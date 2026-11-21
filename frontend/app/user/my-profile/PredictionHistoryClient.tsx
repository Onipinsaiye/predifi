"use client";

import { useCallback, useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { formatUtcDateTime } from "@/lib/date";
import { formatChartValue } from "@/lib/stakeFilters";
import { useCurrentUserAddress } from "@/lib/hooks/useCurrentUserAddress";
import { useProfile } from "@/lib/hooks/useProfile";
import { VirtualizedList } from "@/components/common/VirtualizedList";
import { NotificationBell } from "@/components/notifications/NotificationBell";
import { PerformanceCharts } from "@/components/profile/PerformanceCharts";
import {
  EmptyState,
  ErrorState,
  PartialDataBanner,
  Skeleton,
  SkeletonScreen,
} from "@/components/ui";
import type { ClaimStatus } from "@/lib/api/profile";
import {
  PORTFOLIO_EXPORT_SCHEMA_VERSION,
  buildPortfolioExport,
  downloadPortfolioJson,
  type PortfolioExport,
} from "@/lib/portfolioExport";

type PredictionStatus = "Active" | "Won" | "Lost" | "Pending";

const STATUS_STYLES: Record<PredictionStatus, string> = {
  Active: "bg-blue-500/20 text-blue-400",
  Won: "bg-emerald-500/20 text-emerald-400",
  Lost: "bg-red-500/20 text-red-400",
  Pending: "bg-yellow-500/20 text-yellow-400",
};

type Tab = "All" | PredictionStatus;
const TABS: Tab[] = ["All", "Active", "Won", "Lost", "Pending"];

/** Fixed row height used for virtualizing long prediction histories. */
const CLAIM_ROW_HEIGHT = 56;

function predictionStatus(claim: ClaimStatus): PredictionStatus {
  if (claim.pool_state === "active") return "Active";
  if (claim.pool_state !== "settled" || claim.is_winner === null) return "Pending";
  return claim.is_winner ? "Won" : "Lost";
}

function ClaimBadge({ claim }: { claim: ClaimStatus }) {
  if (claim.is_winner !== true) {
    return <span className="text-xs text-zinc-600">—</span>;
  }
  if (claim.claimed) {
    return <span className="text-xs font-medium text-emerald-400">Claimed</span>;
  }
  if (claim.claim_expired) {
    return <span className="text-xs font-medium text-red-400">Expired</span>;
  }
  if (claim.claim_window_expires_at) {
    return (
      <span className="text-xs font-medium text-yellow-400">
        Claim by {formatUtcDateTime(claim.claim_window_expires_at)}
      </span>
    );
  }
  return <span className="text-xs font-medium text-yellow-400">Unclaimed</span>;
}

function truncateAddress(address: string): string {
  if (address.length <= 10) return address;
  return `${address.slice(0, 4)}…${address.slice(-4)}`;
}

function ClaimRow({ claim }: { claim: ClaimStatus }) {
  const status = predictionStatus(claim);
  return (
    <div
      style={{ height: CLAIM_ROW_HEIGHT }}
      className="grid grid-cols-2 sm:grid-cols-7 items-center gap-2 px-4 border-b border-zinc-800/50 hover:bg-white/[0.02] transition-colors overflow-hidden"
    >
      <div className="col-span-2 sm:col-span-2 min-w-0">
        <p className="text-sm text-white truncate">{claim.pool_name}</p>
      </div>
      <span className="text-sm text-zinc-300 hidden sm:block">Outcome {claim.outcome}</span>
      <span className="text-sm font-mono text-zinc-300 hidden sm:block">
        {formatChartValue(claim.amount)}
      </span>
      <span
        className={cn(
          "text-[10px] font-semibold px-2 py-1 rounded-full w-fit",
          STATUS_STYLES[status],
        )}
      >
        {status}
      </span>
      <div className="col-span-2 hidden sm:block">
        <ClaimBadge claim={claim} />
      </div>
    </div>
  );
}

export function PredictionHistoryClient() {
  const address = useCurrentUserAddress();
  const { profile, isLoading, isError, isStale, refresh } = useProfile(address);
  const [activeTab, setActiveTab] = useState<Tab>("All");
  const [exported, setExported] = useState<PortfolioExport | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);

  const handleExportJson = useCallback(() => {
    if (!address || !profile) {
      setExportError("Connect your wallet and load your history before exporting.");
      return;
    }
    try {
      const result = buildPortfolioExport({
        address,
        claims: profile.claims ?? [],
        stats: profile.stats ?? null,
        performance: profile.performance ?? [],
      });
      setExportError(null);
      setExported(result);
      downloadPortfolioJson(result);
    } catch (err) {
      setExported(null);
      setExportError(
        err instanceof Error ? err.message : "Could not build the portfolio export.",
      );
    }
  }, [address, profile]);

  const handleFetchNextPage = useCallback(() => {
    if (!exported?.continuation) return;
    // The continuation cursor is opaque; the profile service resolves it.
    refresh();
  }, [exported, refresh]);

  const filtered = useMemo(() => {
    const claims = profile?.claims ?? [];
    if (activeTab === "All") return claims;
    return claims.filter((c) => predictionStatus(c) === activeTab);
  }, [profile?.claims, activeTab]);

  const stats = profile?.stats;
  const hasClaims = (profile?.claims?.length ?? 0) > 0;

  return (
    <div className="min-h-screen bg-[#0A0A0A] p-6 lg:p-8 space-y-8">
      {/* Profile header */}
      <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-5">
        <div className="flex items-center gap-5">
          <div className="w-16 h-16 rounded-full bg-gradient-to-br from-[#37B7C3]/40 to-indigo-500/40 border border-white/10 flex-shrink-0" />
          <div>
            <h1 className="text-2xl font-bold text-white">My Profile</h1>
            <p className="text-zinc-400 text-xs mt-0.5 font-mono">
              {address ? truncateAddress(address) : "Not connected"}
            </p>
          </div>
        </div>
        <NotificationBell />
      </div>

      {isError && !isStale ? (
        <ErrorState
          heading="Couldn't load your history"
          description={
            address
              ? "Your profile service didn't respond. Check your connection and try again."
              : "Connect your wallet to load your prediction history."
          }
          onRetry={refresh}
        />
      ) : (
        <>
          {isStale && (
            <PartialDataBanner
              message="Showing cached history — your profile service is unreachable."
              onRetry={refresh}
            />
          )}

          {/* Stats row */}
          <SkeletonScreen label="Loading your statistics">
            <div className="grid grid-cols-2 sm:grid-cols-5 gap-4">
              {isLoading || !stats
                ? Array.from({ length: 5 }).map((_, i) => (
                    <div key={i} className="rounded-xl border border-zinc-800 bg-zinc-900 p-4 space-y-2">
                      <Skeleton className="h-3 w-20" />
                      <Skeleton className="h-5 w-16" />
                    </div>
                  ))
                : [
                    { label: "Total Predictions", value: stats.total_predictions },
                    { label: "Win Rate", value: `${stats.win_rate.toFixed(0)}%` },
                    { label: "Total Staked", value: formatChartValue(stats.total_staked) },
                    { label: "Total Earned", value: formatChartValue(stats.total_earnings) },
                    { label: "Active Positions", value: stats.active_positions },
                  ].map(({ label, value }) => (
                    <div key={label} className="rounded-xl border border-zinc-800 bg-zinc-900 p-4 space-y-1">
                      <p className="text-[10px] text-zinc-500 uppercase tracking-wider">{label}</p>
                      <p className="text-lg font-bold text-white font-mono">{value}</p>
                    </div>
                  ))}
            </div>
          </SkeletonScreen>

          {/* Performance charts */}
          {!isLoading && stats && (
            <PerformanceCharts stats={stats} performance={profile?.performance ?? []} />
          )}

          {/* Predictions history */}
          <div className="space-y-4">
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
              <h2 className="text-base font-semibold text-white">Prediction History</h2>
              <div className="flex flex-col items-start sm:items-end gap-1">
                <button
                  type="button"
                  onClick={handleExportJson}
                  disabled={!address || !profile}
                  className="px-3 py-1.5 text-xs font-medium rounded-lg border border-zinc-700 text-zinc-200 hover:border-[#37B7C3] hover:text-[#37B7C3] disabled:opacity-50 disabled:cursor-not-allowed transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#37B7C3]"
                >
                  Export portfolio JSON
                </button>
                <p className="text-[10px] text-zinc-500">
                  Schema v{PORTFOLIO_EXPORT_SCHEMA_VERSION}
                </p>
              </div>
            </div>

            {exportError && (
              <p role="alert" className="text-xs text-red-400">
                {exportError}
              </p>
            )}

            {exported && (
              <div className="rounded-lg border border-zinc-800 bg-zinc-900 px-4 py-3 space-y-2">
                <p className="text-xs text-zinc-300">
                  Exported {exported.positions.length} position
                  {exported.positions.length === 1 ? "" : "s"} and{" "}
                  {exported.ledger_entries.length} ledger entr
                  {exported.ledger_entries.length === 1 ? "y" : "ies"} (schema v
                  {exported.schema_version}).
                </p>
                {exported.truncated ? (
                  <div className="flex flex-col sm:flex-row sm:items-center gap-2">
                    <p className="text-xs text-yellow-400">
                      Export was truncated — more records are available.
                    </p>
                    <button
                      type="button"
                      onClick={handleFetchNextPage}
                      className="px-3 py-1 text-xs font-medium rounded-lg border border-yellow-500/40 text-yellow-300 hover:bg-yellow-500/10 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-yellow-400"
                    >
                      Fetch next page
                    </button>
                  </div>
                ) : (
                  <p className="text-xs text-emerald-400">Export is complete.</p>
                )}
              </div>
            )}

            {/* Tabs */}
            <div
              className="flex items-center gap-1 border-b border-zinc-800 overflow-x-auto"
              role="tablist"
              aria-label="Filter predictions by status"
            >
              {TABS.map((tab) => (
                <button
                  key={tab}
                  type="button"
                  role="tab"
                  aria-selected={activeTab === tab}
                  onClick={() => setActiveTab(tab)}
                  className={cn(
                    "px-4 py-2.5 text-sm font-medium transition-colors relative whitespace-nowrap focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#37B7C3] rounded",
                    activeTab === tab ? "text-[#37B7C3]" : "text-zinc-500 hover:text-zinc-300",
                  )}
                >
                  {tab}
                  {activeTab === tab && (
                    <span className="absolute bottom-0 left-0 w-full h-0.5 bg-[#37B7C3]" />
                  )}
                </button>
              ))}
            </div>

            {/* Table */}
            {isLoading ? (
              <SkeletonScreen label="Loading prediction history" className="space-y-2">
                {Array.from({ length: 4 }).map((_, i) => (
                  <Skeleton key={i} className="h-14 w-full rounded-lg" />
                ))}
              </SkeletonScreen>
            ) : !hasClaims ? (
              <EmptyState
                heading="No predictions yet"
                description="Place your first prediction on an active pool and it will show up here."
                headingAs="h3"
              />
            ) : filtered.length === 0 ? (
              <EmptyState
                heading={`No ${activeTab.toLowerCase()} predictions`}
                description="Try another tab to see the rest of your history."
                headingAs="h3"
              />
            ) : (
              <div className="rounded-xl border border-zinc-800 overflow-hidden">
                {/* Header */}
                <div className="hidden sm:grid grid-cols-7 px-4 py-2.5 bg-zinc-900 text-[10px] text-zinc-500 uppercase tracking-wider border-b border-zinc-800">
                  <span className="col-span-2">Pool</span>
                  <span>Outcome</span>
                  <span>Stake</span>
                  <span>Status</span>
                  <span className="col-span-2">Claim</span>
                </div>
                {/* Rows */}
                <VirtualizedList
                  items={filtered}
                  itemKey={(claim) => claim.prediction_id}
                  rowHeight={CLAIM_ROW_HEIGHT}
                  height={Math.min(filtered.length * CLAIM_ROW_HEIGHT, 560)}
                  ariaLabel="Prediction history"
                  renderRow={(claim) => <ClaimRow claim={claim} />}
                />
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
}
