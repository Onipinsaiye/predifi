/**
 * Quote-bound slippage enforcement for liquidity actions (issue #274).
 *
 * A quote is only valid for a specific pool, network and accounting snapshot.
 * This module turns a user-approved bound (a maximum price impact or a minimum
 * output) into one immutable value that is used for **both** simulation and
 * transaction construction, and refuses to produce a call when the bound has
 * expired, the network changed, or the pool state moved under the quote.
 */

/** Quote returned by the indexer / contract quote endpoint. */
export interface LiquidityQuote {
  poolId: string;
  /** Amount the provider supplies, in token base units. */
  amountIn: bigint;
  /** Expected shares minted (deposit) or assets returned (withdrawal). */
  expectedOut: bigint;
  /** Expected price impact of the action in basis points. */
  priceImpactBps: number;
  /** Epoch milliseconds at which the quoted state was produced. */
  quotedAtMs: number;
  /** Network the quote was produced against. */
  networkId: string;
  /** Opaque version of the pool accounting state used for the quote. */
  stateCheckpoint: string;
  /** Stable identifier of the quote response, used in audit logs. */
  quoteId: string;
}

/** The user-approved, enforceable bound attached to a call. */
export interface QuoteBound {
  kind: "min-output" | "max-impact";
  /** Minimum output the transaction must return, in base units. */
  minOutput: bigint;
  /** Maximum acceptable price impact in basis points. */
  maxSlippageBps: number;
  quoteId: string;
  poolId: string;
  amountIn: bigint;
  quotedAtMs: number;
  networkId: string;
  stateCheckpoint: string;
}

/** Immutable arguments shared by simulation and transaction construction. */
export interface BoundCallArguments {
  readonly minAmountOut: string;
  readonly maxSlippageBps: number;
  readonly quoteTimestampMs: number;
  readonly quoteId: string;
  readonly poolId: string;
}

export type BoundCheckFailure =
  | "NO_BOUND"
  | "UNENFORCEABLE_BOUND"
  | "QUOTE_EXPIRED"
  | "NETWORK_CHANGED"
  | "STATE_CHANGED";

export type BoundCheckResult =
  | { ok: true; bound: QuoteBound; arguments: BoundCallArguments }
  | { ok: false; reason: BoundCheckFailure; message: string };

export interface BoundOptions {
  /** User-visible minimum output in base units. */
  minOutput?: bigint;
  /** User-visible maximum slippage / price impact in basis points. */
  maxSlippageBps?: number;
}

interface BoundContext {
  poolId: string;
  amountIn: bigint;
  networkId: string;
  stateCheckpoint: string;
}

export const MAX_SLIPPAGE_BPS = 10_000;
export const DEFAULT_QUOTE_MAX_AGE_MS = 300_000;

const FAILURE_MESSAGES: Record<BoundCheckFailure, string> = {
  NO_BOUND: "Set a maximum price impact or minimum output before submitting.",
  UNENFORCEABLE_BOUND: "The selected bound cannot be enforced against this quote.",
  QUOTE_EXPIRED: "This quote has expired. Refresh it before submitting.",
  NETWORK_CHANGED: "The network changed. Refresh the quote before submitting.",
  STATE_CHANGED: "Pool state changed. Refresh the quote before submitting.",
};

/** Floor of `value * (10_000 - bps) / 10_000`, using BigInt throughout. */
export function computeMinOutput(expectedOut: bigint, maxSlippageBps: number): bigint {
  if (maxSlippageBps < 0 || maxSlippageBps > MAX_SLIPPAGE_BPS) {
    throw new RangeError("maxSlippageBps must be between 0 and 10_000");
  }
  return (expectedOut * BigInt(MAX_SLIPPAGE_BPS - maxSlippageBps)) / BigInt(MAX_SLIPPAGE_BPS);
}

/**
 * Build an enforceable bound from user input.
 *
 * Exactly one of `minOutput` / `maxSlippageBps` is required; when both are
 * supplied the stricter value wins so the transaction can never be looser
 * than what the user approved.
 */
export function buildQuoteBound(
  quote: LiquidityQuote,
  options: BoundOptions,
): QuoteBound | null {
  const fromImpact =
    options.maxSlippageBps == null
      ? null
      : computeMinOutput(quote.expectedOut, options.maxSlippageBps);

  let minOutput: bigint | undefined = options.minOutput;
  if (fromImpact != null) {
    minOutput = minOutput == null ? fromImpact : (minOutput > fromImpact ? minOutput : fromImpact);
  }
  if (minOutput == null) return null;
  if (minOutput <= 0n || minOutput > quote.expectedOut) return null;

  // Derive the equivalent impact bound from the chosen minimum output so a
  // single, exact value is used everywhere downstream.
  const impliedImpact = Number((minOutput * 10_000n) / quote.expectedOut);
  const maxSlippageBps = MAX_SLIPPAGE_BPS - impliedImpact;

  return {
    kind: options.maxSlippageBps == null ? "min-output" : "max-impact",
    minOutput,
    maxSlippageBps,
    quoteId: quote.quoteId,
    poolId: quote.poolId,
    amountIn: quote.amountIn,
    quotedAtMs: quote.quotedAtMs,
    networkId: quote.networkId,
    stateCheckpoint: quote.stateCheckpoint,
  };
}

/** Produce the exact immutable argument object used for simulation and tx. */
export function toBoundCallArguments(bound: QuoteBound): BoundCallArguments {
  return Object.freeze({
    minAmountOut: bound.minOutput.toString(),
    maxSlippageBps: bound.maxSlippageBps,
    quoteTimestampMs: bound.quotedAtMs,
    quoteId: bound.quoteId,
    poolId: bound.poolId,
  });
}

/**
 * Verify that a bound is still enforceable in the current context.
 *
 * The ordering is fixed: missing bound → network → pool state → expiry.  A
 * material state change is reported before expiry because the user's next
 * action should be "refresh", regardless of which invariant broke first.
 */
export function verifyQuoteBound(
  bound: QuoteBound | null,
  context: BoundContext,
  now: number = Date.now(),
  maxAgeMs: number = DEFAULT_QUOTE_MAX_AGE_MS,
): BoundCheckResult {
  if (bound == null) return failure("NO_BOUND");
  if (bound.minOutput <= 0n || bound.maxSlippageBps < 0 || bound.maxSlippageBps > MAX_SLIPPAGE_BPS) {
    return failure("UNENFORCEABLE_BOUND");
  }
  if (bound.networkId !== context.networkId) return failure("NETWORK_CHANGED");
  if (
    bound.poolId !== context.poolId ||
    bound.amountIn !== context.amountIn ||
    bound.stateCheckpoint !== context.stateCheckpoint
  ) {
    return failure("STATE_CHANGED");
  }
  if (now - bound.quotedAtMs > maxAgeMs || now < bound.quotedAtMs) {
    return failure("QUOTE_EXPIRED");
  }
  return { ok: true, bound, arguments: toBoundCallArguments(bound) };
}

/** Simulate with exactly the arguments that will later build the transaction. */
export function simulateWithBound(
  bound: QuoteBound | null,
  context: BoundContext,
  now?: number,
  maxAgeMs?: number,
): BoundCheckResult {
  return verifyQuoteBound(bound, context, now, maxAgeMs);
}

/**
 * Build the transaction payload from a *verified* bound.  Re-verifies so a
 * caller cannot skip the freshness checks by passing a stale bound directly.
 */
export function buildBoundTransaction(
  bound: QuoteBound | null,
  context: BoundContext,
  now?: number,
  maxAgeMs?: number,
): BoundCheckResult {
  return verifyQuoteBound(bound, context, now, maxAgeMs);
}

function failure(reason: BoundCheckFailure): BoundCheckResult {
  return { ok: false, reason, message: FAILURE_MESSAGES[reason] };
}
