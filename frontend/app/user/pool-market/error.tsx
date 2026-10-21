"use client";

import Link from "next/link";
import { ArrowLeft } from "lucide-react";
import { RouteError } from "@/components/ui";

/**
 * Route-level error boundary for the pool list segment (#84).
 *
 * Scoped to /user/pool-market so a render error in the list does not unmount
 * the surrounding layout. Shares heading/retry semantics with the other core
 * routes via RouteError.
 */
export default function PoolMarketError({
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
        heading="Failed to load the pool market"
        description="Something went wrong while loading the pool list. You can try again or head back home."
        actions={
          <Link
            href="/"
            className="inline-flex items-center gap-2 rounded-xl border border-zinc-700 bg-zinc-800/50 px-5 py-2.5 text-sm font-medium text-white transition-all hover:bg-zinc-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-zinc-500"
          >
            <ArrowLeft className="h-4 w-4" aria-hidden="true" />
            Go home
          </Link>
        }
      />
    </div>
  );
}
