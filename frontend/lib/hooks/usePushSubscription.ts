"use client";

/**
 * usePushSubscription — opt-in browser push for market events (#288).
 *
 * Manages the full browser push permission and registration lifecycle:
 * 1. Permission is only requested after a deliberate user action (never on mount).
 * 2. Subscriptions are wallet-scoped: switching wallets revokes the old
 *    subscription and registers a new one for the new wallet.
 * 3. Expired or revoked subscriptions are pruned on registration.
 * 4. Subscription rotation (browser cycling keys) is handled by passing the
 *    old endpoint as `previous_endpoint` on the new registration.
 * 5. Payloads sent to the backend contain no sensitive balance data — only
 *    the VAPID endpoint and encryption keys.
 *
 * Requirements: the host page must register a service worker at
 * `/service-worker.js` that handles `push` events.
 */

import { useState, useCallback, useEffect, useRef } from "react";
import {
  savePushSubscription,
  revokePushSubscription,
  pushSubscriptionToRequest,
} from "@/lib/api/notifications";

export type PushPermissionState = "default" | "granted" | "denied" | "unsupported";

export interface UsePushSubscriptionResult {
  /** Current browser permission state. */
  permission: PushPermissionState;
  /** True when a subscription is active for the connected wallet. */
  isSubscribed: boolean;
  /** True while a subscribe/unsubscribe operation is in flight. */
  isLoading: boolean;
  /** Last error message, or null. */
  error: string | null;
  /**
   * Request permission and register a push subscription.
   * Must be called from a user-initiated event handler (button click etc.)
   * so the browser allows the permission prompt.
   */
  subscribe: (vapidPublicKey: string) => Promise<void>;
  /** Unsubscribe and revoke the subscription on the backend. */
  unsubscribe: () => Promise<void>;
}

function urlBase64ToUint8Array(base64String: string): Uint8Array {
  const padding = "=".repeat((4 - (base64String.length % 4)) % 4);
  const base64 = (base64String + padding).replace(/-/g, "+").replace(/_/g, "/");
  const rawData = atob(base64);
  return Uint8Array.from([...rawData].map((char) => char.charCodeAt(0)));
}

function readPermission(): PushPermissionState {
  if (typeof window === "undefined" || !("Notification" in window)) return "unsupported";
  if (!("serviceWorker" in navigator) || !("PushManager" in window)) return "unsupported";
  return Notification.permission as PushPermissionState;
}

export function usePushSubscription(
  address: string | undefined,
): UsePushSubscriptionResult {
  const [permission, setPermission] = useState<PushPermissionState>(readPermission);
  const [isSubscribed, setIsSubscribed] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Track the active PushSubscription object for rotation detection.
  const activeSub = useRef<PushSubscription | null>(null);
  const addressRef = useRef(address);

  // Revoke and clear when wallet changes.
  useEffect(() => {
    if (addressRef.current === address) return;
    addressRef.current = address;

    // Revoke old subscription silently — the previous wallet is gone.
    if (activeSub.current && addressRef.current) {
      void revokePushSubscription(addressRef.current, activeSub.current.endpoint).catch(
        () => {
          // Best-effort: backend prunes expired/revoked subs automatically.
        },
      );
    }
    activeSub.current = null;
    setIsSubscribed(false);
    setError(null);
  }, [address]);

  const subscribe = useCallback(
    async (vapidPublicKey: string) => {
      if (!address) {
        setError("No wallet connected.");
        return;
      }
      if (permission === "unsupported") {
        setError("Push notifications are not supported in this browser.");
        return;
      }
      if (permission === "denied") {
        setError(
          "Push notification permission was denied. Please enable it in your browser settings.",
        );
        return;
      }

      setIsLoading(true);
      setError(null);

      try {
        // 1. Register (or reuse) a service worker.
        const registration = await navigator.serviceWorker.register("/service-worker.js");
        await navigator.serviceWorker.ready;

        // 2. Request permission — only proceeds if the user grants it.
        const result = await Notification.requestPermission();
        setPermission(result as PushPermissionState);
        if (result !== "granted") {
          setError("Push notification permission was not granted.");
          return;
        }

        // 3. Subscribe via PushManager.
        const previousEndpoint = activeSub.current?.endpoint;
        const sub = await registration.pushManager.subscribe({
          userVisibleOnly: true,
          applicationServerKey: urlBase64ToUint8Array(vapidPublicKey),
        });

        // 4. Register with the backend, passing the old endpoint for rotation.
        const request = pushSubscriptionToRequest(sub, previousEndpoint);
        await savePushSubscription(address, request);

        activeSub.current = sub;
        setIsSubscribed(true);
      } catch (err) {
        const message =
          err instanceof Error ? err.message : "Failed to subscribe to push notifications.";
        setError(message);
        setIsSubscribed(false);
      } finally {
        setIsLoading(false);
      }
    },
    [address, permission],
  );

  const unsubscribe = useCallback(async () => {
    if (!address) return;
    setIsLoading(true);
    setError(null);

    try {
      const endpoint = activeSub.current?.endpoint;

      // Unsubscribe from the browser first so the browser stops sending push events.
      if (activeSub.current) {
        await activeSub.current.unsubscribe();
        activeSub.current = null;
      }

      // Revoke on the backend (removes stored encrypted payload).
      await revokePushSubscription(address, endpoint);

      setIsSubscribed(false);
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Failed to unsubscribe from push notifications.";
      setError(message);
    } finally {
      setIsLoading(false);
    }
  }, [address]);

  return {
    permission,
    isSubscribed,
    isLoading,
    error,
    subscribe,
    unsubscribe,
  };
}
