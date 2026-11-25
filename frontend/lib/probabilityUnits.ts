/**
 * Consistent formatting and unit explanations for displayed market probabilities,
 * share prices, payout odds, and stake units (#248).
 *
 * All market surfaces (pool details, calculators, comparison views, and charts)
 * should derive and explain displayed values from this module so users always
 * see:
 *  - the exact unit being shown (%, <TOKEN>/share, × multiplier, or <TOKEN>),
 *  - how the figure is derived from pool stakes,
 *  - an explicit unavailable or invalid state whenever inputs are missing or
 *    out of range (never silent `NaN%`, `Infinity×`, or ambiguous unitless
 *    numbers), and
 *  - that displayed probabilities/prices are illustrative snapshots while
 *    on-chain contract state at resolution is authoritative.
 */

import { STELLAR_DECIMALS } from "@/lib/amount";

export type PricingUnitKind = "probability" | "share_price" | "odds_multiplier" | "stake_amount";

export type MetricStatus = "ok" | "unavailable" | "invalid";

export interface FormattedPricingMetric {
  kind: PricingUnitKind;
  status: MetricStatus;
  /** Underlying numeric value when `status === "ok"`, otherwise `null`. */
  numericValue: number | null;
  /** Formatted string ready for display (e.g. `"62.5%"`, `"0.63 XLM"`, `"×1.60"`, `"—"`). */
  display: string;
  /** Explicit unit description (e.g. `"%"`, `"XLM per 1 XLM winning share"`). */
  unitLabel: string;
  /** Screen-reader-friendly description combining value and unit. */
  ariaLabel: string;
  /** Present when `status !== "ok"` to explain why no numeric value is shown. */
  reason?: string;
  /** Actionable user recovery hint when `status !== "ok"`. */
  recoveryHint?: string;
}

export interface PricingUnitExplanation {
  kind: PricingUnitKind;
  label: string;
  unitSymbol: string;
  formula: string;
  summary: string;
  authorityNote: string;
  example: string;
}

export interface OutcomePricingSummary {
  token: string;
  probability: FormattedPricingMetric;
  sharePrice: FormattedPricingMetric;
  odds: FormattedPricingMetric;
  stake: FormattedPricingMetric;
}

export interface FormatMetricOptions {
  decimals?: number;
}

export interface DeriveOutcomePricingOptions {
  probabilityDecimals?: number;
  priceDecimals?: number;
  oddsDecimals?: number;
  stakeDecimals?: number;
}

export const PRICING_UNIT_KINDS: readonly PricingUnitKind[] = [
  "probability",
  "share_price",
  "odds_multiplier",
  "stake_amount",
] as const;

export const DEFAULT_TOKEN_SYMBOL = "XLM";

export function normalizeTokenSymbol(token?: string | null): string {
  const cleaned = token?.trim();
  return cleaned && cleaned.length > 0 ? cleaned : DEFAULT_TOKEN_SYMBOL;
}

function clampDecimals(decimals: number | undefined, fallback: number): number {
  if (decimals === undefined || !Number.isInteger(decimals) || decimals < 0 || decimals > 7) {
    return fallback;
  }
  return decimals;
}

function formatFixed(value: number, decimals: number): string {
  return value.toLocaleString("en-US", {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  });
}

/**
 * Format an implied probability in `[0, 1]` as a percentage.
 * Explicitly reports `unavailable` for null/undefined and `invalid` for
 * out-of-range or non-finite inputs.
 */
export function formatImpliedProbability(
  probability: number | null | undefined,
  options: FormatMetricOptions = {},
): FormattedPricingMetric {
  const decimals = clampDecimals(options.decimals, 1);

  if (probability === null || probability === undefined) {
    return {
      kind: "probability",
      status: "unavailable",
      numericValue: null,
      display: "—",
      unitLabel: "percent (%)",
      ariaLabel: "Implied probability unavailable",
      reason: "No pool stakes have been recorded yet.",
      recoveryHint: "Place the first prediction or refresh once stakes enter the pool.",
    };
  }

  if (!Number.isFinite(probability) || probability < 0 || probability > 1) {
    return {
      kind: "probability",
      status: "invalid",
      numericValue: null,
      display: "Unavailable",
      unitLabel: "percent (%)",
      ariaLabel: "Implied probability unavailable due to invalid market data",
      reason: "Reported implied probability is outside the valid 0% to 100% range.",
      recoveryHint: "Refresh market data or inspect on-chain pool stakes directly.",
    };
  }

  const percent = probability * 100;
  const display = `${formatFixed(percent, decimals)}%`;
  return {
    kind: "probability",
    status: "ok",
    numericValue: probability,
    display,
    unitLabel: "percent (%)",
    ariaLabel: `Implied probability ${display}`,
  };
}

/**
 * Format an implied unit price in `[0, 1]` denominated in `token` per 1 `token`
 * of winning payout share.
 */
export function formatPriceUnit(
  price: number | null | undefined,
  token?: string | null,
  options: FormatMetricOptions = {},
): FormattedPricingMetric {
  const symbol = normalizeTokenSymbol(token);
  const decimals = clampDecimals(options.decimals, 2);
  const unitLabel = `${symbol} per 1 ${symbol} winning share`;

  if (price === null || price === undefined) {
    return {
      kind: "share_price",
      status: "unavailable",
      numericValue: null,
      display: "—",
      unitLabel,
      ariaLabel: `Implied unit price in ${symbol} unavailable`,
      reason: "Unit price cannot be derived until the pool has active stakes.",
      recoveryHint: "Refresh after the first prediction is confirmed on-chain.",
    };
  }

  if (!Number.isFinite(price) || price < 0 || price > 1) {
    return {
      kind: "share_price",
      status: "invalid",
      numericValue: null,
      display: "Unavailable",
      unitLabel,
      ariaLabel: `Implied unit price in ${symbol} unavailable due to invalid market data`,
      reason: `Reported unit price is outside the valid 0.00 to 1.00 ${symbol} per share range.`,
      recoveryHint: "Refresh market data or verify stake totals on-chain.",
    };
  }

  const display = `${formatFixed(price, decimals)} ${symbol}`;
  return {
    kind: "share_price",
    status: "ok",
    numericValue: price,
    display,
    unitLabel,
    ariaLabel: `Implied unit price ${display} per 1 ${symbol} winning share`,
  };
}

/**
 * Format a payout multiplier (decimal odds `>= 1.00`).
 */
export function formatOddsMultiplier(
  odds: number | null | undefined,
  options: FormatMetricOptions = {},
): FormattedPricingMetric {
  const decimals = clampDecimals(options.decimals, 2);

  if (odds === null || odds === undefined) {
    return {
      kind: "odds_multiplier",
      status: "unavailable",
      numericValue: null,
      display: "—",
      unitLabel: "payout multiplier (×)",
      ariaLabel: "Payout odds multiplier unavailable",
      reason: "No stake has been placed on this outcome yet.",
      recoveryHint: "Odds become defined once at least one stake backs this outcome.",
    };
  }

  if (!Number.isFinite(odds) || odds < 1) {
    return {
      kind: "odds_multiplier",
      status: "invalid",
      numericValue: null,
      display: "Unavailable",
      unitLabel: "payout multiplier (×)",
      ariaLabel: "Payout odds multiplier unavailable due to invalid market data",
      reason: "Reported payout multiplier is below 1.00× or non-finite.",
      recoveryHint: "Refresh market data or verify pool balances on-chain.",
    };
  }

  const formatted = formatFixed(odds, decimals);
  const display = `×${formatted}`;
  return {
    kind: "odds_multiplier",
    status: "ok",
    numericValue: odds,
    display,
    unitLabel: "payout multiplier (×)",
    ariaLabel: `Payout odds ${formatted} times stake`,
  };
}

/**
 * Format a pool or outcome stake amount with an explicit token symbol.
 */
export function formatStakeUnit(
  stake: number | null | undefined,
  token?: string | null,
  options: FormatMetricOptions = {},
): FormattedPricingMetric {
  const symbol = normalizeTokenSymbol(token);
  const decimals = clampDecimals(options.decimals, 2);

  if (stake === null || stake === undefined) {
    return {
      kind: "stake_amount",
      status: "unavailable",
      numericValue: null,
      display: "—",
      unitLabel: symbol,
      ariaLabel: `Stake amount in ${symbol} unavailable`,
      reason: "Stake balance has not been loaded yet.",
      recoveryHint: "Refresh market balances to fetch the latest mirrored on-chain stake.",
    };
  }

  if (!Number.isFinite(stake) || stake < 0) {
    return {
      kind: "stake_amount",
      status: "invalid",
      numericValue: null,
      display: "Unavailable",
      unitLabel: symbol,
      ariaLabel: `Stake amount in ${symbol} unavailable due to invalid data`,
      reason: "Reported stake amount is negative or non-finite.",
      recoveryHint: "Refresh market data or verify pool balances on-chain.",
    };
  }

  const display = `${formatFixed(stake, decimals)} ${symbol}`;
  return {
    kind: "stake_amount",
    status: "ok",
    numericValue: stake,
    display,
    unitLabel: symbol,
    ariaLabel: `Staked ${display}`,
  };
}

/**
 * Derive consistent implied probability, unit share price, payout odds, and
 * stake metrics from an outcome's stake and the total pool stake.
 */
export function deriveOutcomePricing(
  outcomeStake: number,
  totalStake: number,
  token?: string | null,
  options: DeriveOutcomePricingOptions = {},
): OutcomePricingSummary {
  const symbol = normalizeTokenSymbol(token);
  const probDecimals = clampDecimals(options.probabilityDecimals, 1);
  const priceDecimals = clampDecimals(options.priceDecimals, 2);
  const oddsDecimals = clampDecimals(options.oddsDecimals, 2);
  const stakeDecimals = clampDecimals(options.stakeDecimals, 2);

  const inputsInvalid =
    !Number.isFinite(outcomeStake) ||
    !Number.isFinite(totalStake) ||
    outcomeStake < 0 ||
    totalStake < 0 ||
    outcomeStake > totalStake;

  if (inputsInvalid) {
    const reason = "Outcome stake cannot exceed total pool stake or be negative.";
    const recoveryHint = "Refresh market data or check the pool contract balances directly.";
    return {
      token: symbol,
      probability: {
        kind: "probability",
        status: "invalid",
        numericValue: null,
        display: "Unavailable",
        unitLabel: "percent (%)",
        ariaLabel: "Implied probability unavailable due to inconsistent stake totals",
        reason,
        recoveryHint,
      },
      sharePrice: {
        kind: "share_price",
        status: "invalid",
        numericValue: null,
        display: "Unavailable",
        unitLabel: `${symbol} per 1 ${symbol} winning share`,
        ariaLabel: `Implied unit price in ${symbol} unavailable due to inconsistent stake totals`,
        reason,
        recoveryHint,
      },
      odds: {
        kind: "odds_multiplier",
        status: "invalid",
        numericValue: null,
        display: "Unavailable",
        unitLabel: "payout multiplier (×)",
        ariaLabel: "Payout odds unavailable due to inconsistent stake totals",
        reason,
        recoveryHint,
      },
      stake: formatStakeUnit(
        Number.isFinite(outcomeStake) && outcomeStake >= 0 ? outcomeStake : Number.NaN,
        symbol,
        { decimals: stakeDecimals },
      ),
    };
  }

  if (totalStake === 0) {
    return {
      token: symbol,
      probability: formatImpliedProbability(null, { decimals: probDecimals }),
      sharePrice: formatPriceUnit(null, symbol, { decimals: priceDecimals }),
      odds: formatOddsMultiplier(null, { decimals: oddsDecimals }),
      stake: formatStakeUnit(0, symbol, { decimals: stakeDecimals }),
    };
  }

  const probability = outcomeStake / totalStake;
  const odds = outcomeStake > 0 ? totalStake / outcomeStake : null;

  return {
    token: symbol,
    probability: formatImpliedProbability(probability, { decimals: probDecimals }),
    sharePrice: formatPriceUnit(probability, symbol, { decimals: priceDecimals }),
    odds: formatOddsMultiplier(odds, { decimals: oddsDecimals }),
    stake: formatStakeUnit(outcomeStake, symbol, { decimals: stakeDecimals }),
  };
}

/**
 * Retrieve consistent, reviewed user-facing explanation copy for any displayed
 * probability, price, odds, or stake unit.
 */
export function getPricingUnitExplanation(
  kind: PricingUnitKind,
  token?: string | null,
): PricingUnitExplanation {
  const symbol = normalizeTokenSymbol(token);
  const baseUnitNote =
    symbol === "XLM"
      ? `On-chain settlement uses integer stroops (1 XLM = 10^${STELLAR_DECIMALS} stroops).`
      : `On-chain settlement uses integer token base units for ${symbol}.`;

  switch (kind) {
    case "probability":
      return {
        kind,
        label: "Implied probability",
        unitSymbol: "%",
        formula: "Outcome stake ÷ Total pool stake × 100%",
        summary:
          "Share of total pool stakes currently backing this outcome. Probabilities across all outcomes in a pool sum to 100% and reflect pool participation, not a guarantee of the event outcome.",
        authorityNote:
          "Illustrative snapshot mirrored from pool stakes; final settlement is governed by on-chain stakes at market close.",
        example: `600 ${symbol} on an outcome out of 1,000 ${symbol} total = 60.0% implied probability.`,
      };
    case "share_price":
      return {
        kind,
        label: "Implied unit price",
        unitSymbol: `${symbol} / share`,
        formula: `Implied probability × 1.00 ${symbol}`,
        summary: `Implied cost in ${symbol} per 1.00 ${symbol} of gross winning payout share before protocol fees. Values always range from 0.00 to 1.00 ${symbol}.`,
        authorityNote: `Quoted in ${symbol} (not USD unless ${symbol} is a USD-pegged asset). ${baseUnitNote}`,
        example: `A 60.0% implied probability equals an implied unit price of 0.60 ${symbol} per 1.00 ${symbol} winning share.`,
      };
    case "odds_multiplier":
      return {
        kind,
        label: "Payout odds",
        unitSymbol: "×",
        formula: "Total pool stake ÷ Outcome stake",
        summary:
          "Estimated gross payout multiplier on a winning stake before protocol fees. Odds move as new predictions enter the pool and are not locked until market close.",
        authorityNote:
          "Frontend odds are derived estimates; on-chain contract math at resolution is authoritative.",
        example: `1,000 ${symbol} total ÷ 600 ${symbol} on this outcome = ×1.67 gross payout multiplier.`,
      };
    case "stake_amount":
      return {
        kind,
        label: "Pool stake",
        unitSymbol: symbol,
        formula: `Sum of confirmed stakes denominated in ${symbol}`,
        summary: `Total ${symbol} collateral currently committed to this outcome or pool.`,
        authorityNote: `${baseUnitNote} Displayed amounts are read from the indexer mirror of the pool contract.`,
        example: `600.00 ${symbol} staked on Yes and 400.00 ${symbol} on No = 1,000.00 ${symbol} total pool stake.`,
      };
  }
}
