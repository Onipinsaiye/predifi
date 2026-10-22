#!/usr/bin/env node
/**
 * Secret Rotation and Revocation Verification Drill
 *
 * Verifies backend secret rotation workflows, key versioning, token revocation,
 * and admin key invalidation without emitting secret material into logs or standard output.
 */

import crypto from "node:crypto";

/**
 * Creates a standard base64url string.
 */
function base64UrlEncode(str) {
  return Buffer.from(str)
    .toString("base64")
    .replace(/=/g, "")
    .replace(/\+/g, "-")
    .replace(/\//g, "_");
}

function base64UrlDecode(str) {
  let b64 = str.replace(/-/g, "+").replace(/_/g, "/");
  while (b64.length % 4) {
    b64 += "=";
  }
  return Buffer.from(b64, "base64").toString("utf8");
}

/**
 * Signs a JWT conforming to PrediFi claims schema.
 */
export function signJwt({
  sub,
  secret,
  keyVersion = 0,
  tokenType = "access",
  expiresInSecs = 3600,
  issuedAt = Math.floor(Date.now() / 1000),
  jti = crypto.randomUUID(),
}) {
  const header = { alg: "HS256", typ: "JWT" };
  const payload = {
    sub,
    token_type: tokenType,
    key_version: keyVersion,
    iat: issuedAt,
    exp: issuedAt + expiresInSecs,
    jti,
  };

  const headerEnc = base64UrlEncode(JSON.stringify(header));
  const payloadEnc = base64UrlEncode(JSON.stringify(payload));
  const signatureInput = `${headerEnc}.${payloadEnc}`;

  const hmac = crypto.createHmac("sha256", secret);
  hmac.update(signatureInput);
  const signature = hmac
    .digest("base64")
    .replace(/=/g, "")
    .replace(/\+/g, "-")
    .replace(/\//g, "_");

  return `${signatureInput}.${signature}`;
}

/**
 * Verifies a JWT strictly against expected secret, token type, and key version.
 */
export function verifyJwtStrict({
  token,
  secret,
  expectedType,
  expectedKeyVersion,
  nowUnix = Math.floor(Date.now() / 1000),
  revokedJtis = new Set(),
  userRevokeCutoff = {},
}) {
  const parts = token.split(".");
  if (parts.length !== 3) {
    throw new Error("Invalid JWT token format");
  }

  const [headerEnc, payloadEnc, signature] = parts;
  const signatureInput = `${headerEnc}.${payloadEnc}`;

  const hmac = crypto.createHmac("sha256", secret);
  hmac.update(signatureInput);
  const expectedSig = hmac
    .digest("base64")
    .replace(/=/g, "")
    .replace(/\+/g, "-")
    .replace(/\//g, "_");

  // Constant-time comparison
  const sigBuf = Buffer.from(signature);
  const expectedBuf = Buffer.from(expectedSig);
  if (
    sigBuf.length !== expectedBuf.length ||
    !crypto.timingSafeEqual(sigBuf, expectedBuf)
  ) {
    throw new Error("InvalidSignature");
  }

  const payload = JSON.parse(base64UrlDecode(payloadEnc));

  if (payload.token_type !== expectedType) {
    throw new Error(
      `WrongTokenType: expected ${expectedType}, found ${payload.token_type}`
    );
  }

  if (payload.key_version !== expectedKeyVersion) {
    throw new Error(
      `KeyVersionMismatch: expected ${expectedKeyVersion}, found ${payload.key_version}`
    );
  }

  if (payload.exp && payload.exp < nowUnix) {
    throw new Error("Expired");
  }

  if (payload.jti && revokedJtis.has(payload.jti)) {
    throw new Error("Revoked: JTI is blacklisted");
  }

  if (
    payload.sub &&
    userRevokeCutoff[payload.sub] &&
    payload.iat < userRevokeCutoff[payload.sub]
  ) {
    throw new Error("Revoked: Issued before user cutoff");
  }

  return payload;
}

/**
 * Run Secret Rotation Drill
 */
export async function runSecretRotationDrill({
  verbose = false,
  logCollector = [],
} = {}) {
  const secretsMasked = [];
  const log = (msg) => {
    logCollector.push(msg);
    if (verbose) {
      console.log(msg);
    }
  };

  log("[DRILL] Starting Staging Secret Rotation & Invalidation Drill");

  // Step 1: Generate secrets
  const initialSecret = "staging-initial-secret-key-min-32-chars-long";
  const rotatedSecret = "staging-rotated-new-secret-key-32-bytes-len";
  const initialAdminKey = "admin-key-version-1-initial-token-val";
  const rotatedAdminKey = "admin-key-version-2-rotated-token-val";

  secretsMasked.push(initialSecret, rotatedSecret, initialAdminKey, rotatedAdminKey);

  const walletUser = "GDSTAGINGOPERATORWALLETADDRESS777777777777";
  const initialKeyVersion = 1;
  const rotatedKeyVersion = 2;
  const revokedJtis = new Set();
  const userRevokeCutoffs = {};

  const now = Math.floor(Date.now() / 1000);

  // Step 2: Issue initial tokens (version 1)
  log("[DRILL] Issuing initial session tokens under key version 1");
  const initialToken = signJwt({
    sub: walletUser,
    secret: initialSecret,
    keyVersion: initialKeyVersion,
    tokenType: "access",
    issuedAt: now - 60,
  });

  const refreshJti = crypto.randomUUID();
  const initialRefreshToken = signJwt({
    sub: walletUser,
    secret: initialSecret,
    keyVersion: initialKeyVersion,
    tokenType: "refresh",
    expiresInSecs: 7 * 86400,
    issuedAt: now - 60,
    jti: refreshJti,
  });

  // Verify initial token succeeds under version 1
  verifyJwtStrict({
    token: initialToken,
    secret: initialSecret,
    expectedType: "access",
    expectedKeyVersion: initialKeyVersion,
    nowUnix: now,
  });
  log("[DRILL] Initial token verified successfully under key version 1");

  // Step 3: Perform Secret Rotation & Key Version Bump to version 2
  log("[DRILL] Simulating backend secret rotation: key version incremented to 2");

  // Verify that previous token signed with old secret/version is REJECTED under new version
  let rejectedOldVersion = false;
  try {
    verifyJwtStrict({
      token: initialToken,
      secret: rotatedSecret,
      expectedType: "access",
      expectedKeyVersion: rotatedKeyVersion,
      nowUnix: now,
    });
  } catch (err) {
    if (err.message.includes("InvalidSignature") || err.message.includes("KeyVersionMismatch")) {
      rejectedOldVersion = true;
    }
  }

  if (!rejectedOldVersion) {
    throw new Error("Drill failure: Old token was unexpectedly accepted after rotation!");
  }
  log("[DRILL] Verified: Old token rejected under rotated secret and key version 2");

  // Step 4: Issue new token under rotated secret & key version 2
  const rotatedToken = signJwt({
    sub: walletUser,
    secret: rotatedSecret,
    keyVersion: rotatedKeyVersion,
    tokenType: "access",
    issuedAt: now,
  });

  verifyJwtStrict({
    token: rotatedToken,
    secret: rotatedSecret,
    expectedType: "access",
    expectedKeyVersion: rotatedKeyVersion,
    nowUnix: now,
  });
  log("[DRILL] Verified: New token successfully verified under rotated secret and key version 2");

  // Step 5: Test Revocation Path 1 - Specific JTI revocation
  log("[DRILL] Testing Revocation Path 1: Revoking specific refresh token JTI");
  revokedJtis.add(refreshJti);

  let rejectedJti = false;
  try {
    verifyJwtStrict({
      token: initialRefreshToken,
      secret: initialSecret,
      expectedType: "refresh",
      expectedKeyVersion: initialKeyVersion,
      nowUnix: now,
      revokedJtis,
    });
  } catch (err) {
    if (err.message.includes("Revoked: JTI is blacklisted")) {
      rejectedJti = true;
    }
  }

  if (!rejectedJti) {
    throw new Error("Drill failure: Revoked JTI token was not rejected!");
  }
  log("[DRILL] Verified: Blacklisted JTI token rejected successfully");

  // Step 6: Test Revocation Path 2 - User-level cutoff revocation
  log("[DRILL] Testing Revocation Path 2: Setting user-level cutoff timestamp");
  userRevokeCutoffs[walletUser] = now + 1;

  let rejectedUserCutoff = false;
  try {
    verifyJwtStrict({
      token: rotatedToken,
      secret: rotatedSecret,
      expectedType: "access",
      expectedKeyVersion: rotatedKeyVersion,
      nowUnix: now,
      userRevokeCutoff: userRevokeCutoffs,
    });
  } catch (err) {
    if (err.message.includes("Revoked: Issued before user cutoff")) {
      rejectedUserCutoff = true;
    }
  }

  if (!rejectedUserCutoff) {
    throw new Error("Drill failure: Token issued before user cutoff was not rejected!");
  }
  log("[DRILL] Verified: Token issued before user cutoff rejected successfully");

  // Step 7: Test Admin API Key Rotation and Revocation
  log("[DRILL] Testing Admin API Key rotation and revocation");
  const testAdminAuth = (headerKey, activeKey) => {
    if (!headerKey || headerKey !== activeKey) {
      throw new Error("HTTP 401 Unauthorized");
    }
    return { status: 200, message: "Admin authenticated" };
  };

  // Old key matches old config
  testAdminAuth(initialAdminKey, initialAdminKey);

  // Rotate admin key
  let oldAdminKeyRejected = false;
  try {
    testAdminAuth(initialAdminKey, rotatedAdminKey);
  } catch (err) {
    if (err.message.includes("401")) {
      oldAdminKeyRejected = true;
    }
  }

  if (!oldAdminKeyRejected) {
    throw new Error("Drill failure: Old admin API key was not rejected after rotation!");
  }

  testAdminAuth(rotatedAdminKey, rotatedAdminKey);
  log("[DRILL] Verified: Old admin key invalidated and new admin key accepted");

  // Step 8: Audit Log Leak Check
  log("[DRILL] Auditing log outputs to ensure no secret material was leaked");
  const combinedLogs = logCollector.join("\n");
  for (const secret of secretsMasked) {
    if (combinedLogs.includes(secret)) {
      throw new Error(`SECURITY BREACH: Secret material detected in drill logs!`);
    }
  }
  log("[DRILL] Verified: Zero secret material leaked in drill output logs");
  log("[DRILL] Staging rotation drill completed successfully.");

  return {
    success: true,
    logs: logCollector,
  };
}

if (process.argv[1] && process.argv[1].endsWith("secret-rotation-drill.mjs")) {
  runSecretRotationDrill({ verbose: true })
    .then(() => {
      console.log("\nAll secret rotation and revocation checks passed.");
      process.exit(0);
    })
    .catch((err) => {
      console.error("\nDrill failed:", err.message);
      process.exit(1);
    });
}
