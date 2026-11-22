import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe, toHaveNoViolations } from "jest-axe";
import {
  buildQuoteBound,
  computeMinOutput,
  simulateWithBound,
  buildBoundTransaction,
  toBoundCallArguments,
  type BoundCheckResult,
  type LiquidityQuote,
} from "@/lib/liquidity/quoteBound";
import { LiquidityQuoteGuard } from "@/components/pool/LiquidityQuoteGuard";

expect.extend(toHaveNoViolations);

const NOW = Date.parse("2026-01-01T00:00:00Z");

const quote: LiquidityQuote = {
  poolId: "pool-1",
  amountIn: 1_000_000n,
  expectedOut: 990_000n,
  priceImpactBps: 25,
  quotedAtMs: NOW - 1_000,
  networkId: "stellar:testnet",
  stateCheckpoint: "checkpoint-7",
  quoteId: "quote-abc",
};

const context = {
  poolId: "pool-1",
  amountIn: 1_000_000n,
  networkId: "stellar:testnet",
  stateCheckpoint: "checkpoint-7",
};

describe("computeMinOutput", () => {
  it("accepts the exact zero- and full-slippage boundaries", () => {
    expect(computeMinOutput(10_000n, 0)).toBe(10_000n);
    expect(computeMinOutput(10_000n, 10_000)).toBe(0n);
  });

  it("rounds down and keeps an enforceable minimum", () => {
    expect(computeMinOutput(1_000n, 100)).toBe(990n);
    expect(computeMinOutput(101n, 100)).toBe(99n);
  });

  it("rejects out-of-range tolerances", () => {
    expect(() => computeMinOutput(1n, -1)).toThrow(RangeError);
    expect(() => computeMinOutput(1n, 10_001)).toThrow(RangeError);
  });
});

describe("buildQuoteBound", () => {
  it("refuses to create a bound without user input", () => {
    expect(buildQuoteBound(quote, {})).toBeNull();
  });

  it("builds a bound from a maximum impact", () => {
    const bound = buildQuoteBound(quote, { maxSlippageBps: 100 })!;
    expect(bound.minOutput).toBe(980_100n);
    expect(bound.kind).toBe("max-impact");
    expect(bound.maxSlippageBps).toBe(100);
  });

  it("uses the stricter of two supplied bounds", () => {
    const bound = buildQuoteBound(quote, {
      maxSlippageBps: 100,
      minOutput: 985_000n,
    })!;
    expect(bound.minOutput).toBe(985_000n);
  });

  it("rejects unenforceable minimums", () => {
    expect(buildQuoteBound(quote, { minOutput: 0n })).toBeNull();
    expect(buildQuoteBound(quote, { minOutput: quote.expectedOut + 1n })).toBeNull();
  });
});

describe("verifyQuoteBound", () => {
  const bound = buildQuoteBound(quote, { maxSlippageBps: 100 })!;

  it("accepts a fresh, unchanged quote", () => {
    const result = simulateWithBound(bound, context, NOW);
    expect(result.ok).toBe(true);
    if (result.ok) expect(result.arguments.minAmountOut).toBe(bound.minOutput.toString());
  });

  it("refuses to submit without any bound", () => {
    const result = simulateWithBound(null, context, NOW);
    expect(result).toMatchObject({ ok: false, reason: "NO_BOUND" });
  });

  it("rejects an expired quote", () => {
    const result = simulateWithBound(bound, context, NOW + 300_001);
    expect(result).toMatchObject({ ok: false, reason: "QUOTE_EXPIRED" });
  });

  it("accepts a quote exactly at the expiry boundary", () => {
    // The bound was quoted 1s before NOW, so the boundary is NOW + 299_000.
    expect(simulateWithBound(bound, context, NOW + 299_000).ok).toBe(true);
  });

  it("rejects a quote after a network change", () => {
    const result = simulateWithBound(bound, { ...context, networkId: "stellar:pubnet" }, NOW);
    expect(result).toMatchObject({ ok: false, reason: "NETWORK_CHANGED" });
  });

  it("rejects a quote after a material pool-state change", () => {
    const result = simulateWithBound(bound, { ...context, stateCheckpoint: "checkpoint-8" }, NOW);
    expect(result).toMatchObject({ ok: false, reason: "STATE_CHANGED" });
    expect(
      simulateWithBound(bound, { ...context, amountIn: 2_000_000n }, NOW),
    ).toMatchObject({ ok: false, reason: "STATE_CHANGED" });
  });

  it("passes the exact same arguments to simulation and transaction", () => {
    const simulated = simulateWithBound(bound, context, NOW);
    const transaction = buildBoundTransaction(bound, context, NOW);
    expect(simulated.ok && transaction.ok).toBe(true);
    if (simulated.ok && transaction.ok) {
      expect(simulated.arguments).toEqual(transaction.arguments);
      expect(Object.isFrozen(simulated.arguments)).toBe(true);
      expect(toBoundCallArguments(bound)).toEqual(simulated.arguments);
    }
  });

  it("does not loosen the bound when both paths are re-verified later", () => {
    const simulated = simulateWithBound(bound, context, NOW) as Extract<BoundCheckResult, { ok: true }>;
    const later = buildBoundTransaction(bound, context, NOW + 1_000) as Extract<BoundCheckResult, { ok: true }>;
    expect(later.arguments.minAmountOut).toBe(simulated.arguments.minAmountOut);
  });
});

describe("LiquidityQuoteGuard", () => {
  it("refuses to submit without an enforceable bound", async () => {
    const onSubmit = jest.fn();
    const onSimulate = jest.fn();
    render(
      <LiquidityQuoteGuard
        quote={quote}
        context={context}
        now={NOW}
        onSimulate={onSimulate}
        onSubmit={onSubmit}
      />,
    );
    const button = screen.getByRole("button", { name: "Review liquidity action" });
    expect(button).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent(/maximum price impact or minimum output/i);
    await userEvent.click(button);
    expect(onSubmit).not.toHaveBeenCalled();
    expect(onSimulate).not.toHaveBeenCalled();
  });

  it("submits the exact bound used for simulation", async () => {
    const onSubmit = jest.fn();
    const onSimulate = jest.fn();
    render(
      <LiquidityQuoteGuard
        quote={quote}
        context={context}
        now={NOW}
        onSimulate={onSimulate}
        onSubmit={onSubmit}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "1%" }));
    const button = screen.getByRole("button", { name: "Review liquidity action" });
    expect(button).toBeEnabled();
    await userEvent.click(button);
    expect(onSimulate).toHaveBeenCalledTimes(1);
    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit.mock.calls[0][0]).toEqual(onSimulate.mock.calls[0][0]);
    expect(onSubmit.mock.calls[0][0].minAmountOut).toBe("980100");
  });

  it("blocks submission and offers a refresh when the network changes", async () => {
    render(
      <LiquidityQuoteGuard
        quote={quote}
        context={{ ...context, networkId: "stellar:pubnet" }}
        now={NOW}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "1%" }));
    expect(screen.getByRole("button", { name: "Review liquidity action" })).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent(/network changed/i);
    expect(screen.getByRole("button", { name: "Refresh quote" })).toBeInTheDocument();
  });

  it("blocks submission when the quote has expired", async () => {
    render(<LiquidityQuoteGuard quote={quote} context={context} now={NOW + 300_001} />);
    await userEvent.click(screen.getByRole("button", { name: "1%" }));
    expect(screen.getByRole("alert")).toHaveTextContent(/expired/i);
    expect(screen.getByRole("button", { name: "Review liquidity action" })).toBeDisabled();
  });

  it("has no accessibility violations when a bound is enforced", async () => {
    const { container } = render(
      <LiquidityQuoteGuard quote={quote} context={context} now={NOW} />,
    );
    expect(await axe(container)).toHaveNoViolations();
  });
});
