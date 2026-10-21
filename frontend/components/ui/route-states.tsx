"use client";

/**
 * Shared loading / empty / error / partial-data surfaces (#84).
 *
 * Every core route (pool list, pool detail, user history, dashboard) renders
 * the same four non-happy states. Centralising them here keeps the copy,
 * headings, and ARIA semantics identical everywhere instead of each route
 * inventing its own markup:
 *
 *   • EmptyState        — `role="status"` (polite). Carries the route-specific
 *                         heading so a screen reader hears *what* is empty.
 *   • ErrorState        — `role="alert"` (assertive). Always offers a retry.
 *   • PartialDataBanner — `role="status"` (polite). Used when some data is
 *                         available but part of the page is degraded/stale.
 *   • RouteError        — the body of a Next.js `error.tsx` boundary.
 *
 * Headings are rendered as real heading elements (`h2` by default) so the
 * document outline stays meaningful when a state replaces the content.
 */

import { useEffect, type ReactNode } from "react";
import { AlertTriangle, Inbox, RefreshCw } from "lucide-react";
import { cn } from "@/lib/utils";

type HeadingLevel = "h2" | "h3" | "h4";

interface StateHeadingProps {
  as: HeadingLevel;
  children: ReactNode;
}

function StateHeading({ as: Tag, children }: StateHeadingProps) {
  return <Tag className="text-base font-semibold text-white">{children}</Tag>;
}

const RETRY_BUTTON_CLASS =
  "inline-flex items-center gap-2 rounded-lg border border-[#37B7C3]/30 bg-[#37B7C3]/10 px-4 py-2 text-sm font-medium text-[#37B7C3] transition-colors hover:bg-[#37B7C3]/20 hover:border-[#37B7C3]/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#37B7C3]";

export interface EmptyStateProps {
  /** Route-specific heading, e.g. "No pools match your search". */
  heading: string;
  description?: ReactNode;
  /** Optional call-to-action (e.g. "Clear filters"). */
  action?: ReactNode;
  /** Pass `null` to omit the icon entirely. */
  icon?: ReactNode | null;
  headingAs?: HeadingLevel;
  className?: string;
}

/**
 * EmptyState — shown when a request succeeded but returned nothing to display.
 *
 * Announced politely (`role="status"`) because arriving at an empty list is a
 * state change the user should hear, but it is not urgent.
 */
export function EmptyState({
  heading,
  description,
  action,
  icon,
  headingAs = "h2",
  className,
}: EmptyStateProps) {
  return (
    <div
      role="status"
      aria-live="polite"
      className={cn(
        "flex flex-col items-center justify-center gap-3 rounded-xl border border-zinc-800 bg-zinc-900 px-6 py-16 text-center",
        className,
      )}
    >
      {icon !== null &&
        (icon ?? <Inbox className="h-8 w-8 text-zinc-600" aria-hidden="true" />)}
      <StateHeading as={headingAs}>{heading}</StateHeading>
      {description && (
        <p className="max-w-sm text-sm leading-relaxed text-zinc-500">
          {description}
        </p>
      )}
      {action}
    </div>
  );
}

export interface ErrorStateProps {
  /** Route-specific heading, e.g. "Couldn't load pools". */
  heading: string;
  description?: ReactNode;
  /** When provided, renders the retry button. */
  onRetry?: () => void;
  retryLabel?: string;
  /** Extra actions rendered next to the retry button (e.g. a back link). */
  actions?: ReactNode;
  /** Supplementary detail such as a development-only error message. */
  details?: ReactNode;
  headingAs?: HeadingLevel;
  className?: string;
}

/**
 * ErrorState — shown when a request failed and the user can act on it.
 *
 * Uses `role="alert"` so assistive tech interrupts to announce the failure,
 * and always exposes a retry affordance when `onRetry` is supplied.
 */
export function ErrorState({
  heading,
  description,
  onRetry,
  retryLabel = "Try again",
  actions,
  details,
  headingAs = "h2",
  className,
}: ErrorStateProps) {
  return (
    <div
      role="alert"
      className={cn(
        "flex flex-col items-center justify-center gap-3 rounded-xl border border-red-500/20 bg-red-500/5 px-6 py-16 text-center",
        className,
      )}
    >
      <AlertTriangle className="h-8 w-8 text-red-400" aria-hidden="true" />
      <StateHeading as={headingAs}>{heading}</StateHeading>
      {description && (
        <p className="max-w-sm text-sm leading-relaxed text-zinc-400">
          {description}
        </p>
      )}
      {details}
      {(onRetry || actions) && (
        <div className="mt-2 flex flex-col items-center gap-3 sm:flex-row">
          {onRetry && (
            <button type="button" onClick={onRetry} className={RETRY_BUTTON_CLASS}>
              <RefreshCw className="h-4 w-4" aria-hidden="true" />
              {retryLabel}
            </button>
          )}
          {actions}
        </div>
      )}
    </div>
  );
}

export interface PartialDataBannerProps {
  message: ReactNode;
  onRetry?: () => void;
  retryLabel?: string;
  className?: string;
}

/**
 * PartialDataBanner — shown when the page can still render data but part of it
 * is stale, cached, or degraded. Announced politely so it does not interrupt.
 */
export function PartialDataBanner({
  message,
  onRetry,
  retryLabel = "Retry",
  className,
}: PartialDataBannerProps) {
  return (
    <div
      role="status"
      aria-live="polite"
      className={cn(
        "flex flex-wrap items-center justify-between gap-3 rounded-lg border border-amber-500/20 bg-amber-500/10 px-3 py-2 text-xs text-amber-300",
        className,
      )}
    >
      <span>{message}</span>
      {onRetry && (
        <button
          type="button"
          onClick={onRetry}
          className="font-medium underline hover:text-amber-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-400 rounded"
        >
          {retryLabel}
        </button>
      )}
    </div>
  );
}

export interface RouteErrorProps {
  error: Error & { digest?: string };
  reset: () => void;
  heading: string;
  description?: ReactNode;
  /** Extra actions rendered alongside the reset button (e.g. a back link). */
  actions?: ReactNode;
}

/**
 * RouteError — the shared body for a Next.js `error.tsx` boundary.
 *
 * Keeps the "log, show a heading, offer reset" contract in one place so every
 * route-level boundary behaves the same. The raw error message is surfaced in
 * development only.
 */
export function RouteError({
  error,
  reset,
  heading,
  description,
  actions,
}: RouteErrorProps) {
  useEffect(() => {
    console.error("Route error:", error);
  }, [error]);

  return (
    <ErrorState
      heading={heading}
      description={description}
      onRetry={reset}
      actions={actions}
      className="min-h-[400px]"
      details={
        process.env.NODE_ENV === "development" && error.message ? (
          <p className="max-w-sm break-all rounded-md border border-red-500/20 bg-red-500/10 px-3 py-2 font-mono text-xs text-red-400">
            {error.message}
          </p>
        ) : undefined
      }
    />
  );
}
