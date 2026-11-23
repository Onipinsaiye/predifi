"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import {
  createCountdownTimer,
  formatCountdown,
  formatUtcDateTime,
  isPast,
  remainingMs,
  systemClock,
  type Clock,
} from "@/lib/date";

/**
 * Server-authoritative close-time countdown.
 *
 * Derives the display offset from a server timestamp so browser clock drift
 * cannot make the countdown disagree with the backend or contract close
 * boundary. Shows the absolute close time and timezone alongside the
 * countdown, and stops the countdown at the authoritative boundary.
 *
 * The `serverNow` clock is injected so tests can drive deterministic clock
 * skew (positive, negative) and tab-resume scenarios.
 */
export interface ServerCountdownProps {
  /** Pool close time as a Unix timestamp (seconds), from the chain/contract. */
  closeTime: number;
  /** Server timestamp (milliseconds) used as the authoritative "now". */
  serverNow: number;
  /** Optional callback fired when the countdown reaches zero. */
  onExpire?: () => void;
  /** Accessibility label. */
  ariaLabel?: string;
  /** Additional className. */
  className?: string;
  /** Poll interval in ms for refreshing the display. Defaults to 1000. */
  pollInterval?: number;
}

function formatRemaining(seconds: number): string {
  const totalSeconds = Math.max(0, Math.floor(seconds));
  const days = Math.floor(totalSeconds / 86_400);
  const hours = Math.floor((totalSeconds % 86_400) / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  const secs = totalSeconds % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  const clock = `${pad(hours)}:${pad(minutes)}:${pad(secs)}`;
  return days > 0 ? `${days}d ${clock}` : clock;
}

export function ServerCountdown({
  closeTime,
  serverNow,
  onExpire,
  ariaLabel = "Time until market close",
  className,
  pollInterval = 1_000,
}: ServerCountdownProps) {
  const closeTimeMs = closeTime * 1000;

  // Clock derived from the server timestamp. We advance it locally between
  // ticks so the countdown keeps ticking without needing constant re-fetches,
  // but the base is always anchored to serverNow.
  const [nowMs, setNowMs] = useState(serverNow);
  const timerRef = useRef<ReturnType<typeof createCountdownTimer> | null>(
    null,
  );
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const expiredRef = useRef(false);

  // Recreate the timer whenever serverNow or closeTime changes.
  const memoizedClock = useCallback<Clock>(() => nowMs, [nowMs]);
  const memoizedTimer = useMemo(
    () =>
      createCountdownTimer(closeTimeMs, memoizedClock),
    [closeTimeMs, memoizedClock],
  );
  timerRef.current = memoizedTimer;

  // Start / restart the ticking interval.
  useEffect(() => {
    if (intervalRef.current) {
      clearInterval(intervalRef.current);
    }
    intervalRef.current = setInterval(() => {
      setNowMs((prev) => prev + pollInterval);
    }, pollInterval);
    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
      }
    };
  }, [pollInterval]);

  // Sync nowMs to serverNow when the server timestamp changes (e.g. refetch).
  useEffect(() => {
    setNowMs(serverNow);
  }, [serverNow]);

  // Detect expiry transition.
  const expired = isPast(closeTimeMs, nowMs);
  useEffect(() => {
    if (expired && !expiredRef.current) {
      expiredRef.current = true;
      onExpire?.();
    }
    if (!expired) {
      expiredRef.current = false;
    }
  }, [expired, onExpire]);

  const remaining = timerRef.current.remaining();
  const formatted = timerRef.current.formatted();
  const hasExpired = remaining === null || remaining <= 0;

  const closeDate = new Date(closeTimeMs);
  const closeTimeString = closeDate.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    timeZoneName: "short",
  });
  const closeDateString = closeDate.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });

  return (
    <div
      className={cn(
        "flex items-center gap-2 font-mono text-sm",
        hasExpired && "text-zinc-500",
        className,
      )}
      aria-live="polite"
      aria-label={ariaLabel}
    >
      {hasExpired ? (
        <>
          <span className="text-zinc-500">Closed</span>
          <span className="text-zinc-500" aria-hidden="true">
            ·
          </span>
          <span className="text-zinc-400">
            {closeDateString} {closeTimeString}
          </span>
        </>
      ) : (
        <>
          <span className="text-white tabular-nums">{formatted}</span>
          <span className="text-zinc-500" aria-hidden="true">
            ·
          </span>
          <span className="text-zinc-400">
            closes {closeDateString} {closeTimeString}
          </span>
        </>
      )}
    </div>
  );
}
