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

import { useCallback, useRef, useState } from "react";
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
  PricingUnitsLegend,
  ShareButton,
} from "@/components/ui";
import { deriveOutcomePricing } from "@/lib/probabilityUnits";
import { sanitizeRichText } from "@/lib/security";
import { useDemoMode } from "@/lib/demo/DemoModeProvider";
import { demoPoolDetail } from "@/lib/demo/fixtures";
import { useMarketEvents, type MarketEvent } from "@/lib/hooks/useMarketEvents";

const SITE_URL = "https://predifi.app";

function formatStake(amount: number, token: string): string {
  return `${amount.toLocaleString()} ${token}`;
}

function outcomeLabel(outcome: number, labels?: string[]): string {
  return sanitizeRichText(labels?.[outcome] ?? `Outcome ${outcome + 1}`);
}

const EVENT_LABELS: Record<MarketEvent["type"], string> = {
  creation: "Market created",
  trade: "Trade",
  close: "Market closed",
  resolution: "Market resolved",
  claim: "Winnings claimed",
};

function formatEventTimestamp(timestamp: string | number | undefined): string {
  if (timestamp === undefined || timestamp === null || timestamp === "") {
    return "Timestamp unavailable";
  }
  const date =
    typeof timestamp === "number" ? new Date(timestamp) : new Date(timestamp);
  if (Number.isNaN(date.getTime())) {
    return "Timestamp unavailable";
  }
  return date.toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

function eventKey(event: MarketEvent): string {
  return event.id;
}

export interface PoolDetailClientProps {
  poolId: string;
  /** Server-rendered pool used as SWR fallback data for the first paint. */
  initialPool?: PoolDetail | null;
}

export function PoolDetailClient({ poolId, initialPool }: PoolDetailClientProps) {
  const { isDemoMode, exitDemoMode } = useDemoMode();

  // In demo mode the SWR key is null: no request is ever issued to the pool
  // API, so demo exploration can never be mistaken for (or leak into) live
  // market data. All demo values come from local fixtures.
  const { data, error, isLoading, mutate } = useSWR<PoolDetail>(
    isDemoMode ? null : poolDetailUrl(poolId),
    fetchPoolDetail,
    { fallbackData: isDemoMode ? undefined : (initialPool ?? undefined) },
  );

  // Dialog state + focus restoration. Each dialog remembers the control that
  // opened it so focus can be returned there on close (keyboard journeys).
  const [activeDialog, setActiveDialog] = useState<"trade" | "claim" | null>(
    null,
  );
  const [announcement, setAnnouncement] = useState("");
  const tradeTriggerRef = useRef<HTMLButtonElement | null>(null);
  const claimTriggerRef = useRef<HTMLButtonElement | null>(null);

  const openDialog = useCallback(
    (dialog: "trade" | "claim") => {
      // Wallet signing is prohibited from demo state: never open the
      // trade/claim dialogs while exploring illustrative data.
      if (isDemoMode) {
        setAnnouncement(
          "Demo mode is read-only. Exit demo mode to trade or claim.",
        );
        return;
      }
      setActiveDialog(dialog);
      setAnnouncement(
        dialog === "trade"
          ? "Trade confirmation dialog opened."
          : "Claim dialog opened.",
      );
    },
    [isDemoMode],
  );

  const closeDialog = useCallback(() => {
    const trigger =
      activeDialog === "trade" ? tradeTriggerRef.current : claimTriggerRef.current;
    setActiveDialog(null);
    setAnnouncement(
      activeDialog === "trade"
        ? "Trade confirmation dialog closed."
        : "Claim dialog closed.",
    );
    // Restore focus to the triggering control after the dialog unmounts.
    trigger?.focus();
  }, [activeDialog]);

  const {
    events,
    isLoading: eventsLoading,
    error: eventsError,
    mutate: mutateEvents,
  } = useMarketEvents(poolId);

  const pool = isDemoMode
    ? demoPoolDetail(poolId)
    : (data ?? initialPool ?? undefined);
  const isStale = !isDemoMode && Boolean(error && pool);
  const isMissing =
    !isDemoMode &&
    !pool &&
    (error instanceof ApiError ? error.status === 404 : false);

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
  const description = sanitizeRichText(pool.description);

  return (
    <div className="min-h-screen bg-[#0A0A0A] p-6 lg:p-8">
      <div className="mx-auto max-w-3xl space-y-6">
        {/* Live region: announces dialog open/close and wallet-state changes. */}
        <div
          role="status"
          aria-live="polite"
          aria-atomic="true"
          className="sr-only"
        >
          {announcement}
        </div>

        {isDemoMode && (
          <div
            role="note"
            aria-label="Illustrative data"
            className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-amber-500/40 bg-amber-500/10 px-4 py-3"
          >
            <p className="text-sm text-amber-200">
              <span className="font-semibold">Illustrative data.</span> You are
              exploring a read-only demo. These values are not live markets and
              no wallet signing is possible.
            </p>
            <button
              type="button"
              onClick={exitDemoMode}
              className="inline-flex items-center gap-2 rounded-lg border border-amber-500/50 bg-amber-500/20 px-3 py-1.5 text-sm font-medium text-amber-100 transition-colors hover:bg-amber-500/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-400"
            >
              Exit demo — view live data
            </button>
          </div>
        )}

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

        {description && <p className="text-sm text-zinc-400">{description}</p>}

        <div className="flex flex-wrap items-center gap-2">
          <button
            ref={tradeTriggerRef}
            type="button"
            onClick={() => openDialog("trade")}
            aria-haspopup="dialog"
            aria-expanded={activeDialog === "trade"}
            aria-disabled={isDemoMode || undefined}
            title={
              isDemoMode
                ? "Unavailable in demo mode — exit demo to trade"
                : undefined
            }
            className="inline-flex items-center gap-2 rounded-lg bg-[#7DE3EC] px-4 py-2 text-sm font-semibold text-black transition-colors hover:bg-[#5fd0d8] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#7DE3EC] focus-visible:ring-offset-2 focus-visible:ring-offset-[#0A0A0A] disabled:cursor-not-allowed disabled:opacity-50"
          >
            Confirm trade
          </button>
          <button
            ref={claimTriggerRef}
            type="button"
            onClick={() => openDialog("claim")}
            aria-haspopup="dialog"
            aria-expanded={activeDialog === "claim"}
            aria-disabled={isDemoMode || undefined}
            title={
              isDemoMode
                ? "Unavailable in demo mode — exit demo to claim"
                : undefined
            }
            className="inline-flex items-center gap-2 rounded-lg border border-zinc-700 bg-zinc-800/50 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-zinc-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-zinc-500 disabled:cursor-not-allowed disabled:opacity-50"
          >
            Claim winnings
          </button>
        </div>

        {activeDialog && (
          <div
            role="dialog"
            aria-modal="true"
            aria-labelledby="pool-dialog-title"
            className="rounded-xl border border-zinc-700 bg-zinc-900 p-5 space-y-3"
          >
            <h2
              id="pool-dialog-title"
              className="text-sm font-semibold text-white uppercase tracking-wide"
            >
              {activeDialog === "trade"
                ? "Confirm trade"
                : "Claim winnings"}
            </h2>
            <p className="text-sm text-zinc-400">
              {activeDialog === "trade"
                ? "Review your position before submitting the trade."
                : "Review the outcome before claiming your winnings."}
            </p>
            {isDemoMode && (
              <p className="text-sm text-amber-300">
                Illustrative data — wallet signing is disabled in demo mode.
              </p>
            )}
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={closeDialog}
                className="inline-flex items-center gap-2 rounded-lg border border-zinc-700 bg-zinc-800/50 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-zinc-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-zinc-500"
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        <section
          aria-labelledby="pool-activity-heading"
          className="rounded-xl border border-zinc-800 bg-zinc-900 p-5 space-y-3"
        >
          <h2
            id="pool-activity-heading"
            className="text-sm font-semibold text-white uppercase tracking-wide"
          >
            Activity
          </h2>
          {eventsLoading && events.length === 0 ? (
            <p role="status" className="text-sm text-zinc-400">
              Loading activity…
            </p>
          ) : eventsError && events.length === 0 ? (
            <div role="alert" className="space-y-2">
              <p className="text-sm text-zinc-400">
                Couldn&apos;t load this market&apos;s activity.
              </p>
              <button
                type="button"
                onClick={() => void mutateEvents()}
                className="inline-flex items-center gap-2 rounded-lg border border-zinc-700 bg-zinc-800/50 px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-zinc-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-zinc-500"
              >
                Retry activity
              </button>
            </div>
          ) : events.length === 0 ? (
            <p className="text-sm text-zinc-400">
              No activity has been recorded for this market yet.
            </p>
          ) : (
            <ol className="space-y-2">
              {events.map((event) => (
                <li
                  key={eventKey(event)}
                  className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-zinc-800 bg-zinc-800/40 px-4 py-3"
                >
                  <div className="space-y-0.5">
                    <p className="text-sm text-zinc-200">
                      {EVENT_LABELS[event.type] ?? event.type}
                      {event.outcome != null && (
                        <span className="ml-2 text-xs text-zinc-400">
                          {outcomeLabel(
                            event.outcome,
                            pool.outcome_descriptions,
                          )}
                        </span>
                      )}
                    </p>
                    <p className="text-xs text-zinc-500">
                      <time dateTime={event.timestamp || undefined}>
                        {formatEventTimestamp(event.timestamp)}
                      </time>
                    </p>
                  </div>
                  {event.txHash ? (
                    <a
                      href={`https://stellar.expert/explorer/public/tx/${event.txHash}`}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-xs font-medium text-[#7DE3EC] underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#7DE3EC] focus-visible:ring-offset-2 focus-visible:ring-offset-[#0A0A0A] rounded"
                    >
                      View transaction
                      <span className="sr-only">
                        {" "}
                        for {EVENT_LABELS[event.type] ?? event.type}
                      </span>
                    </a>
                  ) : (
                    <span className="text-xs text-zinc-500">
                      Transaction unavailable
                    </span>
                  )}
                </li>
              ))}
            </ol>
          )}
        </section>

        <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-5 space-y-3">
          <h2 className="text-sm font-semibold text-white uppercase tracking-wide">
            Outcomes &amp; Odds
            {isDemoMode && (
              <span className="ml-2 rounded bg-amber-500/20 px-2 py-0.5 text-xs font-medium normal-case tracking-normal text-amber-200">
                Illustrative data
              </span>
            )}
          </h2>
          {pool.odds.length === 0 ? (
            <PartialDataBanner
              message="No outcomes are published for this pool yet."
              className="bg-zinc-800/40 border-zinc-700 text-zinc-400"
            />
          ) : (
            <div className="space-y-3">
              <div className="space-y-2">
                {pool.odds.map((o) => {
                  const pricing = deriveOutcomePricing(
                    o.stake,
                    pool.total_stake,
                    pool.token,
                  );
                  return (
                    <div
                      key={o.outcome}
                      className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-zinc-800 bg-zinc-800/40 px-4 py-3"
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
                      <div className="flex flex-wrap items-center gap-3 text-right">
                        <span
                          aria-label={pricing.probability.ariaLabel}
                          className="text-xs font-mono text-[#7DE3EC]"
                        >
                          {pricing.probability.display}
                        </span>
                        <span
                          aria-label={pricing.sharePrice.ariaLabel}
                          className="text-xs font-mono text-zinc-400"
                        >
                          {pricing.sharePrice.status === "ok"
                            ? `${pricing.sharePrice.display}/share`
                            : pricing.sharePrice.display}
                        </span>
                        <span className="text-sm font-medium text-white">
                          {o.odds.toFixed(2)}x
                          <span className="ml-2 text-zinc-500">
                            ({formatStake(o.stake, pool.token)})
                          </span>
                        </span>
                      </div>
                    </div>
                  );
                })}
              </div>
              <PricingUnitsLegend token={pool.token} />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
