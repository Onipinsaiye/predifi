/**
 * Tests for shareable resolution evidence cards (issue #284).
 *
 * Covers all acceptance criteria:
 * - Deterministic card ID generation from canonical data.
 * - Correct handling of successful, challenged, refunded, and
 *   unavailable-evidence cases.
 * - Clear distinction of disputed/superseded data.
 * - Evidence references and transaction link fields.
 * - Share URL determinism.
 * - TX hash validation.
 */

import {
  buildCardId,
  buildResolutionCard,
  buildResolutionShareUrl,
  buildStatusSummary,
  isValidResolutionOutcome,
  isValidTxHash,
  markSuperseded,
  RESOLUTION_OUTCOMES,
} from "../lib/resolutionEvidence";

// ---------------------------------------------------------------------------
// isValidResolutionOutcome
// ---------------------------------------------------------------------------

describe("isValidResolutionOutcome", () => {
  it("accepts all RESOLUTION_OUTCOMES members", () => {
    for (const o of RESOLUTION_OUTCOMES) {
      expect(isValidResolutionOutcome(o)).toBe(true);
    }
  });

  it("rejects unknown strings", () => {
    expect(isValidResolutionOutcome("cancelled")).toBe(false);
    expect(isValidResolutionOutcome("disputed")).toBe(false);
  });

  it("rejects non-strings", () => {
    expect(isValidResolutionOutcome(null)).toBe(false);
    expect(isValidResolutionOutcome(1)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// buildCardId — deterministic
// ---------------------------------------------------------------------------

describe("buildCardId", () => {
  it("returns the same ID for the same inputs", () => {
    const id1 = buildCardId(42, "2025-01-15T12:00:00Z");
    const id2 = buildCardId(42, "2025-01-15T12:00:00Z");
    expect(id1).toBe(id2);
  });

  it("returns different IDs for different poolIds", () => {
    expect(buildCardId(1, "2025-01-15T12:00:00Z")).not.toBe(
      buildCardId(2, "2025-01-15T12:00:00Z"),
    );
  });

  it("returns different IDs for different timestamps", () => {
    expect(buildCardId(1, "2025-01-15T12:00:00Z")).not.toBe(
      buildCardId(1, "2025-01-16T12:00:00Z"),
    );
  });

  it("includes the pool ID in the card ID string", () => {
    expect(buildCardId(99, "2025-01-15T12:00:00Z")).toContain("99");
  });
});

// ---------------------------------------------------------------------------
// buildStatusSummary
// ---------------------------------------------------------------------------

describe("buildStatusSummary", () => {
  it("returns confirmed message for resolved outcome with no challenges", () => {
    const msg = buildStatusSummary("resolved", []);
    expect(msg).toContain("confirmed");
  });

  it("includes open challenge count when challenges are pending", () => {
    const msg = buildStatusSummary("resolved", [
      {
        challengerAddress: "GADDR",
        challengedAt: "2025-01-15T12:00:00Z",
        reason: "Disputed",
        outcome: "pending",
      },
    ]);
    expect(msg).toContain("1 open challenge");
    expect(msg).toContain("under review");
  });

  it("distinguishes upheld from rejected challenges", () => {
    const upheld = buildStatusSummary("challenged", [
      {
        challengerAddress: "GADDR",
        challengedAt: "2025-01-15T12:00:00Z",
        reason: "Faulty evidence",
        outcome: "upheld",
      },
    ]);
    expect(upheld).toContain("superseded");

    const rejected = buildStatusSummary("challenged", [
      {
        challengerAddress: "GADDR",
        challengedAt: "2025-01-15T12:00:00Z",
        reason: "Invalid claim",
        outcome: "rejected",
      },
    ]);
    expect(rejected).toContain("stands");
  });

  it("returns refunded message for refunded outcome", () => {
    expect(buildStatusSummary("refunded", [])).toContain("cancelled");
  });

  it("returns unavailable message for unavailable outcome", () => {
    expect(buildStatusSummary("unavailable", [])).toContain("unavailable");
  });
});

// ---------------------------------------------------------------------------
// buildResolutionCard — successful resolution
// ---------------------------------------------------------------------------

describe("buildResolutionCard — successful", () => {
  const base = {
    poolId: 1,
    question: "Will BTC exceed $100k by end of 2025?",
    outcome: "resolved" as const,
    winningOutcome: 0,
    winningOutcomeLabel: "Yes",
    resolverAddress: "GRESOLVER",
    resolvedAt: "2025-01-15T12:00:00Z",
  };

  it("builds a card with isDisputed=false when no challenges or unavailable evidence", () => {
    const card = buildResolutionCard(base);
    expect(card.isDisputed).toBe(false);
  });

  it("sets winning outcome and label", () => {
    const card = buildResolutionCard(base);
    expect(card.winningOutcome).toBe(0);
    expect(card.winningOutcomeLabel).toBe("Yes");
  });

  it("generates a deterministic cardId", () => {
    const card1 = buildResolutionCard(base);
    const card2 = buildResolutionCard(base);
    expect(card1.cardId).toBe(card2.cardId);
  });

  it("includes evidence references", () => {
    const card = buildResolutionCard({
      ...base,
      evidenceReferences: [
        { label: "Oracle report", url: "https://oracle.example.com/1" },
      ],
    });
    expect(card.evidenceReferences).toHaveLength(1);
    expect(card.evidenceReferences[0].label).toBe("Oracle report");
  });

  it("includes transaction links", () => {
    const card = buildResolutionCard({
      ...base,
      transactionLinks: [
        {
          label: "Resolution TX",
          explorerUrl: "https://stellar.expert/explorer/testnet/tx/abc123",
          txHash: "a".repeat(64),
        },
      ],
    });
    expect(card.transactionLinks).toHaveLength(1);
    expect(card.transactionLinks[0].label).toBe("Resolution TX");
  });
});

// ---------------------------------------------------------------------------
// buildResolutionCard — challenged
// ---------------------------------------------------------------------------

describe("buildResolutionCard — challenged", () => {
  it("sets isDisputed=true for challenged outcome", () => {
    const card = buildResolutionCard({
      poolId: 2,
      question: "Market Q",
      outcome: "challenged",
      winningOutcome: 1,
      resolverAddress: "GRESOLVER",
      resolvedAt: "2025-02-01T00:00:00Z",
      challenges: [
        {
          challengerAddress: "GCHALLENGER",
          challengedAt: "2025-02-02T00:00:00Z",
          reason: "Oracle data was incorrect",
          outcome: "pending",
        },
      ],
    });
    expect(card.isDisputed).toBe(true);
    expect(card.challenges).toHaveLength(1);
  });

  it("statusSummary reflects open challenge", () => {
    const card = buildResolutionCard({
      poolId: 2,
      question: "Q",
      outcome: "challenged",
      winningOutcome: 1,
      resolverAddress: "GRESOLVER",
      resolvedAt: "2025-02-01T00:00:00Z",
      challenges: [
        {
          challengerAddress: "GCHALLENGER",
          challengedAt: "2025-02-02T00:00:00Z",
          reason: "Bad data",
          outcome: "pending",
        },
      ],
    });
    expect(card.statusSummary).toContain("under review");
  });
});

// ---------------------------------------------------------------------------
// buildResolutionCard — refunded
// ---------------------------------------------------------------------------

describe("buildResolutionCard — refunded", () => {
  it("builds a refunded card with isDisputed=true", () => {
    const card = buildResolutionCard({
      poolId: 3,
      question: "Market cancelled",
      outcome: "refunded",
      winningOutcome: null,
      resolverAddress: "GADMIN",
      resolvedAt: "2025-03-01T00:00:00Z",
    });
    expect(card.isDisputed).toBe(true);
    expect(card.winningOutcome).toBeNull();
    expect(card.statusSummary).toContain("cancelled");
  });
});

// ---------------------------------------------------------------------------
// buildResolutionCard — unavailable evidence
// ---------------------------------------------------------------------------

describe("buildResolutionCard — unavailable evidence", () => {
  it("sets isDisputed=true when any evidence reference is unavailable", () => {
    const card = buildResolutionCard({
      poolId: 4,
      question: "Market with lost evidence",
      outcome: "resolved",
      winningOutcome: 0,
      resolverAddress: "GRESOLVER",
      resolvedAt: "2025-04-01T00:00:00Z",
      evidenceReferences: [
        { label: "Missing source", url: null, unavailable: true },
      ],
    });
    expect(card.isDisputed).toBe(true);
  });

  it("builds unavailable-outcome card with correct status", () => {
    const card = buildResolutionCard({
      poolId: 5,
      question: "Market unverifiable",
      outcome: "unavailable",
      winningOutcome: null,
      resolverAddress: "GRESOLVER",
      resolvedAt: "2025-05-01T00:00:00Z",
    });
    expect(card.statusSummary).toContain("unavailable");
    expect(card.winningOutcome).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// markSuperseded
// ---------------------------------------------------------------------------

describe("markSuperseded", () => {
  it("marks a card as superseded", () => {
    const card = buildResolutionCard({
      poolId: 6,
      question: "Overturned market",
      outcome: "resolved",
      winningOutcome: 1,
      resolverAddress: "GRESOLVER",
      resolvedAt: "2025-06-01T00:00:00Z",
    });
    const superseded = markSuperseded(card);
    expect(superseded.isDisputed).toBe(true);
    expect(superseded.winningOutcome).toBeNull();
    expect(superseded.winningOutcomeLabel).toBeNull();
    expect(superseded.statusSummary.toLowerCase()).toContain("superseded");
  });

  it("does not mutate the original card", () => {
    const card = buildResolutionCard({
      poolId: 7,
      question: "Original",
      outcome: "resolved",
      winningOutcome: 0,
      resolverAddress: "GRESOLVER",
      resolvedAt: "2025-07-01T00:00:00Z",
    });
    markSuperseded(card);
    expect(card.winningOutcome).toBe(0);
    expect(card.isDisputed).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// buildResolutionShareUrl — deterministic
// ---------------------------------------------------------------------------

describe("buildResolutionShareUrl", () => {
  it("produces the same URL for the same inputs", () => {
    const id = buildCardId(42, "2025-01-15T12:00:00Z");
    const url1 = buildResolutionShareUrl("https://predifi.app", id);
    const url2 = buildResolutionShareUrl("https://predifi.app", id);
    expect(url1).toBe(url2);
  });

  it("includes the card ID in the URL", () => {
    const id = buildCardId(42, "2025-01-15T12:00:00Z");
    const url = buildResolutionShareUrl("https://predifi.app", id);
    expect(url).toContain(encodeURIComponent(id));
  });

  it("strips trailing slash from base URL", () => {
    const id = buildCardId(1, "2025-01-15T12:00:00Z");
    const url = buildResolutionShareUrl("https://predifi.app/", id);
    expect(url).not.toContain("//resolution");
  });

  it("includes /resolution/ in the path", () => {
    const id = buildCardId(1, "2025-01-15T12:00:00Z");
    const url = buildResolutionShareUrl("https://predifi.app", id);
    expect(url).toContain("/resolution/");
  });
});

// ---------------------------------------------------------------------------
// isValidTxHash
// ---------------------------------------------------------------------------

describe("isValidTxHash", () => {
  it("accepts a 64-character hex string", () => {
    expect(isValidTxHash("a".repeat(64))).toBe(true);
    expect(isValidTxHash("A".repeat(64))).toBe(true);
    expect(isValidTxHash("0123456789abcdef".repeat(4))).toBe(true);
  });

  it("rejects strings that are not 64 characters", () => {
    expect(isValidTxHash("abc")).toBe(false);
    expect(isValidTxHash("a".repeat(63))).toBe(false);
    expect(isValidTxHash("a".repeat(65))).toBe(false);
  });

  it("rejects strings with non-hex characters", () => {
    expect(isValidTxHash("g".repeat(64))).toBe(false);
    expect(isValidTxHash("z".repeat(64))).toBe(false);
  });

  it("rejects non-strings", () => {
    expect(isValidTxHash(null)).toBe(false);
    expect(isValidTxHash(undefined)).toBe(false);
    expect(isValidTxHash(42)).toBe(false);
  });
});
