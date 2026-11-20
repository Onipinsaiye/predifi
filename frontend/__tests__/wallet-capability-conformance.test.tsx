import { act, renderHook } from "@testing-library/react";
import { handleTransactionError, useWalletConnection } from "@/lib/hooks/useWalletConnection";
import { REQUIRED_STELLAR_NETWORK, type WalletErrorCode } from "@/lib/walletErrors";

type WalletApi = {
  getNetwork?: () => Promise<string>;
  getNetworkDetails?: () => Promise<{ network: string }>;
  getPublicKey?: () => Promise<string>;
  requestAccess?: () => Promise<string>;
};

type WalletApiKey = "freighter" | "freighterApi";

type WalletFixture = {
  name: string;
  apiKey: WalletApiKey;
  createApi: () => WalletApi;
};

const ADDRESS = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

const fixtures: WalletFixture[] = [
  {
    name: "Freighter API",
    apiKey: "freighterApi",
    createApi: () => ({
      getNetworkDetails: async () => ({ network: "TESTNET" }),
      requestAccess: async () => ADDRESS,
    }),
  },
  {
    name: "Freighter legacy API",
    apiKey: "freighter",
    createApi: () => ({
      getNetwork: async () => "TESTNET",
      getPublicKey: async () => ADDRESS,
    }),
  },
  {
    name: "deterministic mock wallet",
    apiKey: "freighterApi",
    createApi: () => ({
      getNetworkDetails: async () => ({ network: "TESTNET" }),
      requestAccess: async () => ADDRESS,
    }),
  },
];

function installWallet(apiKey: WalletApiKey, api?: WalletApi) {
  const walletWindow = window as Window & {
    freighter?: WalletApi;
    freighterApi?: WalletApi;
  };
  delete walletWindow.freighter;
  delete walletWindow.freighterApi;
  if (api) walletWindow[apiKey] = api;
}

describe.each(fixtures)("$name capability conformance", (fixture) => {
  let api: WalletApi;

  beforeEach(() => {
    api = fixture.createApi();
    installWallet(fixture.apiKey, api);
  });

  afterEach(() => installWallet(fixture.apiKey));

  it("connects and records the reported network and address", async () => {
    const { result } = renderHook(() => useWalletConnection());

    await act(async () => {
      await result.current.connect();
    });

    expect(result.current.address).toBe(ADDRESS);
    expect(result.current.network).toBe("TESTNET");
    expect(result.current.identityKey).toBe(`TESTNET:${ADDRESS}`);
    expect(result.current.error).toBeNull();
    expect(result.current.isConnecting).toBe(false);
  });

  it("rejects a connection when the reported network does not match", async () => {
    const wrongNetwork = fixture.createApi();
    const network = REQUIRED_STELLAR_NETWORK === "TESTNET" ? "PUBLIC" : "TESTNET";
    wrongNetwork.getNetworkDetails = async () => ({ network });
    wrongNetwork.getNetwork = async () => network;
    installWallet(fixture.apiKey, wrongNetwork);
    const requestAccess = jest.fn(async () => ADDRESS);
    wrongNetwork.requestAccess = requestAccess;
    const { result } = renderHook(() => useWalletConnection());

    await act(async () => {
      await result.current.connect();
    });

    expect(result.current.address).toBeNull();
    expect(result.current.error?.code).toBe("NETWORK_MISMATCH");
    expect(requestAccess).not.toHaveBeenCalled();
  });

  it("maps a rejected connection request to USER_REJECTED", async () => {
    api.requestAccess = async () => {
      throw new Error("User rejected the request");
    };
    installWallet(fixture.apiKey, api);
    const { result } = renderHook(() => useWalletConnection());

    await act(async () => {
      await result.current.connect();
    });

    expect(result.current.address).toBeNull();
    expect(result.current.error?.code).toBe("USER_REJECTED");
  });

  it("clears the address, network, and error on disconnect", async () => {
    const { result } = renderHook(() => useWalletConnection());

    await act(async () => {
      await result.current.connect();
    });
    act(() => result.current.disconnect());

    expect(result.current.address).toBeNull();
    expect(result.current.network).toBeNull();
    expect(result.current.identityKey).toBeNull();
    expect(result.current.error).toBeNull();
  });

  it("clears a connection refusal on disconnect", async () => {
    api.requestAccess = async () => {
      throw new Error("User rejected the request");
    };
    installWallet(fixture.apiKey, api);
    const { result } = renderHook(() => useWalletConnection());

    await act(async () => {
      await result.current.connect();
    });
    expect(result.current.error?.code).toBe("USER_REJECTED");

    act(() => result.current.disconnect());

    expect(result.current.error).toBeNull();
    expect(result.current.address).toBeNull();
  });

  it("handles absent network methods without throwing", async () => {
    installWallet(fixture.apiKey, { requestAccess: async () => ADDRESS });
    const { result } = renderHook(() => useWalletConnection());

    await act(async () => {
      await result.current.connect();
    });

    expect(result.current.address).toBe(ADDRESS);
    let network: string | null = ADDRESS;
    await act(async () => {
      network = await result.current.refreshNetwork();
    });
    expect(network).toBeNull();
  });

  it("reports unsupported access methods as a missing wallet capability", async () => {
    installWallet(fixture.apiKey, {});
    const { result } = renderHook(() => useWalletConnection());

    await act(async () => {
      await result.current.connect();
    });

    expect(result.current.address).toBeNull();
    expect(result.current.error?.code).toBe("EXTENSION_NOT_INSTALLED");
  });

  it("classifies signing refusal consistently", () => {
    const error = handleTransactionError(new Error("User rejected signature"));
    const code: WalletErrorCode = error.code;
    expect(code).toBe("USER_REJECTED");
  });
});
