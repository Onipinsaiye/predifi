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

export type StellarTxStatus = "idle" | "running" | "confirmed" | "ambiguous" | "failed";

export interface UseStellarTransactionOptions<TParams> {
  /** Connected source account, or null when disconnected. */
  address: string | null;
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
  const { address, submit, maxAutoRetries = 1, createIntent, persistTransaction = true } = options;
  const refreshAccountState = options.refreshAccountState ?? refreshAccountSequence;

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

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    intentRef.current = null;
    setStatus("idle");
    setHash(undefined);
    setError(undefined);
    setIntent(null);
    setRequiresUserConfirmation(false);
    setAccountState(null);

    if (!persistTransaction || !address) return;

    const saved = getPersistedStellarTransaction<TParams>(address);

    if (!saved) return;

    intentRef.current = saved.intent;
    setIntent(saved.intent);
    setHash(saved.hash);
    setAccountState(saved.accountState);
    setStatus(saved.status);
    setRequiresUserConfirmation(saved.requiresUserConfirmation);
    setError(saved.error);
  }, [address, persistTransaction]);

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
    [address, persistTransaction],
  );

  const execute = useCallback(
    async (
      txIntent: TransactionIntent<TParams>,
      userConfirmedRetry: boolean,
    ): Promise<TransactionRunResult<TParams>> => {
      const executionAddress = address;
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
      if (!mountedRef.current || addressRef.current !== executionAddress) {
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
    [address, submit, refreshAccountState, maxAutoRetries, persistResult],
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

      const txIntent = (createIntent ?? defaultIntent)(params);
      return startExecution(txIntent, false);
    },
    [address, status, createIntent, startExecution],
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

    return startExecution(txIntent, true);
  }, [intent, startExecution]);

  const reset = useCallback(() => {
    if (persistTransaction && address) {
      removePersistedStellarTransaction(address);
    }

    intentRef.current = null;
    setStatus("idle");
    setHash(undefined);
    setError(undefined);
    setIntent(null);
    setRequiresUserConfirmation(false);
    setAccountState(null);
  }, [address, persistTransaction]);

  return useMemo(
    () => ({
      status,
      hash,
      error,
      intent,
      requiresUserConfirmation,
      accountState,
      run,
      confirmRetry,
      reset,
    }),
    [status, hash, error, intent, requiresUserConfirmation, accountState, run, confirmRetry, reset],
  );
}
