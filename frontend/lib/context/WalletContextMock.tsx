"use client";

/**
 * WalletContext — single source of truth for the connected wallet address.
 *
 * Why a context instead of calling useWalletConnection() everywhere?
 * ─────────────────────────────────────────────────────────────────
 * • One connection object for the entire component tree — no duplicate wallet
 *   state across siblings.
 * • Eliminates prop-drilling: NotificationBell, ProfileHeader, and any
 *   future auth-gated UI all read from context, not from props passed through
 *   multiple intermediate layers.
 * • Replaces the hardcoded placeholder address in useCurrentUserAddress with
 *   the real connect/disconnect flow (issue #1406).
 * • Cache invalidation: on disconnect the SWR keys that include the address
 *   (notifications, profile) can be invalidated in one place.
 *
 * Usage
 * ─────
 *   // In app/layout.tsx — already wraps SWRProvider / NetworkGuardProvider:
 *   <WalletProvider>…</WalletProvider>
 *
 *   // In any client component:
 *   const { address, connect, disconnect, isConnecting, error } = useWallet();
 *
 *   // When only the address is needed (avoids re-rendering on isConnecting):
 *   const address = useWalletAddress();
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  type ReactNode,
} from "react";
import { useSWRConfig } from "swr";
import {
  useWalletConnection,
  type WalletConnectionState,
} from "@/lib/hooks/useWalletConnection";
import { useNetwork } from "@/lib/context/NetworkContext";

// ── Context ───────────────────────────────────────────────────────────────────

/**
 * Exported so useCurrentUserAddress can consume it directly without going
 * through the barrel (avoids a circular import).
 */
export const WalletContext = createContext<WalletConnectionState | undefined>(
  undefined,
);
WalletContext.displayName = "WalletContext";

// ── Provider ──────────────────────────────────────────────────────────────────

export function WalletProvider({ children }: { children: ReactNode }) {
  const wallet = useWalletConnection();
  const { mutate } = useSWRConfig();
  const { network } = useNetwork();

  // Identity = network + address. Account-scoped cache entries are keyed by
  // this identity so data from one wallet/network is never reused for another.
  const identityKey = useMemo(
    () => `${network}:${wallet.address ?? ""}`,
    [network, wallet.address],
  );

  // Bumped on every identity transition so consumers can reset local
  // account-bound state (balances, history, pending UI) in one place.
  const generationRef = useRef(0);
  const prevIdentityRef = useRef<string>(identityKey);

  // On any identity change (connect, disconnect, account switch, network
  // switch) drop all cached account data so nothing from the previous
  // identity flashes or is reused.
  useEffect(() => {
    const prev = prevIdentityRef.current;
    if (prev !== identityKey) {
      generationRef.current += 1;
      // Clear every cached key — the simplest correct strategy. SWR will
      // refetch any key that is currently mounted.
      void mutate(() => true, undefined, { revalidate: false });
      prevIdentityRef.current = identityKey;
    }
  }, [identityKey, mutate]);

  // Wrap disconnect to also clear the cache immediately on user action,
  // without waiting for the useEffect to run on the next render cycle.
  const disconnect = useCallback(() => {
    wallet.disconnect();
    generationRef.current += 1;
    void mutate(() => true, undefined, { revalidate: false });
  }, [wallet, mutate]);

  // Keep the context value object reference stable when wallet state is
  // unchanged — prevents all consumers from re-rendering on unrelated updates.
  const value = useMemo<WalletConnectionState>(
    () => ({
      address: wallet.address,
      isConnecting: wallet.isConnecting,
      error: wallet.error,
      requiredNetworkLabel: wallet.requiredNetworkLabel,
      connect: wallet.connect,
      disconnect,
      clearError: wallet.clearError,
      identityKey,
      generation: generationRef.current,
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [
      wallet.address,
      wallet.isConnecting,
      wallet.error,
      wallet.requiredNetworkLabel,
      wallet.connect,
      disconnect,
      wallet.clearError,
      identityKey,
    ],
  );

  return (
    <WalletContext.Provider value={value}>{children}</WalletContext.Provider>
  );
}

// ── Hooks ─────────────────────────────────────────────────────────────────────

/**
 * useWallet — full wallet state from context.
 *
 * Throws a descriptive error when called outside <WalletProvider> so
 * misconfiguration surfaces immediately in development.
 */
export function useWallet(): WalletConnectionState {
  const ctx = useContext(WalletContext);
  if (ctx === undefined) {
    throw new Error(
      "useWallet must be used within a <WalletProvider>. " +
        "Add <WalletProvider> to your root layout.",
    );
  }
  return ctx;
}

/**
 * useWalletAddress — returns only the connected address (or `undefined`).
 *
 * Prefer this over useWallet() in components that only need the address so
 * they skip re-renders triggered by `isConnecting` or `error` changes.
 */
export function useWalletAddress(): string | undefined {
  const { address } = useWallet();
  return address ?? undefined;
}

// Re-export the state type so consumers don't need to import from the hook.
export type { WalletConnectionState };
