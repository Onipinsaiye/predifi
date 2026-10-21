"use client";

import Link from "next/link";
import { ArrowLeft } from "lucide-react";
import { RouteError } from "@/components/ui";

/**
 * Route-level error boundary for the pool detail segment (#1764, #84).
 *
 * Scoped to /user/pool-market/[id] so a render error inside a pool page does
 * not unmount the surrounding layout or navigation. The root app/error.tsx
 * still catches anything above this boundary.
 *
 * The shared RouteError keeps the heading, `role="alert"` semantics, and
 * retry affordance identical to the other core routes.
 */
export default function PoolDetailError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  return (
    <RouteError
      error={error}
      reset={reset}
      heading="Failed to load pool"
      description="Something went wrong while loading this prediction pool. You can try again or browse other pools."
      actions={
        <Link
          href="/user/pool-market"
          className="inline-flex items-center gap-2 rounded-xl border border-zinc-700 bg-zinc-800/50 px-5 py-2.5 text-sm font-medium text-white transition-all hover:bg-zinc-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-zinc-500"
        >
          <ArrowLeft className="h-4 w-4" aria-hidden="true" />
          Back to pools
        </Link>
      }
    />
  );
}
