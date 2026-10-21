"use client";

import Link from "next/link";
import { ArrowLeft, RefreshCcw } from "lucide-react";
import { RouteError } from "@/components/ui";

/**
 * Route-level error boundary for /dashboard (#84).
 *
 * Uses the shared RouteError so the heading, alert semantics, and retry
 * behaviour match the other core routes. The retry preserves the current
 * route context by re-rendering the same segment via the boundary reset.
 */
export default function DashboardError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  return (
    <div className="min-h-screen bg-[#0A0A0A] p-6 lg:p-8">
      <RouteError
        error={error}
        reset={reset}
        heading="Failed to load your dashboard"
        description="Something went wrong while loading your dashboard. You can try again or head back to the pool market."
        actions={(
          <>
            <button
              type="button"
              onClick={reset}
              className="inline-flex items-center gap-2 rounded-xl border border-zinc-700 bg-zinc-800/50 px-5 py-2.5 text-sm font-medium text-white transition-all hover:bg-zinc-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-zinc-500"
            >
              <RefreshCcw className="h-4 w-4" aria-hidden="true" />
              Try again
            </button>
            <Link
              href="/user/pool-market"
              className="inline-flex items-center gap-2 rounded-xl border border-zinc-700 bg-zinc-800/50 px-5 py-2.5 text-sm font-medium text-white transition-all hover:bg-zinc-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-zinc-500"
            >
              <ArrowLeft className="h-4 w-4" aria-hidden="true" />
              Back to pools
            </Link>
          </>
        )}
      />
    </div>
  );
}