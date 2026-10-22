import test from "node:test";
import assert from "node:assert/strict";
import { runSecretRotationDrill, signJwt, verifyJwtStrict } from "../scripts/secret-rotation-drill.mjs";

test("staging rotation drill executes successfully with no secret material leaked", async () => {
  const logCollector = [];
  const result = await runSecretRotationDrill({ verbose: false, logCollector });
  assert.equal(result.success, true);
  assert.ok(result.logs.length > 5);

  const rawLogs = logCollector.join("\n");
  assert.ok(!rawLogs.includes("staging-initial-secret-key"));
  assert.ok(!rawLogs.includes("staging-rotated-new-secret-key"));
  assert.ok(!rawLogs.includes("admin-key-version-1"));
  assert.ok(!rawLogs.includes("admin-key-version-2"));
});

test("strict JWT verification rejects wrong key version", () => {
  const secret = "test-secret-at-least-thirty-two-bytes-long";
  const token = signJwt({
    sub: "GTESTWALLET123456789",
    secret,
    keyVersion: 1,
    tokenType: "access",
  });

  assert.throws(
    () => {
      verifyJwtStrict({
        token,
        secret,
        expectedType: "access",
        expectedKeyVersion: 2, // Mismatch
      });
    },
    { message: /KeyVersionMismatch/ }
  );
});

test("revocation path rejects blacklisted JTI", () => {
  const secret = "test-secret-at-least-thirty-two-bytes-long";
  const jti = "revoked-jti-uuid";
  const token = signJwt({
    sub: "GTESTWALLET123456789",
    secret,
    keyVersion: 1,
    tokenType: "refresh",
    jti,
  });

  const revokedJtis = new Set([jti]);

  assert.throws(
    () => {
      verifyJwtStrict({
        token,
        secret,
        expectedType: "refresh",
        expectedKeyVersion: 1,
        revokedJtis,
      });
    },
    { message: /Revoked: JTI is blacklisted/ }
  );
});

test("revocation path rejects tokens issued before user cutoff timestamp", () => {
  const secret = "test-secret-at-least-thirty-two-bytes-long";
  const now = Math.floor(Date.now() / 1000);
  const user = "GTESTWALLET123456789";

  const token = signJwt({
    sub: user,
    secret,
    keyVersion: 1,
    tokenType: "access",
    issuedAt: now - 100,
  });

  const userRevokeCutoff = {
    [user]: now - 10,
  };

  assert.throws(
    () => {
      verifyJwtStrict({
        token,
        secret,
        expectedType: "access",
        expectedKeyVersion: 1,
        nowUnix: now,
        userRevokeCutoff,
      });
    },
    { message: /Revoked: Issued before user cutoff/ }
  );
});
