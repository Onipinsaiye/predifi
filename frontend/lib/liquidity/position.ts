/**
 * Liquidity-provider position status (issue #273).
 *
 * LP positions carry different information from ordinary prediction positions:
 * the deposited principal, the current share fraction, fees accrued so far,
 * any lifecycle lock that prevents a withdrawal, and the checkpoint the
 * indexer last saw.  This module derives a single, deterministic status from
 * those inputs so the UI can both explain the position and gate actions.
 *
 * Withdrawal eligibility is intentionally conservative: if any input needed
 * to verify eligibility is missing or stale, actions are disabled and the
 * reason is surfaced to the user.  We never guess that a withdrawal is safe.
 */

import { getFreshness, type FreshnessInfo } from "@/lib/freshness";
import {
  POOL_LIFECYCLE_STATES,
  resolveLifecycleState,
  type ResolvedLifecycleState,
} from "@/lib/pools";

/** Amounts are token base units (stroops for native XLM); 1 token = 10^7 units. */
export interface LiquidityPosition {
  poolId: string;
  /** Principal the provider deposited, in token base units. */
  depositedValue: bigint;
  /** Provider's current LP share balance. */
  shares: bigint;
  /** Pool-wide outstanding LP shares. */
  totalShares: bigint;
  /** LP fees accrued to this provider so far, in token base units. */
  accruedFees: bigint;
  /** Lifecycle state of the underlying market. */
  marketState: unknown;
  /** ISO timestamp / epoch ms of the last indexed checkpoint, when known. */
  lastIndexedAt?: string | number | null;
  /** Optional explicit lock set by the market (dispute, timelock, etc.). */
  lifecycleLocked?: boolean;
}

export type WithdrawalEligibility =
  | "eligible"
  | "no-shares"
  | "locked"
  | "stale"
  | "unavailable";

export interface LiquidityPositionStatus {
  /** Provider's share of the pool as basis points (10_000 = 100%). */
  shareBps: number;
  /** Human-readable share fraction, e.g. "12.34%". */
  shareFraction: string;
  freshness: FreshnessInfo;
  lifecycleState: ResolvedLifecycleState;
  eligibility: WithdrawalEligibility;
  /** Whether a withdrawal action may be offered to the user. */
  canWithdraw: boolean;
  /** Ordered, human-readable explanations; safe to expose via screen readers. */
  reasons: string[];
}

/** Lifecycle states in which direct LP withdrawal is not permitted. */
export const LP_WITHDRAWAL_LOCKED_STATES: readonly ResolvedLifecycleState[] = [
  "draft",
  "scheduled",
  "paused",
  "closed",
  "resolved",
  "cancelled",
  "unknown",
];

function formatShareFraction(shareBps: number): string {
  if (!Number.isFinite(shareBps)) return "—";
  return `${(shareBps / 100).toFixed(2)}%`;
}

/**
 * Derive the full display + eligibility status for an LP position.
 *
 * The check order is fixed so `reasons` is stable regardless of how the
 * position object was constructed: zero shares → lifecycle lock → freshness.
 */
export function deriveLiquidityPositionStatus(
  position: LiquidityPosition,
  now: number = Date.now(),
  staleThresholdMs?: number,
): LiquidityPositionStatus {
  const freshness = getFreshness(position.lastIndexedAt, now, staleThresholdMs);
  const lifecycleState = resolveLifecycleState(position.marketState);

  const shareBps =
    position.totalShares > 0n
      ? Number((position.shares * 10_000n) / position.totalShares)
      : 0;

  const reasons: string[] = [];
  let eligibility: WithdrawalEligibility = "eligible";

  if (position.shares <= 0n) {
    eligibility = "no-shares";
    reasons.push("You have no LP shares in this pool.");
  }

  if (
    position.lifecycleLocked === true ||
    LP_WITHDRAWAL_LOCKED_STATES.includes(lifecycleState)
  ) {
    if (eligibility === "eligible") eligibility = "locked";
    reasons.push(
      `Withdrawals are locked while the market is ${
        lifecycleState === "unknown" ? "in an unknown state" : lifecycleState
      }.`,
    );
  }

  if (freshness.state === "stale") {
    if (eligibility === "eligible") eligibility = "stale";
    reasons.push("Position data is stale; refresh the checkpoint before withdrawing.");
  } else if (freshness.state === "unavailable") {
    if (eligibility === "eligible") eligibility = "unavailable";
    reasons.push("Withdrawal eligibility cannot be verified without a fresh checkpoint.");
  }

  if (reasons.length === 0) {
    reasons.push("Withdrawals are available for this position.");
  }

  return {
    shareBps,
    shareFraction: formatShareFraction(shareBps),
    freshness,
    lifecycleState,
    eligibility,
    canWithdraw: eligibility === "eligible",
    reasons,
  };
}

/** Re-export for consumers that need to validate a raw lifecycle string. */
export const knownLifecycleStates = POOL_LIFECYCLE_STATES;
