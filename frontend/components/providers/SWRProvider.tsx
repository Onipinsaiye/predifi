"use client";

import { SWRConfig } from "swr";
import type { ReactNode } from "react";
import { useWallet } from "./WalletProvider";

/**
 * SWRProvider — app-wide SWR configuration.
 *
 * Defaults are tuned for the largely-static data PrediFi displays (notably
 * prediction-pool metadata): responses are cached and deduplicated, and we
 * avoid aggressive revalidation that would refetch data the user already has.
 * Individual hooks can still override any of these per call.
 *
 * Cache scoping: the SWR cache map is keyed by the current wallet identity
 * (network + address). When the identity changes (account switch, network
 * switch, or disconnect) a fresh, empty cache map is used, so account-specific
 * balances, history, and cached requests from the previous user are never
 * reused or flashed. `keepPreviousData` is disabled for the same reason.
 */

type CacheMap = Map<string, unknown>;

const cacheMaps = new Map<string, CacheMap>();

function getCacheMap(identityKey: string): CacheMap {
  let map = cacheMaps.get(identityKey);
  if (!map) {
    map = new Map<string, unknown>();
    cacheMaps.set(identityKey, map);
  }
  return map;
}

function makeScopedProvider(identityKey: string) {
  const map = getCacheMap(identityKey);
  return () => map;
}

/**
 * Resolve the identity key from the wallet context. Falls back to a stable
 * "anonymous" key when no wallet is connected so disconnected users share a
 * single empty-scoped cache.
 */
function useIdentityKey(): string {
  const wallet = useWallet() as
    | { address?: string | null; chainId?: number | string | null }
    | undefined;

  const address = wallet?.address ?? null;
  const chainId = wallet?.chainId ?? null;

  if (!address) return "anonymous";
  return `${String(chainId ?? "unknown")}:${address.toLowerCase()}`;
}

export function SWRProvider({ children }: { children: ReactNode }) {
  const identityKey = useIdentityKey();

  return (
    <SWRConfig
      key={identityKey}
      value={{
        // Scope the cache map by identity so a new account/network starts with
        // an empty cache instead of reusing the previous user's entries.
        provider: makeScopedProvider(identityKey),
        // Treat fetched data as fresh for 60s: identical keys requested within
        // this window reuse the in-flight/last response instead of refetching.
        dedupingInterval: 60_000,
        // Static data doesn't change while the user tabs away and back.
        revalidateOnFocus: false,
        // Avoid a refetch storm when connectivity flaps.
        revalidateOnReconnect: false,
        // Never show the previous identity's data while a new query loads.
        keepPreviousData: false,
        // Retry transient network/server errors a couple of times.
        errorRetryCount: 2,
      }}
    >
      {children}
    </SWRConfig>
  );
}