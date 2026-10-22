"use client";

/**
 * useCurrentUserAddress — returns the connected wallet address.
 *
 * Migration (issue #1406):
 *   Previously returned a hardcoded placeholder. Now reads the real address
 *   from WalletContext so all consumers automatically receive `undefined` when
 *   disconnected and the real address after the user connects — no callsite
 *   changes required.
 *
 * Falls back gracefully to `undefined` (no throw) when WalletProvider is not
 * mounted — e.g. marketing pages or isolated unit tests.
 *
 * Identity scoping (issue #81):
 *   `useWalletIdentity` exposes a stable identity key derived from the active
 *   network and address so consumers can scope caches by network+address and
 *   avoid reusing prior user data after an account or network change.
 */

import { useContext } from "react";
// Import the context object directly (not through the barrel) to avoid a
// circular dependency: barrel → WalletContext → (re-exported) → barrel.
import { WalletContext } from "@/lib/context/WalletContext";

export function useCurrentUserAddress(): string | undefined {
  const ctx = useContext(WalletContext);
  return ctx?.address ?? undefined;
}

export interface WalletIdentity {
  /** Active network identifier, if known. */
  network: string | undefined;
  /** Connected wallet address, if any. */
  address: string | undefined;
  /**
   * Stable cache key scoped by network and address.
   * `undefined` when disconnected so callers can clear/skip account data.
   */
  identityKey: string | undefined;
}

/**
 * useWalletIdentity — returns the current network, address, and a combined
 * identity key for scoping account-specific caches and state.
 *
 * The identity key is `undefined` when there is no connected address, which
 * signals consumers to clear sensitive account data. When connected, the key
 * changes whenever the network or address changes, so caches keyed by it will
 * never reuse another account's data.
 */
export function useWalletIdentity(): WalletIdentity {
  const ctx = useContext(WalletContext);
  const address = ctx?.address ?? undefined;
  const network = ctx?.network ?? undefined;

  const identityKey =
    address === undefined
      ? undefined
      : `${network ?? "unknown"}:${address.toLowerCase()}`;

  return { network, address, identityKey };
}