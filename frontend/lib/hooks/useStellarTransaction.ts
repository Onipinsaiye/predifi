"use client";

/**
 * useStellarTransaction — React integration for safe Stellar submission.
 *
 * Wraps {@link runTransactionWithSequenceRecovery} so a component can:
 *   • run an economic action and let definite failures (stale sequence,
 *     pre-submit errors) recover automatically, while
 *   • pausing on an ambiguous outcome — `requiresUserConfirmation` becomes
 *     true and the caller must render a prompt and call `confirmRetry()`.
 *
 * The status maps directly onto the existing `TransactionProgress` states
 * (`idle` | `running` → submitted/processing | `confirmed` | `failed`), and
 * `error` is a `WalletError` ready for `WalletErrorBanner`.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  refreshAccountSequence,
  runTransactionWithSequenceRecovery,
  type AccountState,
  type SubmitFn,
  type TransactionIntent,
  type TransactionRunResult,
} from "@/lib/stellarTransaction";
import type { WalletError } from "@/lib/walletErrors";
import {
  getPersistedStellarTransaction,
  removePersistedStellarTransaction,
  setPersistedStellarTransaction,
} from "@/lib/stellarTransactionStorage";
import { useNetworkGuard } from "@/lib/hooks/useNetworkGuard";

export type StellarTxStatus = "idle" | "running" | "confirmed" | "ambiguous" | "failed";

export interface UseStellarTransactionOptions<TParams> {
  /** Connected source account, or null when disconnected. */
  address: string | null;
  /**
   * The wallet that is currently authorized to sign. In a multi-wallet
   * portfolio view, read-only addresses may be aggregated, but every
   * mutation must originate from this active signing account. Defaults to
   * `address` when omitted.
   */
  activeSigningAddress?: string | null;
  /** Network the wallet is connected to; scopes persisted state. */
  network?: string;
  /** Build, sign and broadcast the intent. Throw `TransactionSubmitError`. */
  submit: SubmitFn<TParams>;
  /** Override account-state refresh (tests / custom RPC). */
  refreshAccountState?: (address: string) => Promise<AccountState>;
  /** Automatic retries for definite failures. Default 1. */
  maxAutoRetries?: number;
  /** Build the retained intent for a run. Defaults to a generated id. */
  createIntent?: (params: TParams) => TransactionIntent<TParams>;
  /** Restore and persist this wallet's transaction state. Defaults to true. */
  persistTransaction?: boolean;
  /**
   * When the configured contract identity does not match the pinned
   * deployment manifest, mutations must be disabled. Defaults to the
   * shared network guard's verdict.
   */
  mutationsDisabled?: boolean;
  /** Diagnostic surfaced when mutations are disabled. */
  identityDiagnostic?: WalletError;
}

export interface UseStellarTransactionResult<TParams> {
  status: StellarTxStatus;
  /** Transaction hash, retained across ambiguity. */
  hash?: string;
  error?: WalletError;
  /** The original intent, retained for a confirmed retry. */
  intent: TransactionIntent<TParams> | null;
  /** True when the caller must ask the user before retrying. */
  requiresUserConfirmation: boolean;
  accountState: AccountState | null;
  /** True when contract identity is unverified; mutations are blocked. */
  mutationsDisabled: boolean;
  /** Run the action, creating a fresh intent. */
  run: (params: TParams) => Promise<TransactionRunResult<TParams>>;
  /** Retry an ambiguous outcome after explicit user confirmation. */
  confirmRetry: () => Promise<TransactionRunResult<TParams> | null>;
  reset: () => void;
}

let intentCounter = 0;

function defaultIntent<TParams>(params: TParams): TransactionIntent<TParams> {
  intentCounter += 1;
  return {
    id: `stellar-tx-${Date.now()}-${intentCounter}`,
    params,
    createdAt: Date.now(),
  };
}

export function useStellarTransaction<TParams>(
  options: UseStellarTransactionOptions<TParams>,
): UseStellarTransactionResult<TParams> {
  const {
    address,
    network,
    submit,
    maxAutoRetries = 1,
    createIntent,
    persistTransaction = true,
  } = options;
  const refreshAccountState = options.refreshAccountState ?? refreshAccountSequence;

  // The active signing wallet. When not explicitly provided, the connected
  // `address` is the only signer.
  const activeSigningAddress =
    options.activeSigningAddress !== undefined
      ? options.activeSigningAddress
      : address;
  const activeSigningAddressRef = useRef(activeSigningAddress);
  activeSigningAddressRef.current = activeSigningAddress;

  /**
   * Refuse to sign from any account other than the active signing wallet.
   * Returns a `WalletError` describing the refusal, or `null` when the
   * requested source is authorized to sign.
   */
  const checkSigningAuthorization = useCallback(
    (requestedAddress: string | null | undefined): WalletError | null => {
      const active = activeSigningAddressRef.current;
      if (!active) {
        return {
          code: "WALLET_NOT_CONNECTED",
          title: "No active signing wallet",
          message: "Connect the wallet you want to sign with before submitting.",
          recoveryAction: "Connect the active signing wallet and try again.",
        };
      }
      if (!requestedAddress || requestedAddress !== active) {
        return {
          code: "SIGNING_WALLET_MISMATCH",
          title: "Wrong signing wallet",
          message:
            "This transaction must be signed by the active wallet. Switch to the active signing wallet to continue.",
          recoveryAction: "Switch to the active signing wallet and retry.",
        };
      }
      return null;
    },
    [],
  );

  // Contract identity guard: a mismatch (or unavailable/stale manifest)
  // disables mutations and surfaces a diagnostic through `error`.
  const guard = useNetworkGuard();
  const mutationsDisabled = options.mutationsDisabled ?? guard.mutationsDisabled;
  const identityDiagnostic = options.identityDiagnostic ?? guard.diagnostic;
  const mutationsDisabledRef = useRef(mutationsDisabled);
  mutationsDisabledRef.current = mutationsDisabled;
  const identityDiagnosticRef = useRef(identityDiagnostic);
  identityDiagnosticRef.current = identityDiagnostic;

  const [status, setStatus] = useState<StellarTxStatus>("idle");
  const [hash, setHash] = useState<string>();
  const [error, setError] = useState<WalletError>();
  const [intent, setIntent] = useState<TransactionIntent<TParams> | null>(null);
  const [requiresUserConfirmation, setRequiresUserConfirmation] = useState(false);
  const [accountState, setAccountState] = useState<AccountState | null>(null);

  const intentRef = useRef<TransactionIntent<TParams> | null>(null);
  const activeRunRef = useRef<Promise<TransactionRunResult<TParams>> | null>(null);
  const mountedRef = useRef(true);
  const addressRef = useRef(address);
  addressRef.current = address;
  const networkRef = useRef(network);
  networkRef.current = network;
  // Identity key: account data is owned by (network, address). Any change
  // invalidates in-flight runs and all account-scoped UI state.
  const identityKey = address ? `${network ?? "unknown"}:${address}` : null;
  const identityRef = useRef(identityKey);
  identityRef.current = identityKey;

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    // Detach any in-flight run so its result cannot touch the new identity.
    activeRunRef.current = null;
    intentRef.current = null;
    setStatus("idle");
    setHash(undefined);
    setError(undefined);
    setIntent(null);
    setRequiresUserConfirmation(false);
    setAccountState(null);

    if (!persistTransaction || !address) return;

    const saved = getPersistedStellarTransaction<TParams>(address, network);

    if (!saved) return;

    intentRef.current = saved.intent;
    setIntent(saved.intent);
    setHash(saved.hash);
    setAccountState(saved.accountState);
    setStatus(saved.status);
    setRequiresUserConfirmation(saved.requiresUserConfirmation);
    setError(saved.error);
  }, [identityKey, address, network, persistTransaction]);

  const persistResult = useCallback(
    (
      nextStatus: Exclude<StellarTxStatus, "idle">,
      txIntent: TransactionIntent<TParams>,
      nextHash?: string,
      nextError?: WalletError,
      nextRequiresUserConfirmation = false,
      nextAccountState: AccountState | null = null,
    ) => {
      if (!persistTransaction || !address) return;

      setPersistedStellarTransaction({
        version: 1,
        address,
        network,
        status: nextStatus,
        intent: txIntent,
        hash: nextHash,
        error: nextError
          ? {
              code: nextError.code,
              title: nextError.title,
              message: nextError.message,
              recoveryAction: nextError.recoveryAction,
            }
          : undefined,
        requiresUserConfirmation: nextRequiresUserConfirmation,
        accountState: nextAccountState,
        updatedAt: Date.now(),
      });
    },
    [address, network, persistTransaction],
  );

  const execute = useCallback(
    async (
      txIntent: TransactionIntent<TParams>,
      userConfirmedRetry: boolean,
    ): Promise<TransactionRunResult<TParams>> => {
      // Refuse to sign from any account other than the active signing wallet.
      const authorizationError = checkSigningAuthorization(address);
      if (authorizationError) {
        const refused: TransactionRunResult<TParams> = {
          status: "failed",
          intent: txIntent,
          error: authorizationError,
          requiresUserConfirmation: false,
          accountState: null,
        };
        if (mountedRef.current) {
          setStatus("failed");
          setError(authorizationError);
          setRequiresUserConfirmation(false);
        }
        return refused;
      }

      // Refuse to submit while the contract identity is unverified.
      if (mutationsDisabledRef.current) {
        const diagnostic = identityDiagnosticRef.current;
        const blocked: TransactionRunResult<TParams> = {
          status: "failed",
          intent: txIntent,
          error: diagnostic,
          requiresUserConfirmation: false,
          accountState: null,
        };
        if (mountedRef.current) {
          setStatus("failed");
          setError(diagnostic);
          setRequiresUserConfirmation(false);
        }
        return blocked;
      }

      const executionAddress = address;
      const executionIdentity = identityKey;
      intentRef.current = txIntent;
      persistResult("running", txIntent);
      setStatus("running");
      setError(undefined);
      setRequiresUserConfirmation(false);

      const result = await runTransactionWithSequenceRecovery({
        intent: txIntent,
        submit,
        address,
        refreshAccountState,
        maxAutoRetries,
        userConfirmedRetry,
      });

      // Save the final result against the wallet that started the transaction.
      persistResult(
        result.status,
        result.intent,
        result.hash,
        result.error,
        result.requiresUserConfirmation,
        result.accountState,
      );

      // Do not let an old wallet's result overwrite the current wallet's UI.
      if (
        !mountedRef.current ||
        addressRef.current !== executionAddress ||
        identityRef.current !== executionIdentity
      ) {
        return result;
      }

      setIntent(result.intent);
      setHash(result.hash);
      setAccountState(result.accountState);

      if (result.status === "confirmed") {
        setStatus("confirmed");
        setRequiresUserConfirmation(false);
      } else if (result.status === "ambiguous") {
        setStatus("ambiguous");
        setError(result.error);
        setRequiresUserConfirmation(true);
      } else {
        setStatus("failed");
        setError(result.error);
        setRequiresUserConfirmation(false);
      }

      return result;
    },
    [
      address,
      identityKey,
      submit,
      refreshAccountState,
      maxAutoRetries,
      persistResult,
      checkSigningAuthorization,
    ],
  );

  const startExecution = useCallback(
    (
      txIntent: TransactionIntent<TParams>,
      userConfirmedRetry: boolean,
    ): Promise<TransactionRunResult<TParams>> => {
      // Reuse the current execution instead of submitting again.
      if (activeRunRef.current) {
        return activeRunRef.current;
      }

      const activeRun = execute(txIntent, userConfirmedRetry).finally(() => {
        if (activeRunRef.current === activeRun) {
          activeRunRef.current = null;
        }
      });

      activeRunRef.current = activeRun;
      return activeRun;
    },
    [execute],
  );

  const run = useCallback(
    (params: TParams): Promise<TransactionRunResult<TParams>> => {
      // Reuse an execution already in progress.
      if (activeRunRef.current) {
        return activeRunRef.current;
      }

      // An ambiguous transaction must be resolved before a new intent starts.
      if (status === "ambiguous") {
        return Promise.reject(
          new Error(
            "An unresolved transaction exists. Confirm the retry or reset the transaction before starting a new one.",
          ),
        );
      }

      if (!address) {
        return Promise.reject(new Error("Connect your wallet before submitting."));
      }

      // Block mutations that would be signed by a non-active account.
      const authorizationError = checkSigningAuthorization(address);
      if (authorizationError) {
        setError(authorizationError);
        return Promise.reject(
          Object.assign(new Error(authorizationError.message), {
            walletError: authorizationError,
          }),
        );
      }

      // Block new mutations when the contract identity is unverified.
      if (mutationsDisabled) {
        const diagnostic = identityDiagnostic;
        if (diagnostic) {
          setError(diagnostic);
        }
        return Promise.reject(
          diagnostic
            ? Object.assign(new Error(diagnostic.message), { walletError: diagnostic })
            : new Error(
                "Contract identity could not be verified. Mutations are disabled.",
              ),
        );
      }

      const txIntent = (createIntent ?? defaultIntent)(params);
      return startExecution(txIntent, false);
    },
    [
      address,
      status,
      createIntent,
      startExecution,
      mutationsDisabled,
      identityDiagnostic,
      checkSigningAuthorization,
    ],
  );

  const confirmRetry = useCallback(() => {
    // Never start a retry while another execution is active.
    if (activeRunRef.current) {
      return activeRunRef.current;
    }

    const txIntent = intentRef.current ?? intent;
    if (!txIntent) {
      return Promise.resolve(null);
    }

    // A confirmed retry is still a mutation; it must be signed by the
    // active wallet.
    const authorizationError = checkSigningAuthorization(address);
    if (authorizationError) {
      setError(authorizationError);
      return Promise.resolve(null);
    }

    // A confirmed retry is still a mutation; block it on identity mismatch.
    if (mutationsDisabled) {
      const diagnostic = identityDiagnostic;
      if (diagnostic) {
        setError(diagnostic);
      }
      return Promise.resolve(null);
    }

    return startExecution(txIntent, true);
  }, [
    intent,
    address,
    startExecution,
    mutationsDisabled,
    identityDiagnostic,
    checkSigningAuthorization,
  ]);

  const reset = useCallback(() => {
    if (persistTransaction && address) {
      removePersistedStellarTransaction(address, network);
    }

    intentRef.current = null;
    setStatus("idle");
    setHash(undefined);
    setError(undefined);
    setIntent(null);
    setRequiresUserConfirmation(false);
    setAccountState(null);
  }, [address, network, persistTransaction]);

  // Surface the identity diagnostic through the existing error banner while
  // idle, without clobbering a real transaction error.
  const surfacedError = useMemo<WalletError | undefined>(() => {
    if (error) return error;
    if (mutationsDisabled && identityDiagnostic) return identityDiagnostic;
    return undefined;
  }, [error, mutationsDisabled, identityDiagnostic]);

  return useMemo(
    () => ({
      status,
      hash,
      error: surfacedError,
      intent,
      requiresUserConfirmation,
      accountState,
      mutationsDisabled,
      run,
      confirmRetry,
      reset,
    }),
    [
      status,
      hash,
      surfacedError,
      intent,
      requiresUserConfirmation,
      accountState,
      mutationsDisabled,
      run,
      confirmRetry,
      reset,
    ],
  );
}
