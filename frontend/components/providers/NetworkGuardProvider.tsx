"use client";

import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  type ReactNode,
} from "react";
import { useNetworkGuard } from "@/lib/hooks/useNetworkGuard";
import { NetworkSwitchModal } from "@/components/modals/NetworkSwitchModal";

export interface NetworkGuardContextValue {
  isMismatch: boolean;
  expectedNetwork: string | null;
  expectedContractEnv: string | null;
  recheckNetwork: () => Promise<void> | void;
}

const NetworkGuardContext = createContext<NetworkGuardContextValue | null>(null);

export function useNetworkGuardContext(): NetworkGuardContextValue {
  const context = useContext(NetworkGuardContext);
  if (!context) {
    throw new Error(
      "useNetworkGuardContext must be used within a NetworkGuardProvider",
    );
  }
  return context;
}

export function NetworkGuardProvider({ children }: { children: ReactNode }) {
  const {
    isWrongNetwork,
    currentChainName,
    switchNetwork,
    switchError,
    switchRecoveryAction,
    expectedNetwork,
    expectedContractEnv,
    recheckNetwork,
  } = useNetworkGuard();

  const handleSwitch = useCallback(async () => {
    await switchNetwork();
    await recheckNetwork();
  }, [switchNetwork, recheckNetwork]);

  const contextValue = useMemo<NetworkGuardContextValue>(
    () => ({
      isMismatch: isWrongNetwork,
      expectedNetwork: expectedNetwork ?? null,
      expectedContractEnv: expectedContractEnv ?? null,
      recheckNetwork,
    }),
    [isWrongNetwork, expectedNetwork, expectedContractEnv, recheckNetwork],
  );

  return (
    <NetworkGuardContext.Provider value={contextValue}>
      {children}
      <NetworkSwitchModal
        isOpen={isWrongNetwork}
        currentChainName={currentChainName}
        expectedNetwork={expectedNetwork ?? null}
        expectedContractEnv={expectedContractEnv ?? null}
        onSwitch={handleSwitch}
        onRecheck={recheckNetwork}
        switchError={switchError}
        switchRecoveryAction={switchRecoveryAction}
      />
    </NetworkGuardContext.Provider>
  );
}
</｜｜DSML｜｜ parameter>