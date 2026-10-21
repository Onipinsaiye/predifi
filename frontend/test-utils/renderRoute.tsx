/**
 * Render helper for core-route component tests (#84).
 *
 * Wraps the component under test in the same providers the app mounts at the
 * root (SWR, wallet, toasts, live region) but with a *fresh, isolated SWR
 * cache per test* so state does not leak between cases. Also exposes helpers
 * for the two states that are hard to reach otherwise: holding a request open
 * (loading) and seeding a stale cache entry (partial data).
 */

import type { ReactElement, ReactNode } from "react";
import { render, type RenderResult } from "@testing-library/react";
import { SWRConfig } from "swr";
import { WalletContext } from "@/lib/context/WalletContext";
import type { WalletConnectionState } from "@/lib/hooks/useWalletConnection";
import { LiveRegionProvider, ToastProvider } from "@/components/ui";
import { TEST_ADDRESS } from "./apiFixtures";

export interface RouteRenderOptions {
  /** Connected wallet address, or `null` for a disconnected session. */
  address?: string | null;
  /** Seed entries into the SWR cache before render (keyed by request URL). */
  swrFallback?: Record<string, unknown>;
}

/** A minimal but complete WalletConnectionState for tests. */
export function walletState(address: string | null): WalletConnectionState {
  return {
    address,
    isConnecting: false,
    error: null,
    requiredNetworkLabel: "Testnet",
    connect: async () => address,
    disconnect: () => {},
    clearError: () => {},
  };
}

function Providers({
  children,
  options,
}: {
  children: ReactNode;
  options: RouteRenderOptions;
}) {
  const { address = TEST_ADDRESS, swrFallback } = options;

  return (
    <SWRConfig
      value={{
        provider: () => new Map(),
        fallback: swrFallback,
        // Deterministic behaviour for tests: no retries, no dedupe window, and
        // no focus/reconnect revalidation firing outside our control.
        dedupingInterval: 0,
        errorRetryCount: 0,
        shouldRetryOnError: false,
        revalidateOnFocus: false,
        revalidateOnReconnect: false,
      }}
    >
      <WalletContext.Provider value={walletState(address)}>
        <LiveRegionProvider>
          <ToastProvider>{children}</ToastProvider>
        </LiveRegionProvider>
      </WalletContext.Provider>
    </SWRConfig>
  );
}

/** Render a route component with all app providers and an isolated SWR cache. */
export function renderRoute(
  ui: ReactElement,
  options: RouteRenderOptions = {},
): RenderResult {
  return render(<Providers options={options}>{ui}</Providers>);
}
