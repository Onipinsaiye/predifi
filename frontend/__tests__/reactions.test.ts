/**
 * Tests for comment reactions (issue #282).
 *
 * Covers: fixed reaction set validation, one-active-per-wallet, duplicate
 * submissions, wallet changes, idempotent count aggregation, and the
 * structural guarantee that reactions never touch pool/on-chain data.
 */

import {
  REACTION_TYPES,
  isValidReaction,
  getWalletReaction,
  getReactionCounts,
  toggleReaction,
  clearReactions,
  seedDemoReactions,
} from "../lib/reactions";

// Use distinct comment IDs per test to avoid store bleed-through.
let _counter = 1;
function nextId(): string {
  return `test_comment_${_counter++}`;
}

describe("isValidReaction", () => {
  it("accepts all REACTION_TYPES members", () => {
    for (const r of REACTION_TYPES) {
      expect(isValidReaction(r)).toBe(true);
    }
  });

  it("rejects arbitrary strings", () => {
    expect(isValidReaction("💰")).toBe(false);
    expect(isValidReaction("bullish")).toBe(false);
    expect(isValidReaction("")).toBe(false);
  });

  it("rejects non-strings", () => {
    expect(isValidReaction(42)).toBe(false);
    expect(isValidReaction(null)).toBe(false);
    expect(isValidReaction(undefined)).toBe(false);
  });
});

describe("getReactionCounts", () => {
  it("returns zero counts for a comment with no reactions", () => {
    const id = nextId();
    const counts = getReactionCounts(id);
    for (const r of REACTION_TYPES) {
      expect(counts[r]).toBe(0);
    }
  });

  it("includes all fixed reaction types in the counts object", () => {
    const id = nextId();
    const counts = getReactionCounts(id);
    for (const r of REACTION_TYPES) {
      expect(r in counts).toBe(true);
    }
  });
});

describe("toggleReaction — add", () => {
  it("adds a reaction and returns outcome=added", () => {
    const id = nextId();
    const result = toggleReaction(id, "WALLET_A", "👍");
    expect(result.outcome).toBe("added");
    expect(result.reaction).toBe("👍");
    expect(result.counts["👍"]).toBe(1);
  });

  it("sets the wallet reaction after adding", () => {
    const id = nextId();
    toggleReaction(id, "WALLET_A", "🔥");
    expect(getWalletReaction(id, "WALLET_A")).toBe("🔥");
  });

  it("two different wallets can add independent reactions", () => {
    const id = nextId();
    toggleReaction(id, "WALLET_A", "👍");
    toggleReaction(id, "WALLET_B", "🔥");
    expect(getReactionCounts(id)["👍"]).toBe(1);
    expect(getReactionCounts(id)["🔥"]).toBe(1);
  });
});

describe("toggleReaction — remove (toggle off)", () => {
  it("removes the reaction when called with the active type", () => {
    const id = nextId();
    toggleReaction(id, "WALLET_A", "👍");
    const result = toggleReaction(id, "WALLET_A", "👍");
    expect(result.outcome).toBe("removed");
    expect(result.counts["👍"]).toBe(0);
  });

  it("clears wallet reaction after removal", () => {
    const id = nextId();
    toggleReaction(id, "WALLET_A", "🎯");
    toggleReaction(id, "WALLET_A", "🎯");
    expect(getWalletReaction(id, "WALLET_A")).toBeUndefined();
  });

  it("idempotent aggregation: add → remove → add yields count 1", () => {
    const id = nextId();
    toggleReaction(id, "WALLET_A", "👍");
    toggleReaction(id, "WALLET_A", "👍"); // remove
    toggleReaction(id, "WALLET_A", "👍"); // add again
    expect(getReactionCounts(id)["👍"]).toBe(1);
  });
});

describe("toggleReaction — replace", () => {
  it("replaces the previous reaction", () => {
    const id = nextId();
    toggleReaction(id, "WALLET_A", "👍");
    const result = toggleReaction(id, "WALLET_A", "🔥");
    expect(result.outcome).toBe("replaced");
    expect(result.counts["👍"]).toBe(0);
    expect(result.counts["🔥"]).toBe(1);
  });

  it("wallet can only hold one reaction at a time", () => {
    const id = nextId();
    toggleReaction(id, "WALLET_A", "👍");
    toggleReaction(id, "WALLET_A", "🤔");
    // Should have exactly one reaction active, not two
    let active = 0;
    for (const r of REACTION_TYPES) {
      if (getWalletReaction(id, "WALLET_A") === r) active++;
    }
    // getWalletReaction returns at most one value
    const current = getWalletReaction(id, "WALLET_A");
    expect(current).toBe("🤔");
    expect(active).toBe(1);
  });
});

describe("duplicate submission", () => {
  it("re-selecting the same reaction (toggle twice) returns to original count", () => {
    const id = nextId();
    toggleReaction(id, "WALLET_A", "⚠️");
    expect(getReactionCounts(id)["⚠️"]).toBe(1);
    toggleReaction(id, "WALLET_A", "⚠️");
    expect(getReactionCounts(id)["⚠️"]).toBe(0);
    // Count never goes negative
    expect(getReactionCounts(id)["⚠️"]).toBeGreaterThanOrEqual(0);
  });

  it("multiple wallets adding the same reaction accumulates correctly", () => {
    const id = nextId();
    toggleReaction(id, "WALLET_A", "👍");
    toggleReaction(id, "WALLET_B", "👍");
    toggleReaction(id, "WALLET_C", "👍");
    expect(getReactionCounts(id)["👍"]).toBe(3);
  });
});

describe("wallet change", () => {
  it("different wallets have independent active reactions", () => {
    const id = nextId();
    toggleReaction(id, "WALLET_A", "👍");
    toggleReaction(id, "WALLET_B", "🔥");
    expect(getWalletReaction(id, "WALLET_A")).toBe("👍");
    expect(getWalletReaction(id, "WALLET_B")).toBe("🔥");
  });

  it("removing wallet A reaction does not affect wallet B", () => {
    const id = nextId();
    toggleReaction(id, "WALLET_A", "👍");
    toggleReaction(id, "WALLET_B", "👍");
    toggleReaction(id, "WALLET_A", "👍"); // remove A
    expect(getReactionCounts(id)["👍"]).toBe(1); // B still has it
    expect(getWalletReaction(id, "WALLET_A")).toBeUndefined();
    expect(getWalletReaction(id, "WALLET_B")).toBe("👍");
  });
});

describe("invalid reaction", () => {
  it("throws when reaction is not in the fixed set", () => {
    const id = nextId();
    // Cast to satisfy TypeScript in the test
    expect(() => toggleReaction(id, "WALLET_A", "💰" as never)).toThrow();
  });
});

describe("clearReactions", () => {
  it("removes all reactions for a comment", () => {
    const id = nextId();
    toggleReaction(id, "WALLET_A", "👍");
    toggleReaction(id, "WALLET_B", "🔥");
    clearReactions(id);
    for (const r of REACTION_TYPES) {
      expect(getReactionCounts(id)[r]).toBe(0);
    }
  });
});

describe("seedDemoReactions", () => {
  it("seeds demo data idempotently", () => {
    const id = nextId();
    seedDemoReactions(id);
    const counts1 = getReactionCounts(id);
    seedDemoReactions(id); // second call should not add more
    const counts2 = getReactionCounts(id);
    expect(counts1["👍"]).toBe(counts2["👍"]);
  });
});

describe("structural: reactions never touch on-chain data", () => {
  it("toggleReaction result carries no pool fields", () => {
    const id = nextId();
    const result = toggleReaction(id, "WALLET_A", "👍");
    expect((result as Record<string, unknown>).total_stake).toBeUndefined();
    expect((result as Record<string, unknown>).odds).toBeUndefined();
    expect((result as Record<string, unknown>).state).toBeUndefined();
  });
});
