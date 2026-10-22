"use client";

import { AlertTriangle, Wallet, XCircle, Clock, Coins, RefreshCw, X } from "lucide-react";
import { Button } from "@/components/ui";
import { cn } from "@/lib/utils";
import type { WalletError, WalletErrorCode } from "@/lib/walletErrors";

const ICONS: Record<WalletErrorCode, typeof AlertTriangle> = {
  NETWORK_MISMATCH: AlertTriangle,
  EXTENSION_NOT_INSTALLED: Wallet,
  USER_REJECTED: XCircle,
  TRANSACTION_TIMEOUT: Clock,
  INSUFFICIENT_BALANCE: Coins,
  STALE_SEQUENCE: RefreshCw,
  UNKNOWN: AlertTriangle,
};

interface WalletErrorBannerProps {
  error: WalletError;
  onRetry?: () => void;
  onDismiss?: () => void;
  /** Expected network name, e.g. "Testnet". Shown for NETWORK_MISMATCH. */
  expectedNetwork?: string;
  /** Contract environment label, e.g. "Testnet contract". Shown for NETWORK_MISMATCH. */
  contractEnvironment?: string;
  /** Re-checks the wallet network after the user switches. */
  onRecheckNetwork?: () => void;
  /** True while a network re-check is in flight. */
  isRechecking?: boolean;
  className?: string;
}

/**
 * Clear user-facing wallet error with recovery CTA.
 *
 * USER_REJECTED is treated as a neutral dismissal — the user intentionally
 * cancelled, so no error styling or `role="alert"` is used (#1771).
 */
export function WalletErrorBanner({
  error,
  onRetry,
  onDismiss,
  expectedNetwork,
  contractEnvironment,
  onRecheckNetwork,
  isRechecking = false,
  className,
}: WalletErrorBannerProps) {
  // A deliberate cancellation is not an error — render neutral copy only.
  if (error.code === "USER_REJECTED") {
    return (
      <div
        className={cn(
          "rounded-xl border border-zinc-800 bg-zinc-900/60 p-4 text-left",
          className
        )}
        data-testid="wallet-rejection-notice"
      >
        <div className="flex items-center justify-between gap-3">
          <p className="text-sm text-zinc-400">Request cancelled. Approve in your wallet to continue.</p>
          {onDismiss && (
            <button
              type="button"
              onClick={onDismiss}
              aria-label="Dismiss"
              className="shrink-0 rounded p-1 text-zinc-500 hover:text-zinc-300 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-zinc-500"
            >
              <X className="h-4 w-4" aria-hidden />
            </button>
          )}
        </div>
      </div>
    );
  }

  const Icon = ICONS[error.code];
  const isNetworkMismatch = error.code === "NETWORK_MISMATCH";
  // While the wallet is on the wrong network, retrying would submit against an
  // unintended contract deployment, so only the re-check action is offered.
  const showRetry = Boolean(onRetry) && !isNetworkMismatch;

  return (
    <div
      role="alert"
      className={cn(
        "rounded-xl border border-zinc-800 bg-zinc-900/90 p-4 text-left shadow-lg",
        className
      )}
    >
      <div className="flex gap-3">
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-amber-500/10">
          <Icon className="h-5 w-5 text-amber-400" aria-hidden />
        </div>
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold text-white">{error.title}</p>
          <p className="mt-1 text-sm text-zinc-400">{error.message}</p>
          <p className="mt-2 text-sm text-zinc-300">{error.recoveryAction}</p>
          {isNetworkMismatch && (expectedNetwork || contractEnvironment) && (
            <dl
              className="mt-3 space-y-1 rounded-lg border border-zinc-800 bg-zinc-950/60 p-3 text-sm"
              data-testid="network-mismatch-details"
            >
              {expectedNetwork && (
                <div className="flex justify-between gap-3">
                  <dt className="text-zinc-500">Expected network</dt>
                  <dd className="font-medium text-zinc-200">{expectedNetwork}</dd>
                </div>
              )}
              {contractEnvironment && (
                <div className="flex justify-between gap-3">
                  <dt className="text-zinc-500">Contract environment</dt>
                  <dd className="font-medium text-zinc-200">{contractEnvironment}</dd>
                </div>
              )}
            </dl>
          )}
          <div className="mt-3 flex flex-wrap gap-2">
            {isNetworkMismatch && onRecheckNetwork && (
              <Button
                type="button"
                size="small"
                onClick={onRecheckNetwork}
                disabled={isRechecking}
                aria-busy={isRechecking}
                data-testid="recheck-network-button"
              >
                {isRechecking ? "Re-checking…" : "Re-check network"}
              </Button>
            )}
            {showRetry && (
              <Button type="button" size="small" onClick={onRetry}>
                Try again
              </Button>
            )}
            {error.code === "EXTENSION_NOT_INSTALLED" && (
              <Button
                type="button"
                size="small"
                variant="secondary"
                onClick={() =>
                  window.open("https://freighter.app", "_blank", "noopener,noreferrer")
                }
              >
                Get Freighter
              </Button>
            )}
            {onDismiss && (
              <Button type="button" size="small" variant="ghost" onClick={onDismiss}>
                Dismiss
              </Button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
