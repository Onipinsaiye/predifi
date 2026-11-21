/**
 * Shareable resolution evidence cards (issue #284).
 *
 * Resolved markets need to be easy to cite while preserving:
 * - Canonical resolution data (outcome, resolver, timestamp).
 * - Evidence references (source URLs, descriptions).
 * - Transaction links (on-chain proof).
 * - Challenge/dispute status.
 * - Clear distinction of disputed or superseded data.
 *
 * This module provides:
 * - Deterministic card generation from canonical resolution data.
 * - Type-safe resolution outcomes for successful, challenged, refunded, and
 *   unavailable-evidence cases.
 * - A shareable page URL builder (deterministic: same inputs → same URL).
 */

/** All possible resolution outcomes for a prediction market. */
export const RESOLUTION_OUTCOMES = [
  "resolved",   // Settled with available evidence
  "challenged", // Result disputed and under review
  "refunded",   // Market cancelled; stakes returned
  "unavailable", // Evidence cannot be retrieved or was removed
] as const;

export type ResolutionOutcome = (typeof RESOLUTION_OUTCOMES)[number];

export function isValidResolutionOutcome(value: unknown): value is ResolutionOutcome {
  return (RESOLUTION_OUTCOMES as readonly unknown[]).includes(value);
}

/** An evidence reference cited during resolution. */
export interface EvidenceReference {
  /** Human-readable label for the evidence. */
  label: string;
  /** URL of the source, or null if the resource is unavailable. */
  url: string | null;
  /** True when the evidence URL is no longer reachable. */
  unavailable?: boolean;
}

/** On-chain transaction link. */
export interface TransactionLink {
  /** Label, e.g. "Resolution TX" or "Challenge TX". */
  label: string;
  /** Explorer URL for the transaction. */
  explorerUrl: string;
  /** Raw transaction hash. */
  txHash: string;
}

/** A challenge/dispute record. */
export interface ChallengeRecord {
  /** Address of the challenger. */
  challengerAddress: string;
  /** ISO timestamp of the challenge. */
  challengedAt: string;
  /** Short reason provided by the challenger. */
  reason: string;
  /** Whether the challenge was upheld (resolution changed) or rejected. */
  outcome?: "upheld" | "rejected" | "pending";
}

/**
 * Canonical resolution data for a prediction market.
 * Generated deterministically from on-chain + indexer data.
 */
export interface ResolutionCard {
  /** Unique identifier — deterministic from pool_id + resolution timestamp. */
  cardId: string;
  poolId: number;
  /** The question/name of the market. */
  question: string;
  outcome: ResolutionOutcome;
  /**
   * The winning outcome index (for "resolved" markets).
   * null for cancelled/unavailable markets.
   */
  winningOutcome: number | null;
  /** Winning outcome label, if available. */
  winningOutcomeLabel: string | null;
  /** Address of the resolver (oracle or admin). */
  resolverAddress: string;
  /** ISO timestamp of resolution. */
  resolvedAt: string;
  /** Evidence references cited during resolution. */
  evidenceReferences: EvidenceReference[];
  /** On-chain transaction links for the resolution (and any challenge). */
  transactionLinks: TransactionLink[];
  /** Challenge records. Empty for uncontested resolutions. */
  challenges: ChallengeRecord[];
  /**
   * Whether data for this card may be incomplete or superseded.
   * True when: challenged outcome, refunded market, or unavailable evidence.
   */
  isDisputed: boolean;
  /**
   * Short human-readable status message.
   * Shown prominently on the card.
   */
  statusSummary: string;
}

/**
 * Derive a deterministic card ID from pool ID and resolution timestamp.
 * Stable across re-renders and re-fetches as long as the inputs are the same.
 */
export function buildCardId(poolId: number, resolvedAt: string): string {
  // Simple deterministic string — in production this would be a stable hash.
  const epoch = Math.floor(new Date(resolvedAt).getTime() / 1000);
  return `rc_${poolId}_${epoch}`;
}

/**
 * Compute the status summary string for a resolution card.
 * Maps each outcome type to a clear, non-ambiguous label.
 */
export function buildStatusSummary(
  outcome: ResolutionOutcome,
  challenges: ChallengeRecord[],
): string {
  const activeChallenges = challenges.filter(
    (c) => c.outcome === "pending" || c.outcome === undefined,
  );
  if (activeChallenges.length > 0) {
    return `Challenged — resolution under review (${activeChallenges.length} open challenge${activeChallenges.length > 1 ? "s" : ""})`;
  }
  switch (outcome) {
    case "resolved":
      return "Resolved — outcome confirmed";
    case "challenged":
      return challenges.some((c) => c.outcome === "upheld")
        ? "Challenged — original resolution superseded"
        : "Challenged — dispute rejected, original resolution stands";
    case "refunded":
      return "Refunded — market cancelled, stakes returned";
    case "unavailable":
      return "Evidence unavailable — resolution data cannot be verified";
  }
}

/**
 * Build a ResolutionCard from canonical data.
 * Derives `isDisputed` and `statusSummary` automatically.
 */
export function buildResolutionCard(params: {
  poolId: number;
  question: string;
  outcome: ResolutionOutcome;
  winningOutcome: number | null;
  winningOutcomeLabel?: string | null;
  resolverAddress: string;
  resolvedAt: string;
  evidenceReferences?: EvidenceReference[];
  transactionLinks?: TransactionLink[];
  challenges?: ChallengeRecord[];
}): ResolutionCard {
  const {
    poolId,
    question,
    outcome,
    winningOutcome,
    winningOutcomeLabel = null,
    resolverAddress,
    resolvedAt,
    evidenceReferences = [],
    transactionLinks = [],
    challenges = [],
  } = params;

  const isDisputed =
    outcome === "challenged" ||
    outcome === "refunded" ||
    outcome === "unavailable" ||
    evidenceReferences.some((e) => e.unavailable) ||
    challenges.length > 0;

  const statusSummary = buildStatusSummary(outcome, challenges);

  return {
    cardId: buildCardId(poolId, resolvedAt),
    poolId,
    question,
    outcome,
    winningOutcome,
    winningOutcomeLabel,
    resolverAddress,
    resolvedAt,
    evidenceReferences,
    transactionLinks,
    challenges,
    isDisputed,
    statusSummary,
  };
}

/**
 * Build a deterministic shareable URL for a resolution card.
 * The URL encodes only stable, canonical fields — it never changes for the
 * same resolution event.
 *
 * @param baseUrl - Public frontend base URL.
 * @param cardId  - The deterministic card ID from buildCardId.
 */
export function buildResolutionShareUrl(baseUrl: string, cardId: string): string {
  return `${baseUrl.replace(/\/$/, "")}/resolution/${encodeURIComponent(cardId)}`;
}

/**
 * Validate that a transaction hash looks like a valid Stellar TX hash.
 * Stellar transaction hashes are 64 hex characters.
 */
export function isValidTxHash(hash: unknown): boolean {
  if (typeof hash !== "string") return false;
  return /^[0-9a-fA-F]{64}$/.test(hash);
}

/**
 * Redact sensitive or stale data from a card when it has been superseded
 * by a successful challenge. Returns a new card with superseded fields
 * clearly marked.
 */
export function markSuperseded(card: ResolutionCard): ResolutionCard {
  return {
    ...card,
    isDisputed: true,
    statusSummary:
      "Superseded — this resolution was overturned by a successful challenge",
    winningOutcome: null,
    winningOutcomeLabel: null,
  };
}
