/** @jest-environment node */
import { NextRequest } from "next/server";
import { middleware } from "../middleware";
import { contentSecurityPolicy, securityHeaders } from "../lib/securityHeaders";

function directives(policy: string) {
  return Object.fromEntries(
    policy.split("; ").map((directive) => {
      const [name, ...values] = directive.split(" ");
      return [name, values];
    }),
  );
}

describe("security headers", () => {
  afterEach(() => jest.restoreAllMocks());

  it("enforces nonce-based scripts and blocks framing, objects and inline handlers", () => {
    const policy = directives(contentSecurityPolicy("test-nonce", { production: true }));
    expect(policy["script-src"]).toEqual(["'self'", "'nonce-test-nonce'", "'strict-dynamic'"]);
    expect(policy["script-src-attr"]).toEqual(["'none'"]);
    expect(policy["frame-ancestors"]).toEqual(["'none'"]);
    expect(policy["frame-src"]).toEqual(["'none'"]);
    expect(policy["object-src"]).toEqual(["'none'"]);
    expect(policy["base-uri"]).toEqual(["'none'"]);
    expect(policy["style-src"]).toEqual(["'self'", "'nonce-test-nonce'"]);
    expect(policy["img-src"]).toEqual(["'self'", "data:", "blob:"]);
    expect(policy).toHaveProperty("upgrade-insecure-requests");
  });

  it("allows only configured API, derived WebSocket and Horizon origins", () => {
    const policy = directives(
      contentSecurityPolicy("nonce", {
        production: true,
        apiUrl: "https://api.example.com:8443/v1?unused=yes",
        horizonUrl: "https://horizon.stellar.org/accounts",
      }),
    );
    expect(policy["connect-src"]).toEqual([
      "'self'",
      "https://api.example.com:8443",
      "wss://api.example.com:8443",
      "https://horizon.stellar.org",
    ]);
  });

  it.each([
    "javascript:alert(1)",
    "https://*.example.com",
    "https://user:password@api.example.com",
    "https://api.example.com;script-src",
  ])("rejects an unsafe endpoint: %s", (apiUrl) => {
    expect(() => contentSecurityPolicy("nonce", { production: true, apiUrl })).toThrow();
  });

  it("limits eval, dev styles and HMR allowances to development", () => {
    const dev = directives(contentSecurityPolicy("nonce", { production: false }));
    const prod = directives(contentSecurityPolicy("nonce", { production: true }));
    expect(dev["script-src"]).toContain("'unsafe-eval'");
    expect(dev["connect-src"]).toContain("ws://localhost:*");
    expect(dev).not.toHaveProperty("upgrade-insecure-requests");
    expect(prod["script-src"]).not.toContain("'unsafe-eval'");
    expect(prod["connect-src"].some((source: string) => source.includes("*"))).toBe(false);
    expect(
      securityHeaders(false).some((header) => header.key === "Strict-Transport-Security"),
    ).toBe(false);
  });

  it("sets transport, framing and MIME headers without isolating wallet popups", () => {
    expect(securityHeaders(true)).toEqual(
      expect.arrayContaining([
        { key: "Strict-Transport-Security", value: "max-age=31536000" },
        { key: "X-Frame-Options", value: "DENY" },
        { key: "X-Content-Type-Options", value: "nosniff" },
        { key: "Cross-Origin-Opener-Policy", value: "same-origin-allow-popups" },
      ]),
    );
  });

  it("replaces spoofed headers, forwards the nonce to Next, and never caches it", () => {
    jest.replaceProperty(process, "env", { ...process.env, NODE_ENV: "production" });
    const request = () =>
      new NextRequest("https://predifi.app/settings", {
        headers: { "x-nonce": "attacker", "Content-Security-Policy": "script-src *" },
      });
    const first = middleware(request());
    const second = middleware(request());
    const policy = first.headers.get("Content-Security-Policy")!;
    const nonce = first.headers.get("x-middleware-request-x-nonce")!;
    expect(nonce).toMatch(/^[A-Za-z0-9+/]{22}==$/);
    expect(policy).toContain(`'nonce-${nonce}'`);
    expect(policy).not.toContain("attacker");
    expect(first.headers.get("x-middleware-request-content-security-policy")).toBe(policy);
    expect(second.headers.get("Content-Security-Policy")).not.toBe(policy);
    expect(first.headers.get("Cache-Control")).toBe("private, no-store");
  });
});
