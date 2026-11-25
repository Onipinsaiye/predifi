#!/usr/bin/env node
/**
 * Offline preflight validator for PrediFi contract deployment manifests.
 *
 * Runs in clean environments using standard Node.js without third-party
 * dependencies. It checks a deployment manifest *before* any contract is
 * uploaded or deployed:
 *
 *   - required manifest fields are present and well formed;
 *   - every artifact's declared SHA-256 matches the bytes that will be
 *     deployed (either a file on disk or an inline `fixture_wasm_hex` fixture);
 *   - contract entries reference a declared artifact and carry a contract ID;
 *   - the manifest's `network` and `version` match the selected CLI profile;
 *   - the manifest's optional Ed25519 signature covers the manifest body.
 *
 * Every problem is reported with the failing field path and a non-zero exit
 * code, so CI and operators get an actionable failure instead of a partial
 * deployment.
 *
 * Usage:
 *   node scripts/validate-deployment-manifest.mjs \
 *     --manifest docs/releases/deployment-manifest.example.json \
 *     --profile testnet \
 *     [--profiles deploy/cli-profiles.json] \
 *     [--artifacts-dir <dir>] \
 *     [--allow-unsigned]
 */

import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.join(__dirname, '..');

export const DEFAULT_PROFILES_PATH = path.join(ROOT, 'deploy', 'cli-profiles.json');
export const DEFAULT_MANIFEST_PATH = path.join(
  ROOT,
  'docs',
  'releases',
  'deployment-manifest.example.json'
);

const SHA256_RE = /^[0-9a-f]{64}$/;
const EVEN_HEX_RE = /^(?:[0-9a-fA-F]{2})+$/;

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

/**
 * Deterministic JSON canonicalisation: object keys are sorted recursively so a
 * signature is stable regardless of how the manifest is formatted on disk.
 */
export function canonicalize(value) {
  if (Array.isArray(value)) {
    return value.map(canonicalize);
  }
  if (isPlainObject(value)) {
    const sorted = {};
    for (const key of Object.keys(value).sort()) {
      sorted[key] = canonicalize(value[key]);
    }
    return sorted;
  }
  return value;
}

/** The manifest body that a signature covers (everything except `signature`). */
export function manifestBody(manifest) {
  const { signature, ...body } = manifest;
  return body;
}

export function canonicalManifestBody(manifest) {
  return JSON.stringify(canonicalize(manifestBody(manifest)));
}

export function sha256Hex(bytes) {
  return crypto.createHash('sha256').update(bytes).digest('hex');
}

/**
 * Validate a deployment manifest against a selected CLI profile.
 *
 * Returns `{ ok, errors, checks }`. It never throws for manifest problems, so
 * callers can report every failure at once.
 */
export function validateDeploymentManifest(manifest, options = {}) {
  const {
    profiles,
    profileName,
    manifestDir = ROOT,
    artifactsDir = manifestDir,
    allowUnsigned = false,
    readFile = fs.readFileSync,
  } = options;

  const errors = [];
  const checks = [];
  const fail = (message) => errors.push(message);
  const pass = (message) => checks.push(message);

  if (!isPlainObject(manifest)) {
    fail('manifest must be a JSON object');
    return { ok: false, errors, checks };
  }

  if (manifest.format_version !== 1) {
    fail('manifest.format_version must be 1');
  } else {
    pass('format_version is 1');
  }

  // ── CLI profile: declared network and version must match ──────────────────
  if (!isPlainObject(profiles)) {
    fail('CLI profiles could not be loaded');
  } else {
    const profile =
      isPlainObject(profiles.profiles) && profileName != null
        ? profiles.profiles[profileName]
        : undefined;

    if (profileName == null || profileName === '') {
      fail('no CLI profile selected (use --profile or set manifest.profile)');
    } else if (!isPlainObject(profile)) {
      fail(`CLI profile "${profileName}" is not defined`);
    } else {
      if (typeof profile.network !== 'string' || profile.network.trim() === '') {
        fail(`CLI profile "${profileName}" is missing a network`);
      } else if (typeof manifest.network !== 'string' || manifest.network.trim() === '') {
        fail('manifest.network is required');
      } else if (manifest.network.toLowerCase() !== profile.network.toLowerCase()) {
        fail(
          `manifest.network "${manifest.network}" does not match CLI profile ` +
            `"${profileName}" network "${profile.network}"`
        );
      } else {
        pass(`network matches CLI profile "${profileName}"`);
      }

      if (typeof profile.version !== 'string' || profile.version.trim() === '') {
        fail(`CLI profile "${profileName}" is missing a version`);
      } else if (typeof manifest.version !== 'string' || manifest.version.trim() === '') {
        fail('manifest.version is required');
      } else if (manifest.version !== profile.version) {
        fail(
          `manifest.version "${manifest.version}" does not match CLI profile ` +
            `"${profileName}" version "${profile.version}"`
        );
      } else {
        pass(`version matches CLI profile "${profileName}"`);
      }

      if (
        typeof manifest.network_passphrase === 'string' &&
        typeof profile.network_passphrase === 'string' &&
        manifest.network_passphrase !== profile.network_passphrase
      ) {
        fail(
          `manifest.network_passphrase does not match CLI profile "${profileName}"`
        );
      }
    }
  }

  if (manifest.profile != null && manifest.profile !== profileName) {
    fail(
      `manifest.profile "${manifest.profile}" does not match selected profile ` +
        `"${profileName}"`
    );
  }

  // ── Artifacts and their checksums ────────────────────────────────────────
  const artifactIds = new Set();
  if (!Array.isArray(manifest.artifacts) || manifest.artifacts.length === 0) {
    fail('manifest.artifacts must be a non-empty array');
  } else {
    manifest.artifacts.forEach((artifact, index) => {
      const at = `manifest.artifacts[${index}]`;
      if (!isPlainObject(artifact)) {
        fail(`${at} must be an object`);
        return;
      }

      let id = `<index ${index}>`;
      if (typeof artifact.id !== 'string' || artifact.id.trim() === '') {
        fail(`${at}.id is required`);
      } else if (artifactIds.has(artifact.id)) {
        fail(`${at}.id "${artifact.id}" is duplicated`);
      } else {
        artifactIds.add(artifact.id);
        id = artifact.id;
      }

      if (typeof artifact.filename !== 'string' || artifact.filename.trim() === '') {
        fail(`${at}.filename is required`);
      }

      if (typeof artifact.sha256 !== 'string' || !SHA256_RE.test(artifact.sha256)) {
        fail(`${at}.sha256 must be a 64-character lowercase hex digest`);
        return;
      }

      const hasPath = typeof artifact.path === 'string' && artifact.path.trim() !== '';
      const hasHex =
        typeof artifact.fixture_wasm_hex === 'string' &&
        artifact.fixture_wasm_hex.trim() !== '';

      if (hasPath === hasHex) {
        fail(`${at} must provide exactly one of path or fixture_wasm_hex`);
        return;
      }

      let bytes;
      let source;
      try {
        if (hasPath) {
          source = path.isAbsolute(artifact.path)
            ? artifact.path
            : path.join(artifactsDir, artifact.path);
          bytes = readFile(source);
        } else {
          if (!EVEN_HEX_RE.test(artifact.fixture_wasm_hex)) {
            fail(`${at}.fixture_wasm_hex must be an even-length hex string`);
            return;
          }
          source = `${at}.fixture_wasm_hex`;
          bytes = Buffer.from(artifact.fixture_wasm_hex, 'hex');
        }
      } catch (err) {
        fail(`could not read artifact "${id}" bytes from ${source}: ${err.message}`);
        return;
      }

      const computed = sha256Hex(bytes);
      if (computed !== artifact.sha256.toLowerCase()) {
        fail(
          `artifact "${id}" checksum mismatch: manifest declares ` +
            `${artifact.sha256}, computed ${computed} from ${source}`
        );
      } else {
        pass(`artifact "${id}" checksum verified`);
      }
    });
  }

  // ── Contract entries (optional) ──────────────────────────────────────────
  const contractIds = new Set();
  if (manifest.contracts != null) {
    if (!Array.isArray(manifest.contracts)) {
      fail('manifest.contracts must be an array when present');
    } else {
      manifest.contracts.forEach((contract, index) => {
        const at = `manifest.contracts[${index}]`;
        if (!isPlainObject(contract)) {
          fail(`${at} must be an object`);
          return;
        }
        if (typeof contract.id !== 'string' || contract.id.trim() === '') {
          fail(`${at}.id is required`);
        } else if (contractIds.has(contract.id)) {
          fail(`${at}.id "${contract.id}" is duplicated`);
        } else {
          contractIds.add(contract.id);
        }
        if (
          typeof contract.artifact_id !== 'string' ||
          !artifactIds.has(contract.artifact_id)
        ) {
          fail(
            `${at}.artifact_id "${contract.artifact_id}" does not reference a ` +
              'declared artifact'
          );
        }
        if (
          typeof contract.contract_id !== 'string' ||
          contract.contract_id.trim() === ''
        ) {
          fail(`${at}.contract_id is required`);
        }
      });
    }
  }

  // ── Signature ────────────────────────────────────────────────────────────
  if (manifest.signature == null) {
    if (allowUnsigned) {
      pass('manifest is unsigned (allowed by --allow-unsigned)');
    } else {
      fail('manifest.signature is required (pass --allow-unsigned to skip)');
    }
  } else if (!isPlainObject(manifest.signature)) {
    fail('manifest.signature must be an object');
  } else {
    const signature = manifest.signature;
    if (signature.algorithm !== 'ed25519') {
      fail('manifest.signature.algorithm must be "ed25519"');
    } else if (
      typeof signature.public_key !== 'string' ||
      signature.public_key.trim() === ''
    ) {
      fail('manifest.signature.public_key is required');
    } else if (
      typeof signature.value !== 'string' ||
      signature.value.trim() === ''
    ) {
      fail('manifest.signature.value is required');
    } else {
      try {
        const publicKey = crypto.createPublicKey({
          key: Buffer.from(signature.public_key, 'base64'),
          format: 'der',
          type: 'spki',
        });
        const valid = crypto.verify(
          null,
          Buffer.from(canonicalManifestBody(manifest), 'utf8'),
          publicKey,
          Buffer.from(signature.value, 'base64')
        );
        if (!valid) {
          fail('manifest signature is invalid for the manifest contents');
        } else {
          pass('manifest signature verified');
        }
      } catch (err) {
        fail(`manifest signature could not be verified: ${err.message}`);
      }
    }
  }

  return { ok: errors.length === 0, errors, checks };
}

export function parseArgs(argv) {
  const args = { allowUnsigned: false };
  for (let i = 0; i < argv.length; i += 1) {
    const token = argv[i];
    const [flag, inline] = token.includes('=')
      ? token.split(/=(.*)/s, 2)
      : [token, undefined];
    const takeValue = () => {
      if (inline !== undefined) return inline;
      const next = argv[i + 1];
      if (next === undefined || next.startsWith('--')) {
        throw new Error(`${flag} requires a value`);
      }
      i += 1;
      return next;
    };

    switch (flag) {
      case '--manifest':
        args.manifest = takeValue();
        break;
      case '--profile':
        args.profile = takeValue();
        break;
      case '--profiles':
        args.profiles = takeValue();
        break;
      case '--artifacts-dir':
        args.artifactsDir = takeValue();
        break;
      case '--allow-unsigned':
        args.allowUnsigned = true;
        break;
      case '--help':
      case '-h':
        args.help = true;
        break;
      default:
        throw new Error(`unknown argument: ${token}`);
    }
  }
  return args;
}

function loadJson(file) {
  let text;
  try {
    text = fs.readFileSync(file, 'utf8');
  } catch (err) {
    throw new Error(`could not read ${file}: ${err.message}`);
  }
  try {
    return JSON.parse(text);
  } catch (err) {
    throw new Error(`could not parse JSON in ${file}: ${err.message}`);
  }
}

function usage() {
  return [
    'Usage: node scripts/validate-deployment-manifest.mjs [options]',
    '',
    'Options:',
    '  --manifest <path>       Deployment manifest (default: example fixture)',
    '  --profile <name>        CLI profile to validate against',
    '  --profiles <path>       CLI profiles file (default: deploy/cli-profiles.json)',
    '  --artifacts-dir <path>  Base directory for relative artifact paths',
    '  --allow-unsigned        Accept a manifest without a signature',
    '  --help                  Show this message',
  ].join('\n');
}

export function runCli(argv = process.argv.slice(2)) {
  let args;
  try {
    args = parseArgs(argv);
  } catch (err) {
    console.error(`FAIL: ${err.message}`);
    console.error(usage());
    return 2;
  }

  if (args.help) {
    console.log(usage());
    return 0;
  }

  const manifestPath = path.resolve(args.manifest ?? DEFAULT_MANIFEST_PATH);
  const profilesPath = path.resolve(args.profiles ?? DEFAULT_PROFILES_PATH);

  let manifest;
  let profiles;
  try {
    manifest = loadJson(manifestPath);
    profiles = loadJson(profilesPath);
  } catch (err) {
    console.error(`FAIL: ${err.message}`);
    return 2;
  }

  const profileName = args.profile ?? manifest.profile;
  const result = validateDeploymentManifest(manifest, {
    profiles,
    profileName,
    manifestDir: path.dirname(manifestPath),
    artifactsDir: args.artifactsDir
      ? path.resolve(args.artifactsDir)
      : path.dirname(manifestPath),
    allowUnsigned: args.allowUnsigned,
  });

  for (const check of result.checks) {
    console.log(`ok: ${check}`);
  }
  for (const error of result.errors) {
    console.error(`FAIL: ${error}`);
  }

  if (!result.ok) {
    console.error(
      `FAIL: ${result.errors.length} deployment preflight error(s) for ${manifestPath}`
    );
    return 1;
  }

  console.log(
    `PASS: deployment manifest ${manifestPath} is valid for profile "${profileName}"`
  );
  return 0;
}

if (process.argv[1] && process.argv[1].endsWith('validate-deployment-manifest.mjs')) {
  process.exitCode = runCli();
}
