/**
 * Centrally maintained market terminology glossary.
 *
 * Definitions are concise and product-neutral. They explain protocol terms
 * without changing contract terminology or implying investment outcomes.
 * Terms are linked from market and transaction flows via the GlossaryTerm
 * component.
 */

export interface GlossaryEntry {
  /** Canonical term key, e.g. "close", "resolve", "claim". */
  key: string;
  /** Term as displayed to users. */
  term: string;
  /** Concise, product-neutral definition. */
  definition: string;
  /** Optional longer explanation shown in the glossary detail view. */
  explanation?: string;
  /** Related term keys for cross-referencing. */
  related?: string[];
}

/** All glossary entries, keyed by term for O(1) lookup. */
export const GLOSSARY: Record<string, GlossaryEntry> = {
  close: {
    key: "close",
    term: "Close",
    definition:
      "The deadline after which no new predictions can be placed on a market. Set by the market creator and recorded on-chain.",
    explanation:
      "Once a market closes, trading stops. The outcome is then resolved based on the resolution source, and payouts are calculated for winning positions.",
    related: ["resolve", "claim"],
  },
  resolve: {
    key: "resolve",
    term: "Resolve",
    definition:
      "The process of determining the winning outcome of a market after it has closed. Resolution is performed by the designated resolution source.",
    explanation:
      "After a market closes, the resolution source reports the outcome. Once resolved, positions on the winning outcome become eligible for claim.",
    related: ["close", "claim", "resolution-source"],
  },
  claim: {
    key: "claim",
    term: "Claim",
    definition:
      "The action of withdrawing your payout after a market has been resolved in your favor. Claims are submitted on-chain and require a transaction fee.",
    explanation:
      "Only winning positions can be claimed. The claimable amount is your stake plus your share of the losing stakes, minus protocol fees.",
    related: ["resolve", "payout"],
  },
  probability: {
    key: "probability",
    term: "Probability",
    definition:
      "The market-implied chance of an outcome occurring, derived from the ratio of stakes across all outcomes. Not a guarantee of the result.",
    explanation:
      "Probability = stake on this outcome / total stake across all outcomes. It reflects where traders are placing their money, not the actual likelihood of the event.",
    related: ["odds", "stake"],
  },
  odds: {
    key: "odds",
    term: "Odds",
    definition:
      "The payout multiplier for a winning outcome, derived from the total stake divided by the stake on that outcome. Higher odds mean less money is on that outcome.",
    explanation:
      "Odds = total stake / stake on this outcome. They change as more predictions are placed and are not fixed at market creation.",
    related: ["probability", "stake"],
  },
  stake: {
    key: "stake",
    term: "Stake",
    definition:
      "The amount you commit when placing a prediction on an outcome. Your stake is locked until the market is resolved.",
    explanation:
      "If your outcome wins, your stake plus a share of the losing stakes (minus fees) becomes claimable. If it loses, your stake is distributed to winners.",
    related: ["probability", "odds", "claim"],
  },
  "resolution-source": {
    key: "resolution-source",
    term: "Resolution Source",
    definition:
      "The designated entity or mechanism that reports the winning outcome after a market closes. The frontend does not choose or override this.",
    explanation:
      "The resolution source is set by the market creator at market creation. Its report is the authoritative input for resolving the market.",
    related: ["resolve", "close"],
  },
  payout: {
    key: "payout",
    term: "Payout",
    definition:
      "The total amount received when claiming a winning position: your original stake plus your portion of the losing stakes, minus protocol fees.",
    explanation:
      "Payouts are calculated after resolution and are claimable on-chain. The frontend displays estimates; on-chain execution is authoritative.",
    related: ["claim", "stake"],
  },
  "position": {
    key: "position",
    term: "Position",
    definition:
      "Your active prediction on a specific market outcome, including the amount staked and the outcome selected.",
    explanation:
      "Positions remain open until the market closes and resolves. Winning positions become claimable; losing positions are settled against winners.",
    related: ["stake", "claim", "resolve"],
  },
};

/** Look up a glossary entry by key. Returns null when the term is not tracked. */
export function getGlossaryEntry(key: string): GlossaryEntry | null {
  return GLOSSARY[key] ?? null;
}

/** All glossary entries sorted alphabetically by term. */
export function getAllGlossaryEntries(): GlossaryEntry[] {
  return Object.values(GLOSSARY).sort((a, b) =>
    a.term.localeCompare(b.term),
  );
}

/** Terms commonly referenced at market decision points. */
export const DECISION_POINT_TERMS: readonly string[] = [
  "close",
  "resolve",
  "claim",
  "probability",
  "odds",
  "stake",
  "position",
  "resolution-source",
  "payout",
] as const;
