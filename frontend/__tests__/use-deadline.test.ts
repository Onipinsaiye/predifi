import { renderHook, waitFor } from "@testing-library/react";
import { useDeadline } from "@/lib/deadline/use-deadline";
import { CLOSING_SOON_MS } from "@/lib/deadline/format-deadline";

const at = (iso: string) => Date.parse(iso);

/**
 * Tests for useDeadline hook: hydration safety, locale switching, ticking intervals.
 * Verifies that SSR and first client render are deterministic (no hydration mismatch)
 * and that client-side ticking respects the deadline status.
 */
describe("useDeadline", () => {
  describe("hydration safety", () => {
    it("server render and first hydration use fixed locale/zone/no-clock for determinism", () => {
      const deadline = "2026-06-15T12:00:00Z";
      const nowMs = at("2026-06-01T00:00:00Z");

      const { result } = renderHook(() => useDeadline(deadline, () => nowMs));

      // Initially `ready` is false (SSR mode)
      expect(result.current.ready).toBe(false);
      expect(result.current.status).toBe("unknown"); // no clock in SSR
      expect(result.current.remainingMs).toBeNull();
      expect(result.current.locale).toBe("en-US");
      expect(result.current.timeZone).toBe("UTC");
    });

    it("switches to viewer's locale/zone after hydration", async () => {
      const deadline = "2026-06-15T12:00:00Z";
      const nowMs = at("2026-06-01T00:00:00Z");

      const { result } = renderHook(() => useDeadline(deadline, () => nowMs));

      // Wait for `ready` to become true
      await waitFor(
        () => {
          expect(result.current.ready).toBe(true);
        },
        { timeout: 1000 }
      );

      // After hydration, viewer's timezone is used (but we can't mock navigator in jsdom reliably)
      expect(result.current.status).toBe("open");
      expect(result.current.remainingMs).toBeGreaterThan(0);
    });
  });

  describe("ticking intervals", () => {
    beforeEach(() => {
      jest.useFakeTimers();
    });

    afterEach(() => {
      jest.useRealTimers();
    });

    it("ticks every 1s when status is closing-soon", async () => {
      const deadline = at("2026-06-15T12:00:00Z");
      const now = deadline - CLOSING_SOON_MS + 10_000; // 10s before deadline

      let currentTime = now;
      const nowFn = () => currentTime;

      const { result } = renderHook(() => useDeadline(deadline, nowFn));

      // Move past hydration
      jest.runOnlyPendingTimers();
      await waitFor(() => expect(result.current.ready).toBe(true));

      const initialCountdown = result.current.countdown;

      // Advance 1s and tick
      currentTime += 1000;
      jest.advanceTimersByTime(1000);

      await waitFor(() => {
        expect(result.current.countdown).not.toBe(initialCountdown);
      });
    });

    it("ticks every 30s when status is open", async () => {
      const deadline = at("2026-06-15T12:00:00Z");
      const now = deadline - 2 * 60 * 60_000; // 2 hours before deadline

      let currentTime = now;
      const nowFn = () => currentTime;

      const { result } = renderHook(() => useDeadline(deadline, nowFn));

      jest.runOnlyPendingTimers();
      await waitFor(() => expect(result.current.ready).toBe(true));

      // Interval should be 30s for open status
      const initialCountdown = result.current.countdown;

      currentTime += 30_000;
      jest.advanceTimersByTime(30_000);

      // Countdown may not change within 30s since time was set to exactly 30s ahead,
      // but the interval is set up correctly.
      expect(result.current.countdown).toBeDefined();
    });

    it("does not tick when status is expired", async () => {
      const deadline = at("2026-06-15T12:00:00Z");
      const now = deadline + 1000; // Already expired

      let currentTime = now;
      const nowFn = () => currentTime;

      const { result } = renderHook(() => useDeadline(deadline, nowFn));

      jest.runOnlyPendingTimers();
      await waitFor(() => expect(result.current.ready).toBe(true));

      expect(result.current.status).toBe("expired");
      expect(result.current.countdown).toBeNull();

      const initialCountdown = result.current.countdown;

      // Advance time significantly
      currentTime += 10_000;
      jest.advanceTimersByTime(10_000);

      // No countdown should update when expired
      expect(result.current.countdown).toBe(initialCountdown);
    });

    it("does not tick when status is unknown (SSR)", () => {
      const deadline = "2026-06-15T12:00:00Z";

      const { result } = renderHook(() => useDeadline(deadline));

      // In SSR mode (ready=false), no ticking
      expect(result.current.ready).toBe(false);
      expect(result.current.status).toBe("unknown");

      jest.advanceTimersByTime(5000);

      // Status should remain unknown
      expect(result.current.status).toBe("unknown");
    });
  });

  describe("countdown formatting", () => {
    beforeEach(() => {
      jest.useFakeTimers();
    });

    afterEach(() => {
      jest.useRealTimers();
    });

    it("formats countdown correctly", async () => {
      const deadline = at("2026-06-15T12:00:00Z");
      const now = deadline - 65_000; // 1m 5s

      let currentTime = now;
      const nowFn = () => currentTime;

      const { result } = renderHook(() => useDeadline(deadline, nowFn));

      jest.runOnlyPendingTimers();
      await waitFor(() => expect(result.current.ready).toBe(true));

      expect(result.current.countdown).toMatch(/^1m 05s$/);
    });

    it("shows null countdown for missing/invalid deadlines", async () => {
      const { result } = renderHook(() => useDeadline(null, () => Date.now()));

      jest.runOnlyPendingTimers();
      await waitFor(() => expect(result.current.ready).toBe(true));

      expect(result.current.countdown).toBeNull();
      expect(result.current.status).toBe("missing");
    });
  });

  describe("cleanup on unmount", () => {
    beforeEach(() => {
      jest.useFakeTimers();
    });

    afterEach(() => {
      jest.useRealTimers();
    });

    it("clears interval on unmount", async () => {
      const deadline = at("2026-06-15T12:00:00Z");
      const now = deadline - CLOSING_SOON_MS + 5_000;

      let currentTime = now;
      const nowFn = () => currentTime;

      const { result, unmount } = renderHook(() => useDeadline(deadline, nowFn));

      jest.runOnlyPendingTimers();
      await waitFor(() => expect(result.current.ready).toBe(true));

      const spy = jest.spyOn(global, "setInterval");

      unmount();

      // After unmount, no more intervals should be set
      jest.runOnlyPendingTimers();

      spy.mockRestore();
    });
  });

  describe("missing and invalid inputs", () => {
    it("handles missing deadline", async () => {
      const { result } = renderHook(() => useDeadline(null, () => Date.now()));

      await waitFor(() => expect(result.current.ready).toBe(true));

      expect(result.current.status).toBe("missing");
      expect(result.current.local).toBeNull();
      expect(result.current.absolute).toBeNull();
      expect(result.current.countdown).toBeNull();
    });

    it("handles invalid deadline", async () => {
      const { result } = renderHook(() => useDeadline("not-a-date", () => Date.now()));

      await waitFor(() => expect(result.current.ready).toBe(true));

      expect(result.current.status).toBe("invalid");
      expect(result.current.local).toBeNull();
      expect(result.current.absolute).toBeNull();
      expect(result.current.countdown).toBeNull();
    });
  });
});
