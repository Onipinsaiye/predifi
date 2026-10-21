import { Skeleton, SkeletonScreen, StatsSkeleton } from "@/components/ui";

/**
 * Route-level loading UI for /dashboard (#84).
 *
 * Mirrors the real layout — heading, metric row, chart row, activity row — so
 * there is no layout shift when the page hydrates, and wraps each block in an
 * accessible `SkeletonScreen` announcement.
 */
export default function Loading() {
  return (
    <div className="min-h-screen bg-[#0A0A0A] p-6 lg:p-8 space-y-8">
      <div className="space-y-2">
        <Skeleton className="h-8 w-56" />
        <Skeleton className="h-4 w-80 max-w-full" />
      </div>

      <StatsSkeleton count={4} />

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        <div className="lg:col-span-1">
          <SkeletonScreen label="Loading balance">
            <Skeleton className="h-[280px] sm:h-[300px] lg:h-[360px] w-full rounded-xl" />
          </SkeletonScreen>
        </div>
        <div className="lg:col-span-2">
          <SkeletonScreen label="Loading staked chart">
            <Skeleton className="h-[280px] sm:h-[300px] lg:h-[360px] w-full rounded-xl" />
          </SkeletonScreen>
        </div>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        <div className="lg:col-span-1">
          <SkeletonScreen label="Loading recent predictions">
            <Skeleton className="h-[300px] w-full rounded-xl" />
          </SkeletonScreen>
        </div>
        <div className="lg:col-span-2">
          <SkeletonScreen label="Loading active pools">
            <Skeleton className="h-[400px] w-full rounded-xl" />
          </SkeletonScreen>
        </div>
      </div>
    </div>
  );
}
