import { render, screen } from "@testing-library/react";
import { axe, toHaveNoViolations } from "jest-axe";
import {
  deriveLiquidityPositionStatus,
  type LiquidityPosition,
} from "@/lib/liquidity/position";
import { LiquidityPositionStatus } from "@/components/pool/LiquidityPositionStatus";

expect.extend(toHaveNoViolations);

const NOW = Date.parse("2026-01-01T00:00:00Z");

function position(overrides: Partial<LiquidityPosition> = {}): LiquidityPosition {
  return {
    poolId: "pool-1",
    depositedValue: 1_000_000_000n,
    shares: 250n,
    totalShares: 1_000n,
    accruedFees: 12_345n,
    marketState: "open",
    lastIndexedAt: new Date(NOW - 5_000).toISOString(),
    ...overrides,
  };
}

describe("deriveLiquidityPositionStatus", () => {
  it("reports an eligible position with an exact share fraction", () => {
    const status = deriveLiquidityPositionStatus(position(), NOW);
    expect(status.shareBps).toBe(2_500);
    expect(status.shareFraction).toBe("25.00%");
    expect(status.eligibility).toBe("eligible");
    expect(status.canWithdraw).toBe(true);
  });

  it("disables actions for zero shares", () => {
    const status = deriveLiquidityPositionStatus(position({ shares: 0n }), NOW);
    expect(status.eligibility).toBe("no-shares");
    expect(status.canWithdraw).toBe(false);
    expect(status.reasons.join(" ")).toMatch(/no LP shares/i);
  });

  it("disables actions when the market lifecycle is locked", () => {
    for (const state of ["resolved", "cancelled", "closed", "paused"]) {
      const status = deriveLiquidityPositionStatus(
        position({ marketState: state }),
        NOW,
      );
      expect(status.eligibility).toBe("locked");
      expect(status.canWithdraw).toBe(false);
    }
  });

  it("honours an explicit lifecycle lock", () => {
    const status = deriveLiquidityPositionStatus(
      position({ lifecycleLocked: true }),
      NOW,
    );
    expect(status.eligibility).toBe("locked");
  });

  it("treats stale checkpoints as unverifiable", () => {
    const status = deriveLiquidityPositionStatus(
      position({ lastIndexedAt: new Date(NOW - 120_000).toISOString() }),
      NOW,
    );
    expect(status.freshness.state).toBe("stale");
    expect(status.eligibility).toBe("stale");
    expect(status.canWithdraw).toBe(false);
  });

  it("treats a missing checkpoint as unavailable", () => {
    const status = deriveLiquidityPositionStatus(
      position({ lastIndexedAt: null }),
      NOW,
    );
    expect(status.freshness.state).toBe("unavailable");
    expect(status.eligibility).toBe("unavailable");
    expect(status.canWithdraw).toBe(false);
  });

  it("never divides by zero when the pool has no shares", () => {
    const status = deriveLiquidityPositionStatus(
      position({ totalShares: 0n }),
      NOW,
    );
    expect(status.shareBps).toBe(0);
    expect(status.shareFraction).toBe("0.00%");
  });

  it("is deterministic in its reason ordering", () => {
    const status = deriveLiquidityPositionStatus(
      position({ shares: 0n, marketState: "resolved", lastIndexedAt: null }),
      NOW,
    );
    expect(status.reasons).toHaveLength(3);
    expect(status.reasons[0]).toMatch(/no LP shares/i);
    expect(status.reasons[1]).toMatch(/locked/i);
    expect(status.reasons[2]).toMatch(/cannot be verified/i);
  });
});

describe("LiquidityPositionStatus", () => {
  it("displays all LP-specific fields", () => {
    render(<LiquidityPositionStatus position={position()} now={NOW} />);
    expect(screen.getByText("Deposited value")).toBeInTheDocument();
    expect(screen.getByText("25.00%")).toBeInTheDocument();
    expect(screen.getByText("Share of pool")).toBeInTheDocument();
    expect(screen.getByText("Accrued fees")).toBeInTheDocument();
    expect(screen.getByText("Last indexed checkpoint")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Withdraw liquidity" })).toBeEnabled();
  });

  it("disables withdrawal and explains why when data is stale", () => {
    render(
      <LiquidityPositionStatus
        position={position({
          lastIndexedAt: new Date(NOW - 120_000).toISOString(),
        })}
        now={NOW}
      />,
    );
    expect(screen.getByRole("button", { name: "Withdraw liquidity" })).toBeDisabled();
    expect(screen.getByText(/data is stale/i)).toBeInTheDocument();
    expect(screen.getByRole("status", { name: "Withdrawal eligibility" })).toHaveTextContent(
      /stale/i,
    );
  });

  it("explains lifecycle locks accessibly", () => {
    render(
      <LiquidityPositionStatus position={position({ marketState: "resolved" })} now={NOW} />,
    );
    expect(screen.getByText(/Withdrawals are locked/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Withdraw liquidity" })).toBeDisabled();
  });

  it("does not call onWithdraw when ineligible", () => {
    const onWithdraw = jest.fn();
    render(
      <LiquidityPositionStatus
        position={position({ shares: 0n })}
        now={NOW}
        onWithdraw={onWithdraw}
      />,
    );
    const button = screen.getByRole("button", { name: "Withdraw liquidity" });
    button.click();
    expect(onWithdraw).not.toHaveBeenCalled();
  });

  it("has no accessibility violations in the locked state", async () => {
    const { container } = render(
      <LiquidityPositionStatus
        position={position({ shares: 0n, marketState: "cancelled" })}
        now={NOW}
      />,
    );
    expect(await axe(container)).toHaveNoViolations();
  });
});
