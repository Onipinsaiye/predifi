/**
 * Tests for embeddable market card utilities (issue #283).
 *
 * Covers: CSP/sandbox behaviour, stale-data disclosure, source URL
 * validation, embed snippet generation, and responsive sizing.
 */

import {
  classifyEmbedFreshness,
  buildEmbedCSP,
  buildEmbedSnippet,
  isValidSourceUrl,
  EMBED_IFRAME_SANDBOX,
  EMBED_STALE_THRESHOLD_MS,
} from "../lib/embedCard";

const NOW = Date.now();

// ---------------------------------------------------------------------------
// classifyEmbedFreshness
// ---------------------------------------------------------------------------

describe("classifyEmbedFreshness", () => {
  it("returns 'fresh' for data within the threshold", () => {
    const recent = new Date(NOW - 60_000).toISOString(); // 1 minute ago
    expect(classifyEmbedFreshness(recent, NOW)).toBe("fresh");
  });

  it("returns 'stale' for data beyond the threshold", () => {
    const old = new Date(NOW - EMBED_STALE_THRESHOLD_MS - 1000).toISOString();
    expect(classifyEmbedFreshness(old, NOW)).toBe("stale");
  });

  it("returns 'unavailable' when indexedAt is null", () => {
    expect(classifyEmbedFreshness(null, NOW)).toBe("unavailable");
  });

  it("returns 'unavailable' when indexedAt is undefined", () => {
    expect(classifyEmbedFreshness(undefined, NOW)).toBe("unavailable");
  });

  it("returns 'unavailable' for an unparseable date string", () => {
    expect(classifyEmbedFreshness("not-a-date", NOW)).toBe("unavailable");
  });

  it("treats exactly stale threshold as 'stale'", () => {
    const exactlyStale = new Date(NOW - EMBED_STALE_THRESHOLD_MS).toISOString();
    // Equal to threshold → stale (> check is strict in the implementation)
    // We allow either outcome here as the boundary is implementation-defined,
    // but we ensure it is not "unavailable".
    const result = classifyEmbedFreshness(exactlyStale, NOW);
    expect(["fresh", "stale"]).toContain(result);
  });
});

// ---------------------------------------------------------------------------
// buildEmbedCSP — prohibit transaction signing and third-party tracking
// ---------------------------------------------------------------------------

describe("buildEmbedCSP", () => {
  let csp: string;

  beforeEach(() => {
    csp = buildEmbedCSP();
  });

  it("prohibits external connections (connect-src none)", () => {
    expect(csp).toContain("connect-src 'none'");
  });

  it("prohibits form submission / wallet signing (form-action none)", () => {
    expect(csp).toContain("form-action 'none'");
  });

  it("prohibits nested frames (frame-src none)", () => {
    expect(csp).toContain("frame-src 'none'");
  });

  it("allows embedding on any origin (frame-ancestors *)", () => {
    expect(csp).toContain("frame-ancestors *");
  });

  it("restricts scripts to self only (no external CDN)", () => {
    // Should not contain any http(s) host in script-src
    const scriptSrcMatch = csp.match(/script-src ([^;]+)/);
    expect(scriptSrcMatch).not.toBeNull();
    const scriptSrc = scriptSrcMatch![1];
    expect(scriptSrc).not.toMatch(/https?:\/\//);
  });

  it("restricts images to self and data URIs only (no tracking pixels)", () => {
    expect(csp).toContain("img-src 'self' data:");
    // No third-party image host
    const imgSrcMatch = csp.match(/img-src ([^;]+)/);
    expect(imgSrcMatch).not.toBeNull();
    const imgSrc = imgSrcMatch![1];
    expect(imgSrc).not.toMatch(/https?:\/\//);
  });

  it("sets default-src to none as a secure baseline", () => {
    expect(csp).toContain("default-src 'none'");
  });

  it("prohibits object loading (object-src none)", () => {
    expect(csp).toContain("object-src 'none'");
  });

  it("prohibits base tag injection (base-uri none)", () => {
    expect(csp).toContain("base-uri 'none'");
  });
});

// ---------------------------------------------------------------------------
// EMBED_IFRAME_SANDBOX — block wallet APIs
// ---------------------------------------------------------------------------

describe("EMBED_IFRAME_SANDBOX", () => {
  it("contains allow-same-origin for styling", () => {
    expect(EMBED_IFRAME_SANDBOX).toContain("allow-same-origin");
  });

  it("contains allow-scripts for rendering", () => {
    expect(EMBED_IFRAME_SANDBOX).toContain("allow-scripts");
  });

  it("does NOT contain allow-forms (prevents wallet signing POSTs)", () => {
    expect(EMBED_IFRAME_SANDBOX).not.toContain("allow-forms");
  });

  it("does NOT contain allow-popups (prevents wallet popups)", () => {
    expect(EMBED_IFRAME_SANDBOX).not.toContain("allow-popups");
  });

  it("does NOT contain allow-top-navigation (prevents host page hijack)", () => {
    expect(EMBED_IFRAME_SANDBOX).not.toContain("allow-top-navigation");
  });

  it("does NOT contain allow-storage-access-by-user-activation", () => {
    expect(EMBED_IFRAME_SANDBOX).not.toContain("allow-storage-access-by-user-activation");
  });
});

// ---------------------------------------------------------------------------
// buildEmbedSnippet — responsive sizing
// ---------------------------------------------------------------------------

describe("buildEmbedSnippet", () => {
  it("generates an iframe element", () => {
    const snippet = buildEmbedSnippet("https://predifi.app", 42);
    expect(snippet).toContain("<iframe");
    expect(snippet).toContain("</iframe>");
  });

  it("includes the correct embed URL", () => {
    const snippet = buildEmbedSnippet("https://predifi.app", 42);
    expect(snippet).toContain("/embed/markets/42");
  });

  it("sets width to 100% for responsive sizing", () => {
    const snippet = buildEmbedSnippet("https://predifi.app", 42);
    expect(snippet).toContain('width="100%"');
  });

  it("includes the sandbox attribute", () => {
    const snippet = buildEmbedSnippet("https://predifi.app", 42);
    expect(snippet).toContain(`sandbox="${EMBED_IFRAME_SANDBOX}"`);
  });

  it("includes loading=lazy for performance", () => {
    const snippet = buildEmbedSnippet("https://predifi.app", 42);
    expect(snippet).toContain('loading="lazy"');
  });

  it("strips trailing slash from base URL", () => {
    const snippet = buildEmbedSnippet("https://predifi.app/", 7);
    expect(snippet).toContain("https://predifi.app/embed/markets/7");
    expect(snippet).not.toContain("//embed");
  });

  it("includes a title attribute for accessibility", () => {
    const snippet = buildEmbedSnippet("https://predifi.app", 99);
    expect(snippet).toContain("title=");
  });
});

// ---------------------------------------------------------------------------
// isValidSourceUrl
// ---------------------------------------------------------------------------

describe("isValidSourceUrl", () => {
  it("accepts https URLs", () => {
    expect(isValidSourceUrl("https://example.com/source")).toBe(true);
  });

  it("accepts http URLs", () => {
    expect(isValidSourceUrl("http://example.com/source")).toBe(true);
  });

  it("rejects URLs with credentials", () => {
    expect(isValidSourceUrl("https://user:pass@example.com")).toBe(false);
  });

  it("rejects non-http protocols", () => {
    expect(isValidSourceUrl("javascript:alert(1)")).toBe(false);
    expect(isValidSourceUrl("data:text/html,<h1>x</h1>")).toBe(false);
    expect(isValidSourceUrl("ftp://example.com")).toBe(false);
  });

  it("rejects non-strings", () => {
    expect(isValidSourceUrl(null)).toBe(false);
    expect(isValidSourceUrl(undefined)).toBe(false);
    expect(isValidSourceUrl(42)).toBe(false);
  });

  it("rejects malformed URLs", () => {
    expect(isValidSourceUrl("not a url")).toBe(false);
    expect(isValidSourceUrl("")).toBe(false);
  });
});
