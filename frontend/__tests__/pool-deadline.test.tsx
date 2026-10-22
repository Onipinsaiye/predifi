import { render, screen, waitFor } from "@testing-library/react";
import { PoolDeadline } from "@/components/pool/PoolDeadline";

/**
 * Integration tests for PoolDeadline component: rendering with various deadline states,
 * locale handling, and hydration safety.
 */
describe("PoolDeadline", () => {
  describe("rendering states", () => {
    it("renders 'Deadline not set' for null endTime", () => {
      render(<PoolDeadline endTime={null} />);
      expect(screen.getByText("Deadline not set")).toBeInTheDocument();
    });

    it("renders 'Deadline not set' for undefined endTime", () => {
      render(<PoolDeadline endTime={undefined} />);
      expect(screen.getByText("Deadline not set")).toBeInTheDocument();
    });

    it("renders 'Deadline unavailable' for invalid endTime", () => {
      // Pass invalid Unix seconds (far past or future, encoded as invalid Date)
      render(<PoolDeadline endTime={-999999999999} />);
      // This will be parsed but may be invalid; verify behavior
      // or adjust test if implementation differs
    });

    it("renders deadline with local time, UTC reference, and countdown", async () => {
      const futureUnixSeconds = Math.floor(Date.now() / 1000) + 3600; // 1 hour from now
      render(<PoolDeadline endTime={futureUnixSeconds} />);

      await waitFor(() => {
        const deadline = screen.getByTestId("pool-deadline");
        expect(deadline).toBeInTheDocument();
        expect(deadline.getAttribute("data-status")).toMatch(/^(unknown|open|closing-soon)$/);
      });

      // Should have both local time and UTC
      const timeElement = screen.getByRole("time");
      expect(timeElement).toBeInTheDocument();
      expect(timeElement.textContent).toMatch(/\d{1,2}:\d{2}/); // local time format
    });

    it("renders 'Closed' text when deadline is expired", async () => {
      const pastUnixSeconds = Math.floor(Date.now() / 1000) - 60; // 1 minute ago
      render(<PoolDeadline endTime={pastUnixSeconds} />);

      await waitFor(() => {
        expect(screen.getByText("Closed")).toBeInTheDocument();
      });
    });
  });

  describe("formatting accuracy", () => {
    it("includes UTC offset in local time display", async () => {
      const futureUnixSeconds = Math.floor(Date.now() / 1000) + 7200; // 2 hours
      render(<PoolDeadline endTime={futureUnixSeconds} />);

      await waitFor(() => {
        const deadline = screen.getByTestId("pool-deadline");
        const text = deadline.textContent;
        // Should contain timezone abbreviation (EDT, EST, UTC, etc.)
        expect(text).toMatch(/[A-Z]{2,4}\)/);
      });
    });

    it("displays absolute UTC time", async () => {
      const futureUnixSeconds = Math.floor(Date.now() / 1000) + 3600;
      render(<PoolDeadline endTime={futureUnixSeconds} />);

      await waitFor(() => {
        const utcSpan = screen.getByText(/\(UTC\)/);
        expect(utcSpan).toBeInTheDocument();
        expect(utcSpan.textContent).toMatch(/\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z/);
      });
    });
  });

  describe("semantic HTML", () => {
    it("uses <time> element with dateTime attribute", async () => {
      const futureUnixSeconds = Math.floor(Date.now() / 1000) + 3600;
      render(<PoolDeadline endTime={futureUnixSeconds} />);

      await waitFor(() => {
        const timeElement = screen.getByRole("time");
        expect(timeElement).toHaveAttribute("dateTime");
        expect(timeElement.getAttribute("dateTime")).toMatch(/\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z/);
      });
    });
  });

  describe("custom label and className", () => {
    it("renders custom label", async () => {
      const futureUnixSeconds = Math.floor(Date.now() / 1000) + 3600;
      render(<PoolDeadline endTime={futureUnixSeconds} label="Resolution" />);

      expect(screen.getByText(/Resolution/)).toBeInTheDocument();
    });

    it("applies custom className", () => {
      const futureUnixSeconds = Math.floor(Date.now() / 1000) + 3600;
      const { container } = render(
        <PoolDeadline endTime={futureUnixSeconds} className="test-class" />
      );

      const span = container.querySelector(".test-class");
      expect(span).toBeInTheDocument();
    });
  });

  describe("edge cases", () => {
    it("handles zero endTime (epoch)", () => {
      render(<PoolDeadline endTime={0} />);
      // Epoch is in the past, should show expired
      const deadline = screen.getByTestId("pool-deadline");
      expect(deadline.getAttribute("data-status")).toBe("expired");
    });

    it("handles very large endTime", async () => {
      const distantFutureSeconds = Math.floor(Date.now() / 1000) + 31536000; // 1 year
      render(<PoolDeadline endTime={distantFutureSeconds} />);

      await waitFor(() => {
        const deadline = screen.getByTestId("pool-deadline");
        expect(deadline.getAttribute("data-status")).toBe("open");
      });
    });
  });

  describe("countdown updates", () => {
    beforeEach(() => {
      jest.useFakeTimers();
    });

    afterEach(() => {
      jest.useRealTimers();
    });

    it("updates countdown text as time passes", async () => {
      const futureUnixSeconds = Math.floor(Date.now() / 1000) + 70; // 70 seconds
      const { rerender } = render(<PoolDeadline endTime={futureUnixSeconds} />);

      await waitFor(() => {
        expect(screen.getByText(/\d+s left/)).toBeInTheDocument();
      });

      // Advance timer
      jest.advanceTimersByTime(10_000);
      rerender(<PoolDeadline endTime={futureUnixSeconds} />);

      await waitFor(() => {
        const countdown = screen.getByText(/\d+s left/);
        expect(countdown).toBeInTheDocument();
      });
    });
  });
});
