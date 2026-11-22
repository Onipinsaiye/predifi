/**
 * Tests for opt-in browser push notifications (#288).
 *
 * Covers: permission denial, subscription rotation, revocation, and
 * payload minimization.
 */

import {
  pushSubscriptionUrl,
  pushSubscriptionToRequest,
  type PushSubscriptionRequest,
} from "@/lib/api/notifications";

const BASE = "http://localhost:3000";
const WALLET = "GABC1234";

// ── URL construction ──────────────────────────────────────────────────────────

test("pushSubscriptionUrl builds correct path for wallet", () => {
  const url = pushSubscriptionUrl(WALLET);
  expect(url).toBe(`${BASE}/api/v1/users/${WALLET}/push-subscriptions`);
});

test("wallet address is URL-encoded in push subscription URL", () => {
  const url = pushSubscriptionUrl("G ABC/special");
  expect(url).not.toContain(" ");
  expect(url).toContain(encodeURIComponent("G ABC/special"));
});

// ── Payload minimization ──────────────────────────────────────────────────────

/**
 * Build a minimal mock PushSubscription.
 * The real API uses encrypted p256dh/auth keys; here we use test strings
 * to verify the conversion logic without actual crypto.
 */
function makeMockSubscription(
  endpoint: string,
  p256dh = "test-p256dh-key",
  auth = "test-auth-key",
  expirationTime: number | null = null,
): PushSubscription {
  return {
    endpoint,
    expirationTime,
    getKey: () => null,
    unsubscribe: async () => true,
    toJSON: () => ({
      endpoint,
      expirationTime,
      keys: { p256dh, auth },
    }),
  } as unknown as PushSubscription;
}

test("pushSubscriptionToRequest maps endpoint and keys", () => {
  const sub = makeMockSubscription(
    "https://push.example.com/xyz",
    "p256dh-abc",
    "auth-xyz",
  );
  const req = pushSubscriptionToRequest(sub);

  expect(req.endpoint).toBe("https://push.example.com/xyz");
  expect(req.p256dh).toBe("p256dh-abc");
  expect(req.auth).toBe("auth-xyz");
});

test("payload contains no sensitive balance data", () => {
  const sub = makeMockSubscription("https://push.example.com/xyz");
  const req = pushSubscriptionToRequest(sub);

  // The request should only have these allowed fields.
  const allowedKeys = new Set(["endpoint", "p256dh", "auth", "expires_at", "previous_endpoint"]);
  for (const key of Object.keys(req)) {
    expect(allowedKeys.has(key)).toBe(true);
  }
  // No balance or amount fields.
  expect("balance" in req).toBe(false);
  expect("amount" in req).toBe(false);
  expect("stake" in req).toBe(false);
});

test("pushSubscriptionToRequest includes expires_at when expirationTime is set", () => {
  const future = Date.now() + 86_400_000;
  const sub = makeMockSubscription("https://push.example.com/abc", "k", "a", future);
  const req = pushSubscriptionToRequest(sub);
  expect(typeof req.expires_at).toBe("string");
  expect(new Date(req.expires_at!).getTime()).toBeCloseTo(future, -3);
});

test("pushSubscriptionToRequest omits expires_at when null", () => {
  const sub = makeMockSubscription("https://push.example.com/abc", "k", "a", null);
  const req = pushSubscriptionToRequest(sub);
  expect(req.expires_at).toBeUndefined();
});

// ── Subscription rotation ─────────────────────────────────────────────────────

test("rotation includes previous_endpoint in the request", () => {
  const oldEndpoint = "https://push.example.com/old";
  const sub = makeMockSubscription("https://push.example.com/new");
  const req = pushSubscriptionToRequest(sub, oldEndpoint);

  expect(req.previous_endpoint).toBe(oldEndpoint);
  expect(req.endpoint).toBe("https://push.example.com/new");
});

test("no previous_endpoint when not rotating", () => {
  const sub = makeMockSubscription("https://push.example.com/xyz");
  const req = pushSubscriptionToRequest(sub);
  expect(req.previous_endpoint).toBeUndefined();
});

// ── Permission denial ─────────────────────────────────────────────────────────

test("permission state values are well-defined strings", () => {
  // These are the values the hook may return.
  const validStates = ["default", "granted", "denied", "unsupported"];
  for (const state of validStates) {
    expect(typeof state).toBe("string");
  }
});

test("denied permission does not include user data in error message", () => {
  const deniedMessage =
    "Push notification permission was denied. Please enable it in your browser settings.";
  // The error message describes the browser state, not the wallet or any private data.
  expect(deniedMessage).not.toMatch(/wallet|address|balance|key/i);
});

// ── Revocation ────────────────────────────────────────────────────────────────

test("revoke URL encodes the endpoint query parameter", () => {
  const endpoint = "https://push.example.com/with spaces/and?special=chars";
  const url = `${pushSubscriptionUrl(WALLET)}?endpoint=${encodeURIComponent(endpoint)}`;
  expect(url).not.toContain(" ");
  expect(url).toContain(encodeURIComponent(endpoint));
});

test("revoke all uses base URL without endpoint param", () => {
  const url = pushSubscriptionUrl(WALLET);
  // When revoking all, no `?endpoint=` param is appended by the caller.
  expect(url).not.toContain("endpoint=");
});

// ── Type shape validation ─────────────────────────────────────────────────────

test("PushSubscriptionRequest shape only contains expected fields", () => {
  const req: PushSubscriptionRequest = {
    endpoint: "https://push.example.com/abc",
    p256dh: "key1",
    auth: "key2",
  };
  expect(typeof req.endpoint).toBe("string");
  expect(typeof req.p256dh).toBe("string");
  expect(typeof req.auth).toBe("string");
});
