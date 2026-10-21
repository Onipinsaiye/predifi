import assert from "node:assert/strict";
import { test } from "node:test";

// Run against `next build && next start`, never the development server.
const base = process.env.SECURITY_TEST_URL ?? "http://localhost:3000";

test("production HTML uses fresh nonces matching its scripts and styles", async () => {
  let previous;
  for (let i = 0; i < 2; i++) {
    const response = await fetch(base, {
      headers: { "x-nonce": "attacker", "Content-Security-Policy": "script-src *" },
    });
    assert.equal(response.status, 200);
    const policy = response.headers.get("content-security-policy");
    assert.ok(policy);
    const scripts = policy.split(";").find((part) => part.trim().startsWith("script-src "));
    assert.ok(scripts.includes("'strict-dynamic'"));
    assert.ok(!scripts.includes("unsafe-"));
    const nonce = scripts.match(/'nonce-([^']+)'/)?.[1];
    assert.ok(nonce && nonce !== "attacker" && nonce !== previous);
    previous = nonce;
    assert.match(response.headers.get("cache-control"), /no-store/);
    assert.equal(response.headers.get("x-frame-options"), "DENY");
    assert.equal(response.headers.get("x-content-type-options"), "nosniff");
    assert.equal(response.headers.get("strict-transport-security"), "max-age=31536000");
    const html = await response.text();
    const tags = [...html.matchAll(/<(script|style)\b[^>]*>/g)];
    assert.ok(tags.length > 0);
    for (const [tag] of tags) assert.ok(tag.includes(`nonce="${nonce}"`), tag);
    assert.doesNotMatch(html, /onload=/i);
  }
});

test("error pages receive CSP while static assets keep their cache policy", async () => {
  const missing = await fetch(`${base}/this-page-does-not-exist`);
  assert.equal(missing.status, 404);
  assert.match(missing.headers.get("content-security-policy"), /frame-ancestors 'none'/);
  const html = await (await fetch(base)).text();
  const asset = html.match(/src="([^\"]*\/_next\/static\/[^\"]+\.js)"/)?.[1];
  assert.ok(asset);
  const response = await fetch(new URL(asset, base));
  assert.equal(response.status, 200);
  assert.match(response.headers.get("cache-control"), /immutable/);
  assert.equal(response.headers.get("x-content-type-options"), "nosniff");
});

test("generated market previews remain PNG responses with security headers", async () => {
  const poolId = process.env.SECURITY_TEST_POOL_ID ?? "1";
  const response = await fetch(`${base}/api/og/pool/${encodeURIComponent(poolId)}`);
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type"), /^image\/png/);
  assert.equal(response.headers.get("x-content-type-options"), "nosniff");
  const bytes = new Uint8Array(await response.arrayBuffer());
  assert.deepEqual([...bytes.slice(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10]);
  const missing = await fetch(`${base}/api/og/pool/security-test-missing`);
  assert.notDeepEqual(
    bytes,
    new Uint8Array(await missing.arrayBuffer()),
    "Seed SECURITY_TEST_POOL_ID in the test backend; a fallback PNG does not validate a populated market preview",
  );
});

test("middleware-skipped responses cannot drop the fallback security policy", async () => {
  for (const [pathname, headers] of [
    ["/missing.json", {}],
    ["/", { "x-middleware-subrequest": Array(5).fill("middleware").join(":") }],
  ]) {
    const response = await fetch(`${base}${pathname}`, { headers });
    const policy = response.headers.get("content-security-policy");
    assert.ok(policy);
    assert.match(policy, /frame-ancestors 'none'/);
    assert.ok(policy.includes("default-src 'none'") || policy.includes("'strict-dynamic'"));
    assert.equal(response.headers.get("x-frame-options"), "DENY");
  }
});
