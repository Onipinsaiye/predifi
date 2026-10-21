import { act, renderHook } from "@testing-library/react";

import { useStellarTransaction } from "@/lib/hooks/useStellarTransaction";
import {
  TransactionSubmitError,
  type AccountState,
  type TransactionIntent,
} from "@/lib/stellarTransaction";

import {
  getPersistedStellarTransaction,
  removePersistedStellarTransaction,
  setPersistedStellarTransaction,
} from "@/lib/stellarTransactionStorage";

type Params = { amount: string };

const ADDRESS = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

const ACCOUNT_STATE: AccountState = {
  address: ADDRESS,
  sequence: "4294967296",
  refreshedAt: 1,
};

function fixedIntent(params: Params): TransactionIntent<Params> {
  return { id: "intent-1", params, createdAt: 1 };
}

describe("useStellarTransaction", () => {
  beforeEach(() => {
  removePersistedStellarTransaction(ADDRESS);
  });

  it("reports confirmed with the transaction hash on success", async () => {
    const { result } = renderHook(() =>
      useStellarTransaction<Params>({
        address: ADDRESS,
        createIntent: fixedIntent,
        submit: jest.fn(async () => ({ hash: "hash-1" })),
      })
    );

    await act(async () => {
      await result.current.run({ amount: "10" });
    });

    expect(result.current.status).toBe("confirmed");
    expect(result.current.hash).toBe("hash-1");
    expect(result.current.requiresUserConfirmation).toBe(false);
  });

  it("pauses on an ambiguous outcome without duplicating the action", async () => {
    const submit = jest.fn(async () => {
      throw new TransactionSubmitError({
        stage: "post_submit",
        hash: "abc123",
        cause: new Error("Request timed out"),
      });
    });

    const { result } = renderHook(() =>
      useStellarTransaction<Params>({
        address: ADDRESS,
        createIntent: fixedIntent,
        refreshAccountState: jest.fn(async () => ACCOUNT_STATE),
        submit,
      })
    );

    await act(async () => {
      await result.current.run({ amount: "10" });
    });

    expect(result.current.status).toBe("ambiguous");
    expect(result.current.requiresUserConfirmation).toBe(true);
    expect(result.current.error?.code).toBe("TRANSACTION_TIMEOUT");
    // Intent + hash retained for reconciliation.
    expect(result.current.intent?.id).toBe("intent-1");
    expect(result.current.hash).toBe("abc123");
    // Never automatically repeated.
    expect(submit).toHaveBeenCalledTimes(1);
  });

  it("retries an ambiguous outcome only when the user confirms", async () => {
    let calls = 0;
    const submit = jest.fn(async () => {
      calls += 1;
      if (calls === 1) {
        throw new TransactionSubmitError({
          stage: "post_submit",
          hash: "hash-1",
          cause: new Error("Request timed out"),
        });
      }
      return { hash: "hash-2" };
    });

    const { result } = renderHook(() =>
      useStellarTransaction<Params>({
        address: ADDRESS,
        createIntent: fixedIntent,
        refreshAccountState: jest.fn(async () => ACCOUNT_STATE),
        submit,
      })
    );

    await act(async () => {
      await result.current.run({ amount: "10" });
    });

    expect(result.current.status).toBe("ambiguous");

    await act(async () => {
      await result.current.confirmRetry();
    });

    expect(result.current.status).toBe("confirmed");
    expect(result.current.hash).toBe("hash-2");
    expect(result.current.requiresUserConfirmation).toBe(false);
    expect(submit).toHaveBeenCalledTimes(2);
  });

    it("prevents duplicate concurrent submissions", async () => {
    let resolveSubmit!: (value: { hash: string }) => void;

    const submission = new Promise<{ hash: string }>((resolve) => {
      resolveSubmit = resolve;
    });

    const submit = jest.fn(() => submission);

    const { result } = renderHook(() =>
      useStellarTransaction<Params>({
        address: ADDRESS,
        createIntent: fixedIntent,
        submit,
      })
    );

    let first!: ReturnType<typeof result.current.run>;
    let second!: ReturnType<typeof result.current.run>;

    act(() => {
      first = result.current.run({ amount: "10" });
      second = result.current.run({ amount: "10" });
    });

    // Resolve the pending submission so both calls can finish.
    await act(async () => {
      resolveSubmit({ hash: "hash-1" });
      await Promise.all([first, second]);
    });

    expect(submit).toHaveBeenCalledTimes(1);
    expect(result.current.status).toBe("confirmed");
    expect(result.current.hash).toBe("hash-1");
  });

  it("resets back to idle", async () => {
    const { result } = renderHook(() =>
      useStellarTransaction<Params>({
        address: ADDRESS,
        createIntent: fixedIntent,
        submit: jest.fn(async () => ({ hash: "hash-1" })),
      })
    );

    await act(async () => {
      await result.current.run({ amount: "10" });
    });
    expect(result.current.status).toBe("confirmed");

    act(() => {
      result.current.reset();
    });

    expect(result.current.status).toBe("idle");
    expect(result.current.hash).toBeUndefined();
    expect(result.current.intent).toBeNull();
  });


it("restores an interrupted transaction without resubmitting it", () => {
  setPersistedStellarTransaction({
    version: 1,
    address: ADDRESS,
    status: "running",
    intent: fixedIntent({ amount: "10" }),
    hash: "hash-pending",
    requiresUserConfirmation: false,
    accountState: null,
    updatedAt: Date.now(),
  });

  const submit = jest.fn(async () => ({ hash: "unexpected-hash" }));

  const { result } = renderHook(() =>
    useStellarTransaction<Params>({
      address: ADDRESS,
      submit,
    }),
  );

  expect(result.current.status).toBe("ambiguous");
  expect(result.current.intent?.id).toBe("intent-1");
  expect(result.current.hash).toBe("hash-pending");
  expect(result.current.requiresUserConfirmation).toBe(true);
  expect(submit).not.toHaveBeenCalled();
});

it("does not start a new intent while a transaction is ambiguous", async () => {
  const submit = jest.fn(async () => {
    throw new TransactionSubmitError({
      stage: "post_submit",
      hash: "hash-ambiguous",
      cause: new Error("Request timed out"),
    });
  });

  const { result } = renderHook(() =>
    useStellarTransaction<Params>({
      address: ADDRESS,
      createIntent: fixedIntent,
      refreshAccountState: jest.fn(async () => ACCOUNT_STATE),
      submit,
    }),
  );

  await act(async () => {
    await result.current.run({ amount: "10" });
  });

  expect(result.current.status).toBe("ambiguous");

  await act(async () => {
    await expect(
      result.current.run({ amount: "20" }),
    ).rejects.toThrow("An unresolved transaction exists");
  });

  expect(submit).toHaveBeenCalledTimes(1);
  expect(result.current.intent?.params.amount).toBe("10");
});

it("clears persisted state when reset is called", async () => {
  const { result } = renderHook(() =>
    useStellarTransaction<Params>({
      address: ADDRESS,
      createIntent: fixedIntent,
      submit: jest.fn(async () => ({ hash: "hash-1" })),
    }),
  );

  await act(async () => {
    await result.current.run({ amount: "10" });
  });

  expect(
    getPersistedStellarTransaction(ADDRESS),
  ).not.toBeNull();

  act(() => {
    result.current.reset();
  });

  expect(result.current.status).toBe("idle");
  expect(
    getPersistedStellarTransaction(ADDRESS),
  ).toBeNull();
});

it("keeps an in-flight transaction associated with its original wallet", async () => {
  const OTHER_ADDRESS = "GANOTHERWALLET";

  let resolveSubmit!: (value: { hash: string }) => void;

  const submission = new Promise<{ hash: string }>((resolve) => {
    resolveSubmit = resolve;
  });

  const submit = jest.fn(() => submission);

  const { result, rerender } = renderHook(
    ({ address }: { address: string }) =>
      useStellarTransaction<Params>({
        address,
        createIntent: fixedIntent,
        submit,
      }),
    {
      initialProps: { address: ADDRESS },
    },
  );

  let pending!: Promise<unknown>;

  act(() => {
    pending = result.current.run({ amount: "10" });
  });

  expect(submit).toHaveBeenCalledTimes(1);

  // Switch wallets while the first wallet's transaction is pending.
  rerender({ address: OTHER_ADDRESS });

  await act(async () => {
    resolveSubmit({ hash: "hash-original-wallet" });
    await pending;
  });

  expect(
    getPersistedStellarTransaction(ADDRESS),
  ).toMatchObject({
    address: ADDRESS,
    status: "confirmed",
    hash: "hash-original-wallet",
  });

  expect(
    getPersistedStellarTransaction(OTHER_ADDRESS),
  ).toBeNull();

  // The old transaction must not overwrite the new wallet's UI.
  expect(result.current.status).toBe("idle");
  expect(result.current.hash).toBeUndefined();
});

it("restores a confirmed transaction without resubmitting", () => {
  setPersistedStellarTransaction({
    version: 1,
    address: ADDRESS,
    status: "confirmed",
    intent: fixedIntent({ amount: "10" }),
    hash: "hash-confirmed",
    requiresUserConfirmation: false,
    accountState: ACCOUNT_STATE,
    updatedAt: Date.now(),
  });

  const submit = jest.fn(async () => ({ hash: "unexpected-hash" }));

  const { result } = renderHook(() =>
    useStellarTransaction<Params>({
      address: ADDRESS,
      submit,
    }),
  );

  expect(result.current.status).toBe("confirmed");
  expect(result.current.hash).toBe("hash-confirmed");
  expect(result.current.requiresUserConfirmation).toBe(false);
  expect(submit).not.toHaveBeenCalled();
});

it("restores a failed transaction and its error without resubmitting", () => {
  setPersistedStellarTransaction({
    version: 1,
    address: ADDRESS,
    status: "failed",
    intent: fixedIntent({ amount: "10" }),
    requiresUserConfirmation: false,
    accountState: null,
    error: {
      code: "USER_REJECTED",
      title: "Transaction rejected",
      message: "The user rejected the wallet request.",
      recoveryAction: "Try again when ready.",
    },
    updatedAt: Date.now(),
  });

  const submit = jest.fn(async () => ({ hash: "unexpected-hash" }));

  const { result } = renderHook(() =>
    useStellarTransaction<Params>({
      address: ADDRESS,
      submit,
    }),
  );

  expect(result.current.status).toBe("failed");
  expect(result.current.error?.code).toBe("USER_REJECTED");
  expect(result.current.error?.message).toBe(
    "The user rejected the wallet request.",
  );
  expect(submit).not.toHaveBeenCalled();
});
});
