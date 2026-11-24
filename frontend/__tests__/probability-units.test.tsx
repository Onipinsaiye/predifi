/**
 * Consistent probability and price-unit formatting & explanation tests (#248).
 */

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import {
  PricingMetricValue,
  PricingUnitsLegend,
} from "@/components/ui/pricing-unit-explainer";
import { getGlossaryEntry } from "@/lib/glossary";
import {
  PRICING_UNIT_KINDS,
  deriveOutcomePricing,
  formatImpliedProbability,
  formatOddsMultiplier,
  formatPriceUnit,
  formatStakeUnit,
  getPricingUnitExplanation,
  normalizeTokenSymbol,
} from "@/lib/probabilityUnits";

describe("probabilityUnits library", () => {
  it("derives consistent probability, share price, odds, and stake for active pools", () => {
    const summary = deriveOutcomePricing(600, 1000, "USDC");

    expect(summary.token).toBe("USDC");
    expect(summary.probability.status).toBe("ok");
    expect(summary.probability.display).toBe("60.0%");
    expect(summary.sharePrice.status).toBe("ok");
    expect(summary.sharePrice.display).toBe("0.60 USDC");
    expect(summary.sharePrice.unitLabel).toBe("USDC per 1 USDC winning share");
    expect(summary.odds.status).toBe("ok");
    expect(summary.odds.display).toBe("×1.67");
    expect(summary.stake.status).toBe("ok");
    expect(summary.stake.display).toBe("600.00 USDC");
  });

  it("handles boundary states for empty pools, zero-stake outcomes, and 100% single-sided pools", () => {
    const emptyPool = deriveOutcomePricing(0, 0, "XLM");
    expect(emptyPool.probability.status).toBe("unavailable");
    expect(emptyPool.probability.display).toBe("—");
    expect(emptyPool.sharePrice.status).toBe("unavailable");
    expect(emptyPool.odds.status).toBe("unavailable");
    expect(emptyPool.stake.status).toBe("ok");
    expect(emptyPool.stake.display).toBe("0.00 XLM");

    const zeroSide = deriveOutcomePricing(0, 500, "XLM");
    expect(zeroSide.probability.status).toBe("ok");
    expect(zeroSide.probability.display).toBe("0.0%");
    expect(zeroSide.sharePrice.status).toBe("ok");
    expect(zeroSide.sharePrice.display).toBe("0.00 XLM");
    expect(zeroSide.odds.status).toBe("unavailable");
    expect(zeroSide.odds.reason).toMatch(/no stake/i);

    const fullSide = deriveOutcomePricing(500, 500, "XLM");
    expect(fullSide.probability.display).toBe("100.0%");
    expect(fullSide.sharePrice.display).toBe("1.00 XLM");
    expect(fullSide.odds.display).toBe("×1.00");
  });

  it("surfaces explicit invalid states instead of NaN or Infinity on malformed inputs", () => {
    const inconsistent = deriveOutcomePricing(1200, 1000, "XLM");
    expect(inconsistent.probability.status).toBe("invalid");
    expect(inconsistent.probability.display).toBe("Unavailable");
    expect(inconsistent.probability.reason).toBeTruthy();
    expect(inconsistent.probability.recoveryHint).toBeTruthy();
    expect(inconsistent.sharePrice.status).toBe("invalid");
    expect(inconsistent.odds.status).toBe("invalid");

    expect(formatImpliedProbability(Number.NaN).status).toBe("invalid");
    expect(formatImpliedProbability(1.25).status).toBe("invalid");
    expect(formatImpliedProbability(-0.1).status).toBe("invalid");
    expect(formatPriceUnit(Number.POSITIVE_INFINITY, "XLM").status).toBe("invalid");
    expect(formatOddsMultiplier(0.5).status).toBe("invalid");
    expect(formatStakeUnit(-10, "XLM").status).toBe("invalid");

    for (const metric of [
      inconsistent.probability,
      inconsistent.sharePrice,
      inconsistent.odds,
      formatImpliedProbability(Number.NaN),
      formatOddsMultiplier(Number.POSITIVE_INFINITY),
    ]) {
      expect(metric.display).not.toContain("NaN");
      expect(metric.display).not.toContain("Infinity");
    }
  });

  it("normalizes missing or blank token symbols to XLM and supplies stroop precision copy", () => {
    expect(normalizeTokenSymbol(undefined)).toBe("XLM");
    expect(normalizeTokenSymbol("   ")).toBe("XLM");
    expect(normalizeTokenSymbol(" USDC ")).toBe("USDC");

    for (const kind of PRICING_UNIT_KINDS) {
      const xlmInfo = getPricingUnitExplanation(kind, "XLM");
      expect(xlmInfo.formula.length).toBeGreaterThan(0);
      expect(xlmInfo.summary.length).toBeGreaterThan(0);
      expect(xlmInfo.authorityNote.length).toBeGreaterThan(0);
    }

    expect(getPricingUnitExplanation("share_price", "XLM").authorityNote).toMatch(/stroops/i);
    expect(getPricingUnitExplanation("share_price", "USDC").unitSymbol).toBe("USDC / share");
  });

  it("registers price-unit in the central glossary", () => {
    const entry = getGlossaryEntry("price-unit");
    expect(entry).not.toBeNull();
    expect(entry?.term).toBe("Price Unit");
    expect(entry?.related).toContain("probability");
  });
});

describe("<PricingMetricValue />", () => {
  it("toggles unit explanation panel via click and exposes formula and authority note", async () => {
    const user = userEvent.setup();
    const pricing = deriveOutcomePricing(750, 1000, "USDC");

    const { container } = render(
      <PricingMetricValue metric={pricing.sharePrice} token="USDC" />,
    );

    expect(screen.getByText("0.75 USDC")).toBeInTheDocument();
    const toggle = screen.getByRole("button", {
      name: /explain implied unit price/i,
    });
    expect(toggle).toHaveAttribute("aria-expanded", "false");

    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText(/USDC per 1 USDC winning share/i)).toBeInTheDocument();
    expect(screen.getByText(/Formula:/i)).toBeInTheDocument();

    expect(await axe(container)).toHaveNoViolations();
  });

  it("renders explicit status and alert recovery states for unavailable and invalid metrics", async () => {
    const unavailable = formatOddsMultiplier(null);
    const { rerender, container } = render(
      <PricingMetricValue metric={unavailable} token="XLM" defaultExpanded />,
    );

    const statusBox = screen.getByRole("status");
    expect(statusBox).toHaveTextContent(/no stake has been placed/i);
    expect(await axe(container)).toHaveNoViolations();

    const invalid = formatImpliedProbability(1.5);
    rerender(<PricingMetricValue metric={invalid} token="XLM" defaultExpanded />);

    const alertBox = screen.getByRole("alert");
    expect(alertBox).toHaveTextContent(/outside the valid 0% to 100% range/i);
    expect(alertBox).toHaveTextContent(/refresh market data/i);
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("<PricingUnitsLegend />", () => {
  it("expands on demand to explain all four units for the market token without axe violations", async () => {
    const user = userEvent.setup();
    const { container } = render(<PricingUnitsLegend token="USDC" />);

    const toggle = screen.getByRole("button", { name: /how units work/i });
    expect(toggle).toHaveAttribute("aria-expanded", "false");

    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Implied probability")).toBeInTheDocument();
    expect(screen.getByText("Implied unit price")).toBeInTheDocument();
    expect(screen.getByText("Payout odds")).toBeInTheDocument();
    expect(screen.getByText("Pool stake")).toBeInTheDocument();
    expect(screen.getByText("USDC / share")).toBeInTheDocument();
    expect(
      screen.getByText(/final payouts are calculated authoritatively on-chain/i),
    ).toBeInTheDocument();

    expect(await axe(container)).toHaveNoViolations();
  });
});
