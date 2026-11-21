'use client';

import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';

/**
 * Demo mode context.
 *
 * Provides a read-only, fully local "demo" experience so users can explore the
 * interface before connecting a wallet. Every value surfaced from this context
 * is explicitly marked as illustrative, all interactions stay local (no network
 * requests), and wallet signing is prohibited while demo mode is active.
 */

export interface IllustrativeValue<T = unknown> {
  /** Always true — flags the value as illustrative, never live data. */
  readonly illustrative: true;
  /** Human-readable label suitable for UI badges. */
  readonly label: string;
  /** The illustrative value itself. */
  readonly value: T;
}

export interface DemoMarket {
  id: string;
  question: string;
  category: string;
  /** Illustrative probability (0-1). */
  probability: number;
  /** Illustrative volume in USD. */
  volumeUsd: number;
  /** Illustrative liquidity in USD. */
  liquidityUsd: number;
  closesAt: string;
  resolved: boolean;
}

export interface DemoFixtures {
  markets: DemoMarket[];
  portfolio: {
    balanceUsd: number;
    openPositions: number;
    totalPnlUsd: number;
  };
  activity: Array<{
    id: string;
    type: 'trade' | 'deposit' | 'withdrawal';
    description: string;
    amountUsd: number;
    timestamp: string;
  }>;
}

/** Wrap a raw value so it is unambiguously marked as illustrative. */
export function illustrative<T>(label: string, value: T): IllustrativeValue<T> {
  return { illustrative: true, label, value };
}

const DEMO_MARKETS: DemoMarket[] = [
  {
    id: 'demo-market-1',
    question: 'Will the demo team win the illustrative championship?',
    category: 'Sports (demo)',
    probability: 0.62,
    volumeUsd: 125000,
    liquidityUsd: 42000,
    closesAt: '2030-01-01T00:00:00.000Z',
    resolved: false,
  },
  {
    id: 'demo-market-2',
    question: 'Will sample metric exceed 50% by year end?',
    category: 'Economics (demo)',
    probability: 0.41,
    volumeUsd: 88000,
    liquidityUsd: 21000,
    closesAt: '2030-06-30T00:00:00.000Z',
    resolved: false,
  },
  {
    id: 'demo-market-3',
    question: 'Will the example proposal pass?',
    category: 'Politics (demo)',
    probability: 0.78,
    volumeUsd: 210000,
    liquidityUsd: 65000,
    closesAt: '2029-12-31T00:00:00.000Z',
    resolved: false,
  },
];

const DEMO_FIXTURES: DemoFixtures = {
  markets: DEMO_MARKETS,
  portfolio: {
    balanceUsd: 1000,
    openPositions: 3,
    totalPnlUsd: 42.5,
  },
  activity: [
    {
      id: 'demo-activity-1',
      type: 'trade',
      description: 'Bought YES on demo market #1 (illustrative)',
      amountUsd: 50,
      timestamp: '2030-01-01T12:00:00.000Z',
    },
    {
      id: 'demo-activity-2',
      type: 'deposit',
      description: 'Illustrative demo deposit',
      amountUsd: 1000,
      timestamp: '2030-01-01T11:00:00.000Z',
    },
  ],
};

export interface DemoModeContextValue {
  /** True when the app is in read-only demo mode. */
  isDemo: boolean;
  /** Enter demo mode. All interactions become local and signing is blocked. */
  enterDemo: () => void;
  /** Exit demo mode and return to live data. */
  exitDemo: () => void;
  /** Illustrative fixtures for demo exploration. */
  fixtures: DemoFixtures;
  /** Convenience: fixtures wrapped as illustrative values. */
  illustrativeFixtures: {
    markets: IllustrativeValue<DemoMarket[]>;
    portfolio: IllustrativeValue<DemoFixtures['portfolio']>;
    activity: IllustrativeValue<DemoFixtures['activity']>;
  };
  /**
   * Guard for wallet signing. Returns true when signing is allowed.
   * In demo mode this always returns false and records the blocked attempt.
   */
  canSign: () => boolean;
  /**
   * Intercept a wallet signing request. In demo mode the request is rejected
   * locally and never forwarded to the wallet.
   */
  guardSigning: <T>(action: () => Promise<T>) => Promise<T>;
  /** Number of signing attempts blocked while in demo mode. */
  blockedSigningAttempts: number;
  /** Last reason a signing attempt was blocked, if any. */
  lastBlockedReason: string | null;
}

const DemoModeContext = createContext<DemoModeContextValue | null>(null);

const DEMO_STORAGE_KEY = 'pm.demoMode';

function readInitialDemoState(): boolean {
  if (typeof window === 'undefined') return false;
  try {
    return window.sessionStorage.getItem(DEMO_STORAGE_KEY) === 'true';
  } catch {
    return false;
  }
}

export interface DemoModeProviderProps {
  children: React.ReactNode;
  /** Optional initial state override (useful for tests). */
  initialDemo?: boolean;
}

export function DemoModeProvider({
  children,
  initialDemo,
}: DemoModeProviderProps) {
  const [isDemo, setIsDemo] = useState<boolean>(() =>
    typeof initialDemo === 'boolean' ? initialDemo : readInitialDemoState(),
  );
  const [blockedSigningAttempts, setBlockedSigningAttempts] = useState(0);
  const [lastBlockedReason, setLastBlockedReason] = useState<string | null>(
    null,
  );

  // Keep a ref so guards always see the latest value synchronously.
  const isDemoRef = useRef(isDemo);
  isDemoRef.current = isDemo;

  // Persist demo state for the session so route transitions keep the mode.
  useEffect(() => {
    if (typeof window === 'undefined') return;
    try {
      if (isDemo) {
        window.sessionStorage.setItem(DEMO_STORAGE_KEY, 'true');
      } else {
        window.sessionStorage.removeItem(DEMO_STORAGE_KEY);
      }
    } catch {
      // Storage may be unavailable (private mode); demo state stays in memory.
    }
  }, [isDemo]);

  const enterDemo = useCallback(() => {
    setIsDemo(true);
  }, []);

  const exitDemo = useCallback(() => {
    setIsDemo(false);
    setBlockedSigningAttempts(0);
    setLastBlockedReason(null);
  }, []);

  const canSign = useCallback(() => !isDemoRef.current, []);

  const guardSigning = useCallback(
    async <T,>(action: () => Promise<T>): Promise<T> => {
      if (isDemoRef.current) {
        const reason =
          'Wallet signing is disabled in demo mode. Exit demo to use live data.';
        setBlockedSigningAttempts((n) => n + 1);
        setLastBlockedReason(reason);
        throw new Error(reason);
      }
      return action();
    },
    [],
  );

  const illustrativeFixtures = useMemo(
    () => ({
      markets: illustrative('Illustrative demo markets', DEMO_FIXTURES.markets),
      portfolio: illustrative(
        'Illustrative demo portfolio',
        DEMO_FIXTURES.portfolio,
      ),
      activity: illustrative(
        'Illustrative demo activity',
        DEMO_FIXTURES.activity,
      ),
    }),
    [],
  );

  const value = useMemo<DemoModeContextValue>(
    () => ({
      isDemo,
      enterDemo,
      exitDemo,
      fixtures: DEMO_FIXTURES,
      illustrativeFixtures,
      canSign,
      guardSigning,
      blockedSigningAttempts,
      lastBlockedReason,
    }),
    [
      isDemo,
      enterDemo,
      exitDemo,
      illustrativeFixtures,
      canSign,
      guardSigning,
      blockedSigningAttempts,
      lastBlockedReason,
    ],
  );

  return (
    <DemoModeContext.Provider value={value}>
      {children}
    </DemoModeContext.Provider>
  );
}

export function useDemoMode(): DemoModeContextValue {
  const ctx = useContext(DemoModeContext);
  if (!ctx) {
    throw new Error('useDemoMode must be used within a DemoModeProvider');
  }
  return ctx;
}

/**
 * Route guard helper.
 *
 * Use this to ensure demo state never leaks into API requests. When demo mode
 * is active, any network-bound action is short-circuited and the provided
 * local fallback is returned instead. This keeps demo interactions local.
 */
export function useDemoRouteGuard() {
  const { isDemo } = useDemoMode();

  const guardRequest = useCallback(
    async <T,>(request: () => Promise<T>, localFallback: () => T | Promise<T>): Promise<T> => {
      if (isDemo) {
        return localFallback();
      }
      return request();
    },
    [isDemo],
  );

  /**
   * Returns true when a request to the given path should be blocked because
   * demo mode is active. Useful for intercepting fetch/axios calls.
   */
  const shouldBlockRequest = useCallback(
    (path?: string) => {
      if (!isDemo) return false;
      // In demo mode, all API-bound requests are blocked.
      return true;
    },
    [isDemo],
  );

  return { isDemo, guardRequest, shouldBlockRequest };
}

/**
 * Standalone (non-hook) guard for use outside React components, e.g. in an
 * API client. Pass the current demo flag from the provider.
 */
export function createDemoRequestGuard(getIsDemo: () => boolean) {
  return function guardRequest<T>(
    request: () => Promise<T>,
    localFallback: () => T | Promise<T>,
  ): Promise<T> {
    if (getIsDemo()) {
      return Promise.resolve(localFallback());
    }
    return request();
  };
}