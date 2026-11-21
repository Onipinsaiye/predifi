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

/**
 * Normalize a Stellar address for comparison purposes.
 *
 * Stellar account IDs are case-sensitive base32 (StrKey), so we only trim
 * surrounding whitespace. We deliberately do NOT lowercase — doing so would
 * silently merge distinct addresses and could misattribute portfolio data.
 */
export function normalizeWalletAddress(
  address: string | null | undefined,
): string | undefined {
  if (typeof address !== "string") return undefined;
  const trimmed = address.trim();
  return trimmed.length > 0 ? trimmed : undefined;
}

export interface MultiWalletPortfolio {
  /** The active signing wallet address, if connected. */
  activeAddress: string | undefined;
  /**
   * Normalized, de-duplicated read-only addresses selected for aggregation.
   * Never includes the active signing wallet.
   */
  readOnlyAddresses: string[];
  /**
   * All addresses to aggregate (read-only selections plus the active wallet
   * when connected), in stable order: read-only first, then active.
   */
  allAddresses: string[];
  /**
   * Returns true when `address` is the active signing wallet. Mutation paths
   * must use this (or `activeAddress`) to authorize writes; read-only
   * selections are never allowed to sign.
   */
  isActiveWallet: (address: string | null | undefined) => boolean;
}

/**
 * useMultiWalletPortfolio — resolves the explicitly selected read-only wallet
 * addresses alongside the active signing wallet for the consolidated
 * portfolio view (issue #266).
 *
 * Guarantees:
 *   - Addresses are normalized (trimmed) and de-duplicated.
 *   - The active signing wallet is never included in `readOnlyAddresses`.
 *   - `isActiveWallet` lets mutation paths refuse to sign from a non-active
 *     account.
 *
 * Falls back to an empty selection when WalletProvider is not mounted.
 */
export function useMultiWalletPortfolio(
  selectedAddresses: ReadonlyArray<string | null | undefined> = [],
): MultiWalletPortfolio {
  const ctx = useContext(WalletContext);
  const activeAddress = normalizeWalletAddress(ctx?.address);

  const readOnlyAddresses: string[] = [];
  const seen = new Set<string>();
  for (const candidate of selectedAddresses) {
    const normalized = normalizeWalletAddress(candidate);
    if (normalized === undefined) continue;
    if (normalized === activeAddress) continue;
    if (seen.has(normalized)) continue;
    seen.add(normalized);
    readOnlyAddresses.push(normalized);
  }

  const allAddresses =
    activeAddress === undefined
      ? readOnlyAddresses
      : [...readOnlyAddresses, activeAddress];

  const isActiveWallet = (address: string | null | undefined): boolean => {
    const normalized = normalizeWalletAddress(address);
    return normalized !== undefined && normalized === activeAddress;
  };

  return { activeAddress, readOnlyAddresses, allAddresses, isActiveWallet };
}