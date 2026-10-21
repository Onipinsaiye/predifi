"use client";

/**
 * PoolDetailClient — the interactive surface of the pool detail route (#84).
 *
 * The server component owns SEO metadata; this component owns the four
 * predictable states the route needs:
 *
 *   loading  → PoolDetailSkeleton (accessible `role="status"` + label)
 *   empty    → pool does not exist (404) — heading + link back, no retry
 *   error    → request failed — heading + retry (`mutate`)
 *   partial  → cached pool rendered while the API is failing (stale banner),
 *              or a pool that has no odds yet
 *
 * `initialPool` is the server-rendered pool, passed as SWR `fallbackData` so
 * the happy path still paints server-side (no layout shift, good SEO) while
 * the client keeps it fresh.
 */

import Link from "next/link";
import useSWR from "swr";
import { ArrowLeft } from "lucide-react";
import {
  fetchPoolDetail,
  poolDetailUrl,
  ApiError,
  type PoolDetail,
} from "@/lib/api/pools";
import {
  CopyButton,
  ErrorState,
  EmptyState,
  PartialDataBanner,
  PoolDetailSkeleton,
  ShareButton,
} from "@/components/ui";

const SITE_URL = "https://predifi.app";

function formatStake(amount: number, token: string): string {
  return `${amount.toLocaleString()} ${token}`;
}

function outcomeLabel(outcome: number, labels?: string[]): string {
  return labels?.[outcome] ?? `Outcome ${outcome + 1}`;
}

export interface PoolDetailClientProps {
  poolId: string;
  /** Server-rendered pool used as SWR fallback data for the first paint. */
  initialPool?: PoolDetail | null;
}

export function PoolDetailClient({ poolId, initialPool }: PoolDetailClientProps) {
  const { data, error, isLoading, mutate } = useSWR<PoolDetail>(
    poolDetailUrl(poolId),
    fetchPoolDetail,
    { fallbackData: initialPool ?? undefined },
  );

  const pool = data ?? initialPool ?? undefined;
  const isStale = Boolean(error && pool);
  const isMissing =
    !pool && (error instanceof ApiError ? error.status === 404 : false);

  if (isLoading && !pool) {
    return (
      <div className="min-h-screen bg-[#0A0A0A] p-6 lg:p-8">
        <div className="mx-auto max-w-3xl">
          <PoolDetailSkeleton />
        </div>
      </div>
    );
  }

  if (!pool) {
    if (isMissing) {
      return (
        <div className="min-h-screen bg-[#0A0A0A] p-6 lg:p-8">
          <div className="mx-auto max-w-3xl">
            <EmptyState
              heading="Pool not found"
              description={`No prediction pool exists with the ID "${poolId}". It may have been removed, or the link may be incorrect.`}
              action={
                <Link
                  href="/user/pool-market"
                  className="inline-flex items-center gap-2 rounded-lg border border-zinc-700 bg-zinc-800/50 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-zinc-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-zinc-500"
                >
                  <ArrowLeft className="h-4 w-4" aria-hidden="true" />
                  Back to pools
                </Link>
              }
            />
          </div>
        </div>
      );
    }

    return (
      <div className="min-h-screen bg-[#0A0A0A] p-6 lg:p-8">
        <div className="mx-auto max-w-3xl">
          <ErrorState
            heading="Couldn't load this pool"
            description="The pool service didn't respond. Check your connection and try again."
            onRetry={() => void mutate()}
            actions={
              <Link
                href="/user/pool-market"
                className="inline-flex items-center gap-2 rounded-lg border border-zinc-700 bg-zinc-800/50 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-zinc-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-zinc-500"
              >
                <ArrowLeft className="h-4 w-4" aria-hidden="true" />
                Back to pools
              </Link>
            }
          />
        </div>
      </div>
    );
  }

  const shareUrl = `${SITE_URL}/user/pool-market/${pool.pool_id}`;
  const shareTitle = `${pool.name} — predict on PrediFi`;

  return (
    <div className="min-h-screen bg-[#0A0A0A] p-6 lg:p-8">
      <div className="mx-auto max-w-3xl space-y-6">
        {isStale && (
          <PartialDataBanner
            message="Showing cached pool data — the pool service is unreachable."
            onRetry={() => void mutate()}
          />
        )}

        <div className="flex items-start justify-between gap-4">
          <div className="space-y-1">
            <p className="text-xs uppercase tracking-wide text-[#7DE3EC]">
              {pool.category}
            </p>
            <h1 className="text-3xl font-bold text-white">{pool.name}</h1>
            <p className="text-sm text-zinc-500">
              {formatStake(pool.total_stake, pool.token)} staked · {pool.state}
            </p>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            <CopyButton
              text={shareUrl}
              size="md"
              className="rounded-md border border-zinc-700 p-2"
              aria-label="Copy pool link"
            />
            <ShareButton url={shareUrl} title={shareTitle} text={pool.name} />
          </div>
        </div>

        <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-5 space-y-3">
          <h2 className="text-sm font-semibold text-white uppercase tracking-wide">
            Outcomes &amp; Odds
          </h2>
          {pool.odds.length === 0 ? (
            <PartialDataBanner
              message="No outcomes are published for this pool yet."
              className="bg-zinc-800/40 border-zinc-700 text-zinc-400"
            />
          ) : (
            <div className="space-y-2">
              {pool.odds.map((o) => (
                <div
                  key={o.outcome}
                  className="flex items-center justify-between rounded-lg border border-zinc-800 bg-zinc-800/40 px-4 py-3"
                >
                  <span className="text-sm text-zinc-200">
                    {outcomeLabel(o.outcome, pool.outcome_descriptions)}
                    {pool.result != null &&
                      Number(pool.result) === o.outcome && (
                        <span className="ml-2 text-xs font-medium text-emerald-400">
                          Winner
                        </span>
                      )}
                  </span>
                  <span className="text-sm font-medium text-white">
                    {o.odds.toFixed(2)}x
                    <span className="ml-2 text-zinc-500">
                      ({formatStake(o.stake, pool.token)})
                    </span>
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
