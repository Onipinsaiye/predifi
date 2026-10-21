
import type {
  AccountState,
  TransactionIntent,
} from "@/lib/stellarTransaction";
import type { WalletError } from "@/lib/walletErrors";

const STORAGE_PREFIX = "predifi:stellar-transaction:";
const STORAGE_VERSION = 1;
const MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;

export type PersistedTransactionStatus =
  | "running"
  | "confirmed"
  | "ambiguous"
  | "failed";

export type RestoredTransactionStatus = Exclude<
  PersistedTransactionStatus,
  "running"
>;

export type PersistedTransactionError = Pick<
  WalletError,
  "code" | "title" | "message" | "recoveryAction"
>;

export interface PersistedStellarTransaction<TParams = unknown> {
  version: typeof STORAGE_VERSION;
  address: string;
  status: PersistedTransactionStatus;
  intent: TransactionIntent<TParams>;
  hash?: string;
  error?: PersistedTransactionError;
  requiresUserConfirmation: boolean;
  accountState: AccountState | null;
  updatedAt: number;
}

export interface RestoredStellarTransaction<TParams = unknown>
  extends Omit<PersistedStellarTransaction<TParams>, "status"> {
  status: RestoredTransactionStatus;
}

function getStorageKey(address: string): string {
  return `${STORAGE_PREFIX}${encodeURIComponent(address)}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isPersistedTransaction(
  value: unknown,
  expectedAddress: string,
): value is PersistedStellarTransaction<unknown> {
  if (!isRecord(value) || !isRecord(value.intent)) {
    return false;
  }

  const validStatuses: PersistedTransactionStatus[] = [
    "running",
    "confirmed",
    "ambiguous",
    "failed",
  ];

  return (
    value.version === STORAGE_VERSION &&
    value.address === expectedAddress &&
    typeof value.status === "string" &&
    validStatuses.includes(
      value.status as PersistedTransactionStatus,
    ) &&
    typeof value.intent.id === "string" &&
    typeof value.intent.createdAt === "number" &&
    Number.isFinite(value.intent.createdAt) &&
    "params" in value.intent &&
    typeof value.updatedAt === "number" &&
    Number.isFinite(value.updatedAt) &&
    typeof value.requiresUserConfirmation === "boolean" &&
    (value.hash === undefined || typeof value.hash === "string") &&
    (value.accountState === null || value.accountState !== undefined)
  );
}

/**
 * Saves serializable transaction metadata in localStorage.
 *
 * Do not include private keys, wallet secrets, signed transaction
 * payloads, or other sensitive values in intent.params.
 */
export function setPersistedStellarTransaction<TParams>(
  transaction: PersistedStellarTransaction<TParams>,
): boolean {
  try {
    if (typeof window === "undefined") {
      return false;
    }

    window.localStorage.setItem(
      getStorageKey(transaction.address),
      JSON.stringify(transaction),
    );

    return true;
  } catch {
    // Storage may be blocked, full, or unable to serialize the record.
    return false;
  }
}

/**
 * Restores a transaction for the specified wallet.
 *
 * A persisted "running" state means the previous page was interrupted.
 * It is restored as "ambiguous" and must never be resubmitted automatically.
 */
export function getPersistedStellarTransaction<TParams = unknown>(
  address: string,
  now = Date.now(),
): RestoredStellarTransaction<TParams> | null {
  if (!address) {
    return null;
  }

  const key = getStorageKey(address);

  try {
    if (typeof window === "undefined") {
      return null;
    }

    const raw = window.localStorage.getItem(key);

    if (!raw) {
      return null;
    }

    const parsed: unknown = JSON.parse(raw);

    if (!isPersistedTransaction(parsed, address)) {
      window.localStorage.removeItem(key);
      return null;
    }

    if (
      now - parsed.updatedAt > MAX_AGE_MS ||
      parsed.updatedAt > now
    ) {
      window.localStorage.removeItem(key);
      return null;
    }

    if (parsed.status === "running") {
      const {
        status: _runningStatus,
        error: _previousError,
        ...transaction
      } = parsed;

      return {
        ...transaction,
        status: "ambiguous",
        requiresUserConfirmation: true,
        error: {
          code: "UNKNOWN",
          title: "Transaction status unknown",
          message:
            "The page was interrupted while this transaction was running. It may already have reached the Stellar network.",
          recoveryAction:
            "Check the transaction hash or wallet activity before deciding whether to retry.",
        },
      } as RestoredStellarTransaction<TParams>;
    }

    return {
      ...parsed,
      requiresUserConfirmation:
        parsed.status === "ambiguous"
          ? true
          : parsed.requiresUserConfirmation,
    } as RestoredStellarTransaction<TParams>;
  } catch {
    // Invalid JSON or unavailable browser storage must not crash the app.
    try {
      if (typeof window !== "undefined") {
        window.localStorage.removeItem(key);
      }
    } catch {
      // Ignore storage cleanup failures.
    }

    return null;
  }
}

export function removePersistedStellarTransaction(
  address: string,
): void {
  if (!address) {
    return;
  }

  try {
    if (typeof window !== "undefined") {
      window.localStorage.removeItem(getStorageKey(address));
    }
  } catch {
    // Cleanup should not crash the app if storage is unavailable.
  }
}
