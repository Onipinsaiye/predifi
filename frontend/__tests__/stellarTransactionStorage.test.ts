
import {
  getPersistedStellarTransaction,
  removePersistedStellarTransaction,
  setPersistedStellarTransaction,
  type PersistedStellarTransaction,
} from "@/lib/stellarTransactionStorage";

const ADDRESS = "GTESTWALLET123";
const STORAGE_KEY = `predifi:stellar-transaction:${encodeURIComponent(ADDRESS)}`;

function createTransaction(
  overrides: Partial<PersistedStellarTransaction<string>> = {},
): PersistedStellarTransaction<string> {
  return {
    version: 1,
    address: ADDRESS,
    status: "running",
    intent: {
      id: "stellar-tx-test-1",
      params: "create-pool",
      createdAt: Date.now(),
    },
    requiresUserConfirmation: false,
    accountState: null,
    updatedAt: Date.now(),
    ...overrides,
  };
}

describe("stellarTransactionStorage", () => {
  beforeEach(() => {
    localStorage.clear();
    jest.restoreAllMocks();
  });

  it("persists and restores a transaction", () => {
    const transaction = createTransaction({
      status: "confirmed",
      hash: "hash-123",
    });

    expect(setPersistedStellarTransaction(transaction)).toBe(true);

    expect(
      getPersistedStellarTransaction<string>(ADDRESS),
    ).toMatchObject({
      address: ADDRESS,
      status: "confirmed",
      hash: "hash-123",
      intent: {
        id: "stellar-tx-test-1",
        params: "create-pool",
      },
    });
  });

  it("restores an interrupted running transaction as ambiguous", () => {
    const transaction = createTransaction({
      status: "running",
      hash: "hash-pending",
    });

    setPersistedStellarTransaction(transaction);

    const restored =
      getPersistedStellarTransaction<string>(ADDRESS);

    expect(restored).toMatchObject({
      status: "ambiguous",
      hash: "hash-pending",
      requiresUserConfirmation: true,
      error: {
        code: "UNKNOWN",
        title: "Transaction status unknown",
      },
    });
  });

  it("does not restore a transaction belonging to another wallet", () => {
    setPersistedStellarTransaction(createTransaction());

    expect(
      getPersistedStellarTransaction("GANOTHERWALLET"),
    ).toBeNull();
  });

  it("removes expired transactions", () => {
    const sevenDaysAndOneSecondAgo =
      Date.now() - (7 * 24 * 60 * 60 * 1000 + 1000);

    setPersistedStellarTransaction(
      createTransaction({
        updatedAt: sevenDaysAndOneSecondAgo,
      }),
    );

    expect(
      getPersistedStellarTransaction(ADDRESS),
    ).toBeNull();

    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("removes malformed JSON without throwing", () => {
    localStorage.setItem(STORAGE_KEY, "{not valid JSON");

    expect(() =>
      getPersistedStellarTransaction(ADDRESS),
    ).not.toThrow();

    expect(
      getPersistedStellarTransaction(ADDRESS),
    ).toBeNull();

    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("removes the transaction when explicitly requested", () => {
    setPersistedStellarTransaction(createTransaction());

    removePersistedStellarTransaction(ADDRESS);

    expect(
      getPersistedStellarTransaction(ADDRESS),
    ).toBeNull();
  });

  it("handles unavailable storage without throwing", () => {
    jest
      .spyOn(Storage.prototype, "setItem")
      .mockImplementation(() => {
        throw new Error("Storage unavailable");
      });

    expect(
      setPersistedStellarTransaction(createTransaction()),
    ).toBe(false);
  });
});
