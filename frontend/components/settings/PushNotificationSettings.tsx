"use client";

/**
 * PushNotificationSettings (#288)
 *
 * Opt-in browser push subscription management. Permission is only requested
 * after a deliberate button click (never automatically). Subscriptions are
 * wallet-bound and can be revoked at any time.
 */

import { Bell, BellOff } from "lucide-react";
import { Button } from "@/components/ui";
import { usePushSubscription } from "@/lib/hooks/usePushSubscription";
import { useWalletAddress } from "@/lib/context";

/** VAPID public key — set via environment variable. */
const VAPID_PUBLIC_KEY = process.env.NEXT_PUBLIC_VAPID_PUBLIC_KEY ?? "";

export function PushNotificationSettings() {
  const address = useWalletAddress();
  const { permission, isSubscribed, isLoading, error, subscribe, unsubscribe } =
    usePushSubscription(address);

  if (!address) {
    return (
      <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-6">
        <p className="text-sm text-zinc-400">Connect a wallet to manage browser push notifications.</p>
      </div>
    );
  }

  if (permission === "unsupported") {
    return (
      <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-6">
        <p className="text-sm text-zinc-400">
          Browser push notifications are not supported in this browser or context.
        </p>
      </div>
    );
  }

  return (
    <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-6 space-y-4">
      <div className="space-y-1">
        <h3 className="text-sm font-semibold text-white">Browser push notifications</h3>
        <p className="text-xs text-zinc-500">
          Receive time-sensitive market alerts outside the active tab.
          No sensitive account data is included in push payloads.
        </p>
      </div>

      <div className="flex items-center gap-3">
        {isSubscribed ? (
          <>
            <span className="flex items-center gap-1.5 text-xs text-emerald-400">
              <Bell className="h-3.5 w-3.5" aria-hidden="true" />
              Push notifications enabled
            </span>
            <Button
              type="button"
              variant="outline"
              size="small"
              onClick={unsubscribe}
              disabled={isLoading}
            >
              <BellOff className="mr-1.5 h-3.5 w-3.5" aria-hidden="true" />
              {isLoading ? "Revoking…" : "Disable"}
            </Button>
          </>
        ) : (
          <>
            {permission === "denied" ? (
              <p className="text-xs text-amber-400">
                Push permission is blocked. Enable it in your browser settings to continue.
              </p>
            ) : (
              <Button
                type="button"
                size="small"
                onClick={() => subscribe(VAPID_PUBLIC_KEY)}
                disabled={isLoading || !VAPID_PUBLIC_KEY}
              >
                <Bell className="mr-1.5 h-3.5 w-3.5" aria-hidden="true" />
                {isLoading ? "Enabling…" : "Enable push notifications"}
              </Button>
            )}
          </>
        )}
      </div>

      {error && (
        <p className="text-xs text-red-400" role="alert">
          {error}
        </p>
      )}

      {isSubscribed && (
        <p className="text-xs text-zinc-600">
          Your subscription is bound to your connected wallet. It will be automatically
          removed if you disconnect or switch wallets.
        </p>
      )}
    </div>
  );
}
