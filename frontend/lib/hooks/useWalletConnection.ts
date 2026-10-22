"use client";

import { useCallback, useState } from "react";
import {
  classifyWalletError,
  checkNetworkMatch,
  isStellarWalletInstalled,
  makeWalletError,
  withWalletTimeout,
  type WalletError,
  REQUIRED_STELLAR_NETWORK,
  STELLAR_NETWORK_LABELS,
} from "@/lib/walletErrors";

export interface WalletConnectionState {
  address: string | null;
  network: string | null;
  identityKey: string | null;
  isConnecting: boolean;
  error: WalletError | null;
  requiredNetworkLabel: string;
  connect: () => Promise<string | null>;
  disconnect: () => void;
  clearError: () => void;
  refreshNetwork: () => Promise<string | null>;
}

type FreighterLike = {
  isConnected?: () => Promise<boolean>;
  getNetwork?: () => Promise<string>;
  getNetworkDetails?: () => Promise<{ network: string; networkPassphrase?: string }>;
  getPublicKey?: () => Promise<string>;
  requestAccess?: () => Promise<string>;
};

function getFreighterApi(): FreighterLike | null {
  if (typeof window === "undefined") return null;
  const w = window as Window & {
    freighterApi?: FreighterLike;
    freighter?: FreighterLike;
  };
  return w.freighterApi ?? w.freighter ?? null;
}

/**
 * Connect / disconnect with structured error handling for:
 * extension missing, network mismatch, user rejection, timeout.
 */
export function useWalletConnection(): WalletConnectionState {
  const [address, setAddress] = useState<string | null>(null);
  const [network, setNetwork] = useState<string | null>(null);
  const [isConnecting, setIsConnecting] = useState(false);
  const [error, setError] = useState<WalletError | null>(null);

  const clearError = useCallback(() => setError(null), []);

  const disconnect = useCallback(() => {
    setAddress(null);
    setNetwork(null);
    setError(null);
  }, []);

  const refreshNetwork = useCallback(async (): Promise<string | null> => {
    const api = getFreighterApi();
    if (!api) {
      setNetwork(null);
      return null;
    }
    try {
      const current =
        (await api.getNetworkDetails?.())?.network ??
        (await api.getNetwork?.()) ??
        null;
      setNetwork(current);
      return current;
    } catch (netErr) {
      const classified = classifyWalletError(netErr);
      if (classified.code !== "UNKNOWN") {
        setError(classified);
      }
      setNetwork(null);
      return null;
    }
  }, []);

  const connect = useCallback(async (): Promise<string | null> => {
    setIsConnecting(true);
    setError(null);

    try {
      if (!isStellarWalletInstalled()) {
        const err = makeWalletError("EXTENSION_NOT_INSTALLED");
        setError(err);
        return null;
      }

      const api = getFreighterApi();
      if (!api) {
        const err = makeWalletError("EXTENSION_NOT_INSTALLED");
        setError(err);
        return null;
      }

      // Network check before requesting access
      try {
        const currentNetwork =
          (await api.getNetworkDetails?.())?.network ??
          (await api.getNetwork?.()) ??
          null;
        setNetwork(currentNetwork);
        const mismatch = checkNetworkMatch(currentNetwork);
        if (mismatch) {
          setError(mismatch);
          return null;
        }
      } catch (netErr) {
        // Some wallets throw if locked — classify rather than swallow.
        const classified = classifyWalletError(netErr);
        if (classified.code !== "UNKNOWN") {
          setError(classified);
          return null;
        }
      }

      const publicKey = await withWalletTimeout(
        (async () => {
          if (api.requestAccess) {
            return api.requestAccess();
          }
          if (api.getPublicKey) {
            return api.getPublicKey();
          }
          throw makeWalletError("EXTENSION_NOT_INSTALLED");
        })(),
        60_000
      );

      if (!publicKey || typeof publicKey !== "string") {
        const err = makeWalletError("USER_REJECTED");
        setError(err);
        return null;
      }

      setAddress(publicKey);
      return publicKey;
    } catch (err) {
      // withWalletTimeout may reject with a WalletError directly
      if (
        err &&
        typeof err === "object" &&
        "code" in err &&
        "recoveryAction" in err
      ) {
        setError(err as WalletError);
        return null;
      }
      const classified = classifyWalletError(err);
      setError(classified);
      return null;
    } finally {
      setIsConnecting(false);
    }
  }, []);

  const identityKey = address ? `${network ?? "unknown"}:${address}` : null;

  return {
    address,
    network,
    identityKey,
    isConnecting,
    error,
    requiredNetworkLabel: STELLAR_NETWORK_LABELS[REQUIRED_STELLAR_NETWORK],
    connect,
    disconnect,
    clearError,
    refreshNetwork,
  };
}

/**
 * Classify a transaction submission failure (balance, timeout, reject).
 * Use around contract invoke / payment submits.
 */
export function handleTransactionError(err: unknown): WalletError {
  return classifyWalletError(err);
}
