/**
 * Moderated reactions for market discussion (issue #282).
 *
 * Design decisions:
 * - Fixed reaction set so reactions cannot carry arbitrary signals.
 * - One active reaction per wallet per comment — toggling the current
 *   reaction removes it; selecting a different one replaces it.
 * - Counts are eventually consistent: they reflect client-side state and
 *   must be clearly disclosed as non-authoritative.
 * - Reactions do not affect resolution, price, or any on-chain value.
 *   This invariant is documented in the type system and enforced by
 *   keeping the reactions store completely separate from pool data.
 *
 * Validation covers duplicate submissions and idempotent count aggregation
 * (re-selecting the same reaction is a no-op).
 */

/** The fixed, moderated set of allowed reactions. */
export const REACTION_TYPES = ["👍", "🔥", "🤔", "🎯", "⚠️"] as const;
export type ReactionType = (typeof REACTION_TYPES)[number];

/** Human-readable labels for accessibility and tooltips. */
export const REACTION_LABELS: Record<ReactionType, string> = {
  "👍": "Agree",
  "🔥": "Hot take",
  "🤔": "Skeptical",
  "🎯": "Spot on",
  "⚠️": "Caution",
};

/** Returns true when value is a member of REACTION_TYPES. */
export function isValidReaction(value: unknown): value is ReactionType {
  return (REACTION_TYPES as readonly unknown[]).includes(value);
}

/**
 * Per-comment reaction aggregate.
 * commentId → (reactionType → Set of wallet addresses that selected it).
 */
const store = new Map<string, Map<ReactionType, Set<string>>>();

function getOrCreate(commentId: string): Map<ReactionType, Set<string>> {
  let entry = store.get(commentId);
  if (!entry) {
    entry = new Map();
    store.set(commentId, entry);
  }
  return entry;
}

/**
 * Find which reaction, if any, the wallet has placed on a comment.
 * Returns undefined when the wallet has not reacted.
 */
export function getWalletReaction(
  commentId: string,
  walletAddress: string,
): ReactionType | undefined {
  const entry = store.get(commentId);
  if (!entry) return undefined;
  for (const [reaction, wallets] of entry.entries()) {
    if (wallets.has(walletAddress)) return reaction;
  }
  return undefined;
}

/** Snapshot of reaction counts for a comment, ordered by REACTION_TYPES. */
export interface ReactionCounts {
  [reaction: string]: number;
}

/** Return current reaction counts for a comment. */
export function getReactionCounts(commentId: string): ReactionCounts {
  const entry = store.get(commentId);
  const counts: ReactionCounts = {};
  for (const reaction of REACTION_TYPES) {
    counts[reaction] = entry?.get(reaction)?.size ?? 0;
  }
  return counts;
}

export interface ToggleReactionResult {
  /** The reaction that was toggled. */
  reaction: ReactionType;
  /**
   * "added" — the reaction is now active for this wallet.
   * "removed" — the wallet had this reaction and it was cleared.
   * "replaced" — the wallet had a different reaction which was replaced.
   * "noop" — no change (should not occur in normal usage).
   */
  outcome: "added" | "removed" | "replaced" | "noop";
  /** Updated counts after the operation. */
  counts: ReactionCounts;
}

/**
 * Toggle a reaction for a wallet on a comment.
 *
 * Rules:
 * - If the wallet has the same reaction already → remove it (toggle off).
 * - If the wallet has a different reaction → replace it.
 * - If the wallet has no reaction → add it.
 * - Re-sending the exact current state is handled gracefully (noop).
 *
 * This satisfies idempotent count aggregation: calling toggle twice with the
 * same type returns to the original state with the original count.
 */
export function toggleReaction(
  commentId: string,
  walletAddress: string,
  reaction: ReactionType,
): ToggleReactionResult {
  if (!isValidReaction(reaction)) {
    throw new Error(`"${String(reaction)}" is not in the allowed reaction set.`);
  }

  const entry = getOrCreate(commentId);
  const existing = getWalletReaction(commentId, walletAddress);

  if (existing === reaction) {
    // Toggle off — remove the reaction.
    entry.get(reaction)?.delete(walletAddress);
    return { reaction, outcome: "removed", counts: getReactionCounts(commentId) };
  }

  if (existing !== undefined) {
    // Replace — remove old reaction first.
    entry.get(existing)?.delete(walletAddress);
  }

  // Add the new reaction.
  if (!entry.has(reaction)) {
    entry.set(reaction, new Set());
  }
  entry.get(reaction)!.add(walletAddress);

  const outcome = existing === undefined ? "added" : "replaced";
  return { reaction, outcome, counts: getReactionCounts(commentId) };
}

/** Remove all reactions for a comment (e.g. when a comment is deleted). */
export function clearReactions(commentId: string): void {
  store.delete(commentId);
}

/** Seed deterministic demo reactions for a comment. Used in tests and demos. */
export function seedDemoReactions(commentId: string): void {
  if (store.has(commentId)) return;
  const entry = new Map<ReactionType, Set<string>>();
  entry.set("👍", new Set(["GDEMO_A", "GDEMO_B"]));
  entry.set("🔥", new Set(["GDEMO_C"]));
  store.set(commentId, entry);
}
