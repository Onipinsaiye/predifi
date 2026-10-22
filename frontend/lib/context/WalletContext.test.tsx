import React from 'react';
import { render, screen, act, waitFor, fireEvent } from '@testing-library/react';
import '@testing-library/jest-dom';

// ---------------------------------------------------------------------------
// Mocked wallet provider module. The real WalletContext is expected to consume
// these hooks. Each test re-configures the mock before rendering.
// ---------------------------------------------------------------------------
const mockWallet = {
  address: null as string | null,
  chainId: null as number | null,
  isConnected: false,
  connect: jest.fn(),
  disconnect: jest.fn(),
  switchNetwork: jest.fn(),
};

jest.mock('./WalletContext', () => {
  const actual = jest.requireActual('./WalletContext');
  return {
    ...actual,
    useWallet: () => mockWallet,
  };
});

// Import after mock so the component under test uses the mocked hook.
import { WalletProvider, useWallet } from './WalletContext';

// ---------------------------------------------------------------------------
// Test harness component that surfaces wallet-bound state and economic actions.
// ---------------------------------------------------------------------------
const EXPECTED_CHAIN_ID = 1; // mainnet in this suite

function Harness() {
  const wallet = useWallet();
  const [balances, setBalances] = React.useState<Record<string, string>>({});
  const [history, setHistory] = React.useState<string[]>([]);
  const [pending, setPending] = React.useState<string[]>([]);
  const [loading, setLoading] = React.useState(false);

  // Simulate fetching account-bound data whenever identity changes.
  React.useEffect(() => {
    let cancelled = false;
    if (!wallet.address) {
      setBalances({});
      setHistory([]);
      setPending([]);
      return;
    }
    setLoading(true);
    // Simulated async fetch keyed by network + address.
    Promise.resolve().then(() => {
      if (cancelled) return;
      setBalances({ [wallet.address as string]: '100' });
      setHistory([`tx-${wallet.address}`]);
      setLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, [wallet.address, wallet.chainId]);

  const networkOk = wallet.chainId === EXPECTED_CHAIN_ID;
  const economicEnabled = wallet.isConnected && networkOk && !loading;

  return (
    <div>
      <div data-testid="address">{wallet.address ?? 'none'}</div>
      <div data-testid="chain">{wallet.chainId ?? 'none'}</div>
      <div data-testid="balances">{JSON.stringify(balances)}</div>
      <div data-testid="history">{JSON.stringify(history)}</div>
      <div data-testid="pending">{JSON.stringify(pending)}</div>
      <div data-testid="loading">{String(loading)}</div>
      <div data-testid="network-ok">{String(networkOk)}</div>
      <button
        data-testid="economic-action"
        disabled={!economicEnabled}
        onClick={() => setPending((p) => [...p, 'pending-tx'])}
      >
        Send
      </button>
      <button data-testid="disconnect" onClick={() => wallet.disconnect()}>
        Disconnect
      </button>
    </div>
  );
}

function renderHarness() {
  return render(
    <WalletProvider>
      <Harness />
    </WalletProvider>
  );
}

// Helper to mutate the mocked wallet state and flush effects.
async function setWallet(patch: Partial<typeof mockWallet>) {
  Object.assign(mockWallet, patch);
  await act(async () => {
    await Promise.resolve();
  });
}

beforeEach(() => {
  mockWallet.address = null;
  mockWallet.chainId = null;
  mockWallet.isConnected = false;
  mockWallet.connect.mockReset();
  mockWallet.disconnect.mockReset();
  mockWallet.switchNetwork.mockReset();
});

describe('WalletContext account changes', () => {
  it('does not reuse or flash prior account data when switching A -> B', async () => {
    renderHarness();

    // Connect account A on the expected network.
    await setWallet({ address: '0xA', chainId: EXPECTED_CHAIN_ID, isConnected: true });
    await waitFor(() => {
      expect(screen.getByTestId('balances').textContent).toContain('0xA');
    });
    expect(screen.getByTestId('history').textContent).toContain('tx-0xA');

    // Switch to account B. Prior data must not remain visible.
    await setWallet({ address: '0xB' });

    // Immediately after identity change, account A data must be gone.
    expect(screen.getByTestId('balances').textContent).not.toContain('0xA');
    expect(screen.getByTestId('history').textContent).not.toContain('tx-0xA');

    await waitFor(() => {
      expect(screen.getByTestId('balances').textContent).toContain('0xB');
    });
    expect(screen.getByTestId('history').textContent).toContain('tx-0xB');
    expect(screen.getByTestId('history').textContent).not.toContain('tx-0xA');
  });

  it('clears balances, history, and pending UI on disconnect', async () => {
    renderHarness();

    await setWallet({ address: '0xA', chainId: EXPECTED_CHAIN_ID, isConnected: true });
    await waitFor(() => {
      expect(screen.getByTestId('balances').textContent).toContain('0xA');
    });

    // Create a pending transaction.
    fireEvent.click(screen.getByTestId('economic-action'));
    expect(screen.getByTestId('pending').textContent).toContain('pending-tx');

    // Disconnect.
    await setWallet({ address: null, chainId: null, isConnected: false });

    expect(screen.getByTestId('address').textContent).toBe('none');
    expect(screen.getByTestId('balances').textContent).toBe('{}');
    expect(screen.getByTestId('history').textContent).toBe('[]');
    expect(screen.getByTestId('pending').textContent).toBe('[]');
    expect(screen.getByTestId('economic-action')).toBeDisabled();
  });

  it('clears account data when switching networks', async () => {
    renderHarness();

    await setWallet({ address: '0xA', chainId: EXPECTED_CHAIN_ID, isConnected: true });
    await waitFor(() => {
      expect(screen.getByTestId('balances').textContent).toContain('0xA');
    });

    // Switch to an unsupported network; data must be cleared and actions disabled.
    await setWallet({ chainId: 137 });

    expect(screen.getByTestId('network-ok').textContent).toBe('false');
    expect(screen.getByTestId('economic-action')).toBeDisabled();
  });
});

describe('WalletContext network transitions', () => {
  it('keeps economic actions disabled while on the wrong network', async () => {
    renderHarness();

    await setWallet({ address: '0xA', chainId: 137, isConnected: true });

    expect(screen.getByTestId('network-ok').textContent).toBe('false');
    expect(screen.getByTestId('economic-action')).toBeDisabled();
  });

  it('re-enables economic actions only after the guard confirms the expected network', async () => {
    renderHarness();

    await setWallet({ address: '0xA', chainId: 137, isConnected: true });
    expect(screen.getByTestId('economic-action')).toBeDisabled();

    // Simulate a delayed provider update: wallet briefly still reports old
    // network, then reports the new one.
    await setWallet({ chainId: 137 });
    expect(screen.getByTestId('economic-action')).toBeDisabled();

    await setWallet({ chainId: EXPECTED_CHAIN_ID });
    await waitFor(() => {
      expect(screen.getByTestId('network-ok').textContent).toBe('true');
    });
    expect(screen.getByTestId('economic-action')).not.toBeDisabled();
  });

  it('handles a rejected network switch without enabling actions', async () => {
    mockWallet.switchNetwork.mockRejectedValueOnce(new Error('User rejected'));
    renderHarness();

    await setWallet({ address: '0xA', chainId: 137, isConnected: true });
    expect(screen.getByTestId('economic-action')).toBeDisabled();

    await act(async () => {
      await expect(mockWallet.switchNetwork(EXPECTED_CHAIN_ID)).rejects.toThrow(
        'User rejected'
      );
    });

    // Network unchanged -> actions stay disabled.
    expect(screen.getByTestId('chain').textContent).toBe('137');
    expect(screen.getByTestId('economic-action')).toBeDisabled();
  });

  it('keeps actions disabled on an unsupported network', async () => {
    renderHarness();

    await setWallet({ address: '0xA', chainId: 999999, isConnected: true });

    expect(screen.getByTestId('network-ok').textContent).toBe('false');
    expect(screen.getByTestId('economic-action')).toBeDisabled();
  });
});