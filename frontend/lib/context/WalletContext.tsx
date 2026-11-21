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
  useState,
  type ReactNode,
} from "react";
import { useSWRConfig } from "swr";
import {
  useWalletConnection,
  type WalletConnectionState,
} from "@/lib/hooks/useWalletConnection";
import { removePersistedStellarTransaction } from "@/lib/stellar/pendingTransactions";

// ── Context ───────────────────────────────────────────────────────────────────

/**
 * Wallet state exposed to consumers.
 *
 * `identityKey` is a stable string derived from the active network + address
 * (or `null` when disconnected). Account-scoped hooks should include it in
 * their SWR keys so cached data is never reused across identities.
 *
 * `generation` is bumped on every identity change. Async work started under a
 * previous generation can compare against the current value and discard its
 * result, preventing stale responses from a prior account/network leaking in.
 */
export interface WalletContextValue extends WalletConnectionState {
  identityKey: string | null;
  generation: number;
  /**
   * Explicitly selected read-only wallet addresses used for the aggregated
   * portfolio view. These are NEVER used for signing — every mutation must
   * go through the active signing wallet (`address`).
   *
   * Normalized (trimmed + uppercased) and deduplicated. The active signing
   * address is never included here.
   */
  readOnlyAddresses: string[];
  /** Add a read-only address to the aggregated portfolio view. */
  addReadOnlyAddress: (address: string) => void;
  /** Remove a read-only address from the aggregated portfolio view. */
  removeReadOnlyAddress: (address: string) => void;
  /** Clear all read-only addresses. */
  clearReadOnlyAddresses: () => void;
  /** True when `address` is the active signing wallet (normalized compare). */
  isActiveSigningAddress: (address: string | null | undefined) => boolean;
  /**
   * Returns the active signing address or throws when no wallet is connected.
   * Mutations must call this so they can never sign from a read-only account.
   */
  requireActiveSigningAddress: () => string;
}

/**
 * Normalize a Stellar address for comparison/aggregation: trim surrounding
 * whitespace and uppercase (Stellar StrKey is case-insensitive in practice
 * for our comparison purposes). Returns an empty string for non-strings.
 */
export function normalizeWalletAddress(address: string): string {
  if (typeof address !== "string") return "";
  return address.trim().toUpperCase();
}

/**
 * Exported so useCurrentUserAddress can consume it directly without going
 * through the barrel (avoids a circular import).
 */
export const WalletContext = createContext<WalletContextValue | undefined>(
  undefined,
);
WalletContext.displayName = "WalletContext";

// ── Provider ──────────────────────────────────────────────────────────────────

export function WalletProvider({ children }: { children: ReactNode }) {
  const wallet = useWalletConnection();
  const { mutate } = useSWRConfig();

  // Identity = network + address. `null` when no wallet is connected.
  const identityKey = wallet.identityKey;

  // Bumped on every identity change so in-flight async work can detect that
  // it belongs to a previous identity and discard its result.
  const [generation, setGeneration] = useState(0);

  // Explicitly selected read-only addresses for the aggregated portfolio view.
  // Stored normalized + deduplicated; never used for signing.
  const [readOnlyAddresses, setReadOnlyAddresses] = useState<string[]>([]);

  const activeAddress = wallet.address;

  const addReadOnlyAddress = useCallback(
    (address: string) => {
      const normalized = normalizeWalletAddress(address);
      if (normalized === "") return;
      // The active signing wallet is always included implicitly; never store
      // it as a read-only entry (avoids duplicate attribution).
      if (
        activeAddress !== null &&
        normalizeWalletAddress(activeAddress) === normalized
      ) {
        return;
      }
      setReadOnlyAddresses((prev) =>
        prev.includes(normalized) ? prev : [...prev, normalized],
      );
    },
    [activeAddress],
  );

  const removeReadOnlyAddress = useCallback((address: string) => {
    const normalized = normalizeWalletAddress(address);
    if (normalized === "") return;
    setReadOnlyAddresses((prev) => prev.filter((a) => a !== normalized));
  }, []);

  const clearReadOnlyAddresses = useCallback(() => {
    setReadOnlyAddresses([]);
  }, []);

  const isActiveSigningAddress = useCallback(
    (address: string | null | undefined) => {
      if (activeAddress === null || address == null) return false;
      const normalized = normalizeWalletAddress(address);
      return (
        normalized !== "" &&
        normalized === normalizeWalletAddress(activeAddress)
      );
    },
    [activeAddress],
  );

  const requireActiveSigningAddress = useCallback(() => {
    if (activeAddress === null) {
      throw new Error(
        "No active signing wallet connected. Connect a wallet to sign transactions.",
      );
    }
    return activeAddress;
  }, [activeAddress]);

  // Track the previous identity/address so we can detect transitions.
  const prevIdentityRef = useRef<string | null>(identityKey);
  const prevAddressRef = useRef<string | null>(wallet.address);

  // Clear all account-scoped state whenever the identity changes: disconnect,
  // account switch, or network switch. This prevents balances, history,
  // cached requests, and pending UI from a previous identity remaining visible.
  useEffect(() => {
    const prevIdentity = prevIdentityRef.current;
    const prevAddress = prevAddressRef.current;
    const currIdentity = identityKey;
    const currAddress = wallet.address;

    if (prevIdentity !== currIdentity) {
      // 1. Bump the generation so stale in-flight responses are ignored.
      setGeneration((g) => g + 1);

      // 1b. Drop read-only selections on identity change. Aggregation is
      //     scoped to the current session/account; keeping stale addresses
      //     would silently mix portfolios across accounts.
      setReadOnlyAddresses([]);

      // 2. Drop every cached SWR entry. Keys are identity-scoped, so nothing
      //    from the previous account/network can be reused.
      void mutate(() => true, undefined, { revalidate: false });

      // 3. Clear pending transaction UI for the previous address.
      if (prevAddress !== null) {
        removePersistedStellarTransaction(prevAddress);
      }
    }

    prevIdentityRef.current = currIdentity;
    prevAddressRef.current = currAddress;
  }, [identityKey, wallet.address, mutate]);

  // Wrap disconnect to also clear the cache immediately on user action,
  // without waiting for the useEffect to run on the next render cycle.
  const disconnect = useCallback(() => {
    const prevAddress = wallet.address;
    wallet.disconnect();
    setGeneration((g) => g + 1);
    setReadOnlyAddresses([]);
    void mutate(() => true, undefined, { revalidate: false });
    if (prevAddress !== null) {
      removePersistedStellarTransaction(prevAddress);
    }
  }, [wallet, mutate]);

  // Keep the context value object reference stable when wallet state is
  // unchanged — prevents all consumers from re-rendering on unrelated updates.
  const value = useMemo<WalletContextValue>(
    () => ({
      address: wallet.address,
      isConnecting: wallet.isConnecting,
      error: wallet.error,
      requiredNetworkLabel: wallet.requiredNetworkLabel,
      connect: wallet.connect,
      disconnect,
      clearError: wallet.clearError,
      identityKey,
      generation,
      readOnlyAddresses,
      addReadOnlyAddress,
      removeReadOnlyAddress,
      clearReadOnlyAddresses,
      isActiveSigningAddress,
      requireActiveSigningAddress,
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
      generation,
      readOnlyAddresses,
      addReadOnlyAddress,
      removeReadOnlyAddress,
      clearReadOnlyAddresses,
      isActiveSigningAddress,
      requireActiveSigningAddress,
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
export function useWallet(): WalletContextValue {
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
 * useWalletIdentity — returns the identity key and generation counter.
 *
 * Account-scoped hooks should include `identityKey` in their SWR keys and
 * compare `generation` before applying async results so responses from a
 * previous account or network are never reused.
 */
export function useWalletIdentity(): {
  identityKey: string | null;
  generation: number;
} {
  const { identityKey, generation } = useWallet();
  return { identityKey, generation };
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

/**
 * usePortfolioAddresses — returns the addresses to aggregate for the
 * portfolio view: the active signing wallet (if connected) plus any
 * explicitly selected read-only addresses.
 *
 * The active address is always first so consumers can attribute it as the
 * signing wallet. Read-only addresses are normalized + deduplicated and never
 * include the active address.
 */
export function usePortfolioAddresses(): {
  activeAddress: string | null;
  readOnlyAddresses: string[];
  allAddresses: string[];
} {
  const { address, readOnlyAddresses } = useWallet();
  const activeAddress = address ?? null;
  const allAddresses = useMemo(() => {
    const list = activeAddress ? [activeAddress] : [];
    for (const addr of readOnlyAddresses) {
      if (activeAddress && normalizeWalletAddress(addr) === normalizeWalletAddress(activeAddress)) {
        continue;
      }
      list.push(addr);
    }
    return list;
  }, [activeAddress, readOnlyAddresses]);
  return { activeAddress, readOnlyAddresses, allAddresses };
}

// Re-export the state type so consumers don't need to import from the hook.
export type { WalletConnectionState };
