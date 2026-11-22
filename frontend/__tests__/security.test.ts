import { normalizeExternalUrl, sanitizeRichText } from "../lib/security";

describe("untrusted rich text", () => {
  it("renders markup as plain text and removes executable blocks", () => {
    expect(sanitizeRichText("<b>Hello</b><script>alert(1)</script><img src=x>world")).toBe(
      "Hello world",
    );
  });

  it("removes Unicode controls and normalizes confusable text", () => {
    expect(sanitizeRichText("pay\u202Eload\u200B now")).toBe("pay load now");
  });

  it("caps untrusted content", () => {
    expect(sanitizeRichText("a".repeat(20), 10)).toHaveLength(10);
  });
});

describe("external links", () => {
  it.each([
    "javascript:alert(1)",
    "java\u0000script:alert(1)",
    "data:text/html,<script>alert(1)</script>",
    "https://user:pass@example.com/",
  ])("rejects unsafe destination %s", (url) => {
    expect(normalizeExternalUrl(url)).toBeNull();
  });

  it("normalizes safe HTTPS URLs", () => {
    expect(normalizeExternalUrl("  HTTPS://example.com/a path  ")).toBe(
      "https://example.com/a%20path",
    );
  });
});
