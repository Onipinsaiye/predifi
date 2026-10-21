import { describe, expect, it } from "vitest";

import {
  MAX_DESCRIPTION_LENGTH,
  MAX_TITLE_LENGTH,
  buildCanonicalUrl,
  buildPoolDescription,
  buildPoolTitle,
  generatePoolMetadata,
  sanitizeText,
} from "../lib/metadata";
import { getPoolById, isValidPoolId } from "../lib/pools";

const BASE = "https://example.org";

describe("pool identifier validation", () => {
  it("accepts safe identifiers", () => {
    expect(isValidPoolId("eth-usdc")).toBe(true);
    expect(isValidPoolId("0xabc123")).toBe(true);
  });

  it("rejects malicious or malformed identifiers", () => {
    expect(isValidPoolId("<script>alert(1)</script>")).toBe(false);
    expect(isValidPoolId("")).toBe(false);
    expect(isValidPoolId("a".repeat(129))).toBe(false);
    expect(isValidPoolId(undefined)).toBe(false);
  });
});

describe("pool lookup", () => {
  it("returns a pool for a valid id", () => {
    return expect(getPoolById("eth-usdc")).resolvesToMatchObject({ id: "eth-usdc" });
  });

  it("returns null for an unknown id", () => {
    return expect(getPoolById("unknown-pool")).resolvesToBeNull();
  });

  it("returns null for a malformed id", () => {
    return expect(getPoolById("<img src=x>")).resolvesToBeNull();
  });
});

describe("sanitization", () => {
  it("strips control characters and collapses whitespace", () => {
    expect(sanitizeText("  hello\n\tworld  ", 100)).toBe("hello world");
  });

  it("truncates to the max length", () => {
    const out = sanitizeText("a".repeat(500), 20);
    expect(out.length).toBeLessThanOrEqual(20);
    expect(out.endsWith("\u2026")).toBe(true);
  });

  it("handles non-string input", () => {
    expect(sanitizeText(undefined, 10)).toBe("");
  });
});

describe("metadata builders", () => {
  it("builds a canonical url from a valid id", () => {
    expect(buildCanonicalUrl("eth-usdc", BASE)).toBe(
      "https://example.org/markets/eth-usdc",
    );
  });

  it("falls back to the markets index for invalid ids", () => {
    expect(buildCanonicalUrl("<x>", BASE)).toBe("https://example.org/markets");
  });

  it("produces fallback title and description without a pool", () => {
    expect(buildPoolTitle(null)).toBe("Market not found");
    expect(buildPoolDescription(null).length).toBeGreaterThan(0);
  });
});

describe("generatePoolMetadata", () => {
  it("resolves complete metadata for a valid pool", () => {
    return generatePoolMetadata("eth-usdc", { baseUrl: BASE }).then((meta) => {
      expect(meta.title).toContain("ETH / USDC");
      expect(meta.description).toBeTruthy();
      expect(meta.alternates?.canonical).toBe(
        "https://example.org/markets/eth-usdc",
      );
      expect(meta.openGraph?.images).toHaveLength(1);
      expect(meta.robots).toMatchObject({ index: true });
    });
  });

  it("provides safe fallbacks for an unknown pool", () => {
    return generatePoolMetadata("unknown-pool", { baseUrl: BASE }).then((meta) => {
      expect(meta.title).toBe("Market not found");
      expect(meta.robots).toMatchObject({ index: false });
      expect(meta.openGraph?.url).toBe(
        "https://example.org/markets/unknown-pool",
      );
    });
  });

  it("sanitizes malformed identifiers in metadata", () => {
    return generatePoolMetadata("<script>alert(1)</script>", { baseUrl: BASE }).then(
      (meta) => {
        expect(meta.title).toBe("Market not found");
        expect(meta.alternates?.canonical).toBe("https://example.org/markets");
        expect(JSON.stringify(meta)).not.toContain("<script>");
      },
    );
  });

  it("caps title and description lengths", () => {
    return generatePoolMetadata("eth-usdc", { baseUrl: BASE }).then((meta) => {
      expect((meta.title as string).length).toBeLessThanOrEqual(
        MAX_TITLE_LENGTH,
      );
      expect((meta.description as string).length).toBeLessThanOrEqual(
        MAX_DESCRIPTION_LENGTH,
      );
    });
  });
});
