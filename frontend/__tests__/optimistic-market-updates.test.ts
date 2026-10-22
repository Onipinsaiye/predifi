import { renderHook, waitFor } from "@testing-library/react";
import { useOptimisticAction } from "@/lib/hooks/useOptimisticAction";
import type { PoolDetail } from "@/lib/api/pools";

/**
 * Tests for useOptimisticAction in market update scenarios (#72).
 * Covers concurrent updates, route changes, stale responses, and preventing
 * data reversion with newer confirmed data.
 */
describe("useOptimisticAction - Market Update Scenarios (#72)", () => {
  const mockPool: PoolDetail = {
    pool_id: 1,
    name: "Test Pool",
    category: "Sports",
    tags: ["test"],
    total_stake: 1000,
    end_time: Math.floor(Date.now() / 1000) + 3600,
    created_at: new Date().toISOString(),
    state: "active",
    creator: "0xabc123",
    token: "XYZ",
    result: null,
    odds: [
      { outcome: 0, stake: 600, odds: 1.67 },
      { outcome: 1, stake: 400, odds: 2.5 },
    ],
    prediction_count: 10,
  };

  describe("single market optimistic update", () => {
    it("applies optimistic stake update immediately", async () => {
      const commit = jest.fn().mockResolvedValue(undefined);

      const { result } = renderHook(() =>
        useOptimisticAction({
          data: mockPool,
          applyOptimistic: (current, input: { amount: number; outcome: number }) => ({
            ...current,
            total_stake: current.total_stake + input.amount,
            odds: current.odds.map((o) =>
              o.outcome === input.outcome
                ? { ...o, stake: o.stake + input.amount }
                : o
            ),
          }),
          commit,
        })
      );

      // Initial state
      expect(result.current.value.total_stake).toBe(1000);

      // Apply optimistic update
      const prediction = { amount: 100, outcome: 0 };
      result.current.run(prediction);

      // Optimistic value applied immediately
      expect(result.current.value.total_stake).toBe(1100);
      expect(result.current.isPending).toBe(true);

      await waitFor(() => {
        expect(result.current.status).toBe("success");
      });

      // After commit, state remains optimistic
      expect(result.current.value.total_stake).toBe(1100);
    });

    it("rolls back on rejection and preserves original state", async () => {
      const commit = jest
        .fn()
        .mockRejectedValue(new Error("Transaction failed"));

      const { result } = renderHook(() =>
        useOptimisticAction({
          data: mockPool,
          applyOptimistic: (current, input: { amount: number; outcome: number }) => ({
            ...current,
            total_stake: current.total_stake + input.amount,
          }),
          commit,
        })
      );

      const prediction = { amount: 100, outcome: 0 };
      result.current.run(prediction);

      expect(result.current.value.total_stake).toBe(1100);

      await waitFor(() => {
        expect(result.current.status).toBe("error");
      });

      // Rolled back to original
      expect(result.current.value.total_stake).toBe(1000);
    });
  });

  describe("concurrent market updates", () => {
    it("handles overlapping predictions on same market", async () => {
      let commitCount = 0;
      const commit = jest.fn().mockImplementation(() => {
        commitCount++;
        return Promise.resolve(undefined);
      });

      const { result } = renderHook(() =>
        useOptimisticAction({
          data: mockPool,
          applyOptimistic: (current, input: { amount: number; outcome: number }) => ({
            ...current,
            total_stake: current.total_stake + input.amount,
            prediction_count: (current.prediction_count ?? 10) + 1,
          }),
          commit,
        })
      );

      // Fire first prediction
      const first = result.current.run({ amount: 100, outcome: 0 });
      expect(result.current.value.total_stake).toBe(1100);
      expect(result.current.value.prediction_count).toBe(11);

      // Fire second prediction while first is pending
      const second = result.current.run({ amount: 50, outcome: 1 });
      expect(result.current.value.total_stake).toBe(1150);
      expect(result.current.value.prediction_count).toBe(12);

      await Promise.all([first, second]);

      await waitFor(() => {
        expect(result.current.status).toBe("success");
      });

      expect(result.current.value.total_stake).toBe(1150);
    });

    it("prevents data reversion when newer commit comes after stale", async () => {
      let commitIndex = 0;
      const commits = [
        { amount: 100, stake: 1100, seq: 1 },
        { amount: 50, stake: 1150, seq: 2 },
      ];

      const commit = jest.fn().mockImplementation(async (input) => {
        const result = commits[commitIndex];
        commitIndex++;
        return { ...mockPool, total_stake: result.stake };
      });

      const { result } = renderHook(() =>
        useOptimisticAction({
          data: mockPool,
          applyOptimistic: (current, input: { amount: number }) => ({
            ...current,
            total_stake: current.total_stake + input.amount,
          }),
          commit,
        })
      );

      // First update
      result.current.run({ amount: 100 });
      expect(result.current.value.total_stake).toBe(1100);

      // Wait for first to settle
      await waitFor(() => expect(result.current.status).toBe("success"));

      // Second update
      result.current.run({ amount: 50 });
      expect(result.current.value.total_stake).toBe(1150);

      await waitFor(() => {
        // Should be 1150, never revert to 1100
        expect(result.current.value.total_stake).toBe(1150);
      });
    });

    it("handles same outcome multiple predictions", async () => {
      const commit = jest.fn().mockResolvedValue(undefined);

      const { result } = renderHook(() =>
        useOptimisticAction({
          data: mockPool,
          applyOptimistic: (current, input: { amount: number; outcome: number }) => ({
            ...current,
            total_stake: current.total_stake + input.amount,
            odds: current.odds.map((o) =>
              o.outcome === input.outcome
                ? { ...o, stake: o.stake + input.amount, odds: (current.total_stake + input.amount) / (o.stake + input.amount) }
                : o
            ),
          }),
          commit,
        })
      );

      // Multiple bets on outcome 0
      result.current.run({ amount: 100, outcome: 0 });
      result.current.run({ amount: 200, outcome: 0 });

      await waitFor(() => {
        expect(result.current.status).toBe("success");
      });

      // Outcome 0 stake should be 600 + 100 + 200 = 900
      const outcome0 = result.current.value.odds.find((o) => o.outcome === 0);
      expect(outcome0?.stake).toBe(900);
    });
  });

  describe("route changes and state preservation", () => {
    it("completes in-flight update even after navigation", async () => {
      let resolveCommit: ((value: PoolDetail) => void) | null = null;
      const commit = jest.fn(
        () =>
          new Promise<PoolDetail>((resolve) => {
            resolveCommit = resolve;
          })
      );

      const { result, unmount } = renderHook(() =>
        useOptimisticAction({
          data: mockPool,
          applyOptimistic: (current, input: { amount: number }) => ({
            ...current,
            total_stake: current.total_stake + input.amount,
          }),
          commit,
        })
      );

      result.current.run({ amount: 100 });
      expect(result.current.value.total_stake).toBe(1100);

      // Simulate route navigation (unmount)
      unmount();

      // Commit resolves after unmount
      const updatedPool = { ...mockPool, total_stake: 1100 };
      resolveCommit?.(updatedPool);

      // No state updates should occur after unmount (test passes if no errors)
    });

    it("does not restore old state when switching markets", async () => {
      const commit = jest.fn().mockResolvedValue(undefined);

      const { result, rerender } = renderHook(
        ({ pool }: { pool: PoolDetail }) =>
          useOptimisticAction({
            data: pool,
            applyOptimistic: (current, input: { amount: number }) => ({
              ...current,
              total_stake: current.total_stake + input.amount,
            }),
            commit,
          }),
        { initialProps: { pool: mockPool } }
      );

      result.current.run({ amount: 100 });
      expect(result.current.value.total_stake).toBe(1100);

      await waitFor(() => {
        expect(result.current.status).toBe("success");
      });

      // Switch to different pool
      const pool2: PoolDetail = { ...mockPool, pool_id: 2, total_stake: 2000 };
      rerender({ pool: pool2 });

      // New hook should start with pool2's state
      expect(result.current.value.total_stake).toBe(2000);
    });
  });

  describe("stale response handling", () => {
    it("ignores stale response if newer confirmed data exists", async () => {
      let responseSeq = 0;
      const stalePool = { ...mockPool, total_stake: 1100 };
      const freshPool = { ...mockPool, total_stake: 1150 };

      const commit = jest.fn().mockImplementation(async () => {
        responseSeq++;
        return responseSeq === 1 ? freshPool : stalePool;
      });

      const { result } = renderHook(() =>
        useOptimisticAction({
          data: mockPool,
          applyOptimistic: (current, input: { amount: number }) => ({
            ...current,
            total_stake: current.total_stake + input.amount,
          }),
          commit,
        })
      );

      // First update: gets fresh data (1150)
      const first = result.current.run({ amount: 150 });
      expect(result.current.value.total_stake).toBe(1150);

      // Second update fires but response comes out of order: gets stale data (1100)
      const second = result.current.run({ amount: 100 });
      expect(result.current.value.total_stake).toBe(1200);

      await Promise.all([first, second]);

      await waitFor(() => {
        expect(result.current.status).toBe("success");
      });

      // Should not have reverted to stale 1100
      expect(result.current.value.total_stake).toBeGreaterThanOrEqual(1150);
    });

    it("reconciles authoritative state after network error", async () => {
      let shouldFail = true;
      const commit = jest.fn().mockImplementation(() => {
        if (shouldFail) {
          return Promise.reject(new Error("Network error"));
        }
        return Promise.resolve(undefined);
      });

      const { result } = renderHook(() =>
        useOptimisticAction({
          data: mockPool,
          applyOptimistic: (current, input: { amount: number }) => ({
            ...current,
            total_stake: current.total_stake + input.amount,
          }),
          commit,
        })
      );

      // Fails
      result.current.run({ amount: 100 });
      expect(result.current.value.total_stake).toBe(1100);

      await waitFor(() => {
        expect(result.current.status).toBe("error");
      });

      // Rolled back
      expect(result.current.value.total_stake).toBe(1000);

      // Reset and retry with network working
      shouldFail = false;
      result.current.reset();
      result.current.run({ amount: 100 });

      await waitFor(() => {
        expect(result.current.status).toBe("success");
      });

      expect(result.current.value.total_stake).toBe(1100);
    });
  });

  describe("terminal failure handling", () => {
    it("does not retry after 404 or 403", async () => {
      let attempts = 0;
      const commit = jest.fn().mockImplementation(() => {
        attempts++;
        return Promise.reject(new Error("404 Not Found"));
      });

      const { result } = renderHook(() =>
        useOptimisticAction({
          data: mockPool,
          applyOptimisticAction: (current, input: { amount: number }) => ({
            ...current,
            total_stake: current.total_stake + input.amount,
          }),
          commit,
        })
      );

      // Note: The hook doesn't implement automatic retry logic; that's caller responsibility
      // This test verifies the hook doesn't retry on its own
      result.current.run({ amount: 100 });

      await waitFor(() => {
        expect(result.current.error).toBeDefined();
      });

      expect(attempts).toBe(1);
    });

    it("shows user-friendly error on terminal failure", async () => {
      const commit = jest
        .fn()
        .mockRejectedValue(new Error("Permission denied"));

      const { result } = renderHook(() =>
        useOptimisticAction({
          data: mockPool,
          applyOptimistic: (current, input: { amount: number }) => ({
            ...current,
            total_stake: current.total_stake + input.amount,
          }),
          commit,
        })
      );

      result.current.run({ amount: 100 });

      await waitFor(() => {
        expect(result.current.error?.message).toBe("Permission denied");
      });
    });
  });

  describe("prediction count and odds updates", () => {
    it("updates prediction_count on each optimistic prediction", async () => {
      const commit = jest.fn().mockResolvedValue(undefined);

      const { result } = renderHook(() =>
        useOptimisticAction({
          data: mockPool,
          applyOptimistic: (current, input: { amount: number; outcome: number }) => ({
            ...current,
            total_stake: current.total_stake + input.amount,
            prediction_count: (current.prediction_count ?? 10) + 1,
          }),
          commit,
        })
      );

      expect(result.current.value.prediction_count).toBe(10);

      result.current.run({ amount: 100, outcome: 0 });
      expect(result.current.value.prediction_count).toBe(11);

      result.current.run({ amount: 50, outcome: 1 });
      expect(result.current.value.prediction_count).toBe(12);

      await waitFor(() => {
        expect(result.current.status).toBe("success");
      });

      expect(result.current.value.prediction_count).toBe(12);
    });
  });
});
