import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import {
  validateDeploymentManifest,
  sha256Hex,
} from '../scripts/validate-deployment-manifest.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.join(__dirname, '..');
const SCRIPT = path.join(ROOT, 'scripts', 'validate-deployment-manifest.mjs');
const MANIFEST_PATH = path.join(
  ROOT,
  'docs',
  'releases',
  'deployment-manifest.example.json'
);
const PROFILES_PATH = path.join(ROOT, 'deploy', 'cli-profiles.json');

const baseManifest = JSON.parse(fs.readFileSync(MANIFEST_PATH, 'utf8'));
const profiles = JSON.parse(fs.readFileSync(PROFILES_PATH, 'utf8'));

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function validate(manifest, overrides = {}) {
  return validateDeploymentManifest(manifest, {
    profiles,
    profileName: 'testnet',
    manifestDir: path.dirname(MANIFEST_PATH),
    ...overrides,
  });
}

function messages(result) {
  return result.errors.join('\n');
}

function runCli(args) {
  return spawnSync(process.execPath, [SCRIPT, ...args], {
    encoding: 'utf8',
    cwd: ROOT,
  });
}

describe('Deployment manifest preflight', () => {
  it('accepts a valid signed release manifest', () => {
    const result = validate(baseManifest);
    assert.equal(result.ok, true, messages(result));
    assert.ok(result.checks.includes('manifest signature verified'));
  });

  it('rejects a tampered artifact checksum', () => {
    const manifest = clone(baseManifest);
    delete manifest.signature;
    manifest.artifacts[0].sha256 = '0'.repeat(64);

    const result = validate(manifest, { allowUnsigned: true });
    assert.equal(result.ok, false);
    assert.match(messages(result), /artifact "access-control" checksum mismatch/);
  });

  it('rejects artifact bytes that do not match the declared checksum', () => {
    const manifest = clone(baseManifest);
    delete manifest.signature;
    manifest.artifacts[0].fixture_wasm_hex = '0061736d0100000000020199';

    const result = validate(manifest, { allowUnsigned: true });
    assert.equal(result.ok, false);
    assert.match(messages(result), /artifact "access-control" checksum mismatch/);
  });

  it('rejects missing required fields with field-level messages', () => {
    const manifest = clone(baseManifest);
    delete manifest.signature;
    delete manifest.network;
    delete manifest.version;
    manifest.artifacts[0].id = '';
    delete manifest.artifacts[1].filename;

    const result = validate(manifest, { allowUnsigned: true });
    assert.equal(result.ok, false);
    assert.match(messages(result), /manifest\.network is required/);
    assert.match(messages(result), /manifest\.version is required/);
    assert.match(messages(result), /manifest\.artifacts\[0\]\.id is required/);
    assert.match(messages(result), /manifest\.artifacts\[1\]\.filename is required/);
  });

  it('rejects a manifest whose network does not match the selected profile', () => {
    const manifest = clone(baseManifest);
    delete manifest.signature;
    manifest.network = 'mainnet';

    const result = validate(manifest, { allowUnsigned: true });
    assert.equal(result.ok, false);
    assert.match(
      messages(result),
      /manifest\.network "mainnet" does not match CLI profile "testnet" network "testnet"/
    );
  });

  it('rejects a manifest whose version does not match the selected profile', () => {
    const manifest = clone(baseManifest);
    delete manifest.signature;
    manifest.version = '9.9.9';

    const result = validate(manifest, { allowUnsigned: true });
    assert.equal(result.ok, false);
    assert.match(
      messages(result),
      /manifest\.version "9\.9\.9" does not match CLI profile "testnet" version "0\.1\.0-rc\.1"/
    );
  });

  it('rejects an unknown CLI profile', () => {
    const result = validate(baseManifest, { profileName: 'staging' });
    assert.equal(result.ok, false);
    assert.match(messages(result), /CLI profile "staging" is not defined/);
  });

  it('rejects a tampered signature', () => {
    const manifest = clone(baseManifest);
    manifest.manifest_id = 'tampered-after-signing';

    const result = validate(manifest);
    assert.equal(result.ok, false);
    assert.match(messages(result), /manifest signature is invalid/);
  });

  it('rejects a missing signature unless explicitly allowed', () => {
    const manifest = clone(baseManifest);
    delete manifest.signature;

    const strict = validate(manifest);
    assert.equal(strict.ok, false);
    assert.match(messages(strict), /manifest\.signature is required/);

    const allowed = validate(manifest, { allowUnsigned: true });
    assert.equal(allowed.ok, true, messages(allowed));
  });

  it('rejects contracts that reference an undeclared artifact', () => {
    const manifest = clone(baseManifest);
    delete manifest.signature;
    manifest.contracts[0].artifact_id = 'missing-artifact';

    const result = validate(manifest, { allowUnsigned: true });
    assert.equal(result.ok, false);
    assert.match(messages(result), /does not reference a declared artifact/);
  });

  it('rejects duplicate artifact ids', () => {
    const manifest = clone(baseManifest);
    delete manifest.signature;
    manifest.artifacts[1].id = manifest.artifacts[0].id;

    const result = validate(manifest, { allowUnsigned: true });
    assert.equal(result.ok, false);
    assert.match(messages(result), /is duplicated/);
  });

  it('requires exactly one artifact byte source', () => {
    const manifest = clone(baseManifest);
    delete manifest.signature;
    manifest.artifacts[0].path = 'extra.wasm';

    const result = validate(manifest, { allowUnsigned: true });
    assert.equal(result.ok, false);
    assert.match(messages(result), /exactly one of path or fixture_wasm_hex/);
  });

  it('hashes artifacts read from disk and rejects a missing file', () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'predifi-artifacts-'));
    const bytes = Buffer.from('0061736d0100000000020199', 'hex');
    fs.writeFileSync(path.join(dir, 'release.wasm'), bytes);

    const manifest = {
      format_version: 1,
      profile: 'testnet',
      network: 'testnet',
      version: '0.1.0-rc.1',
      artifacts: [
        {
          id: 'release',
          filename: 'release.wasm',
          path: 'release.wasm',
          sha256: sha256Hex(bytes),
        },
      ],
    };

    const ok = validate(manifest, { allowUnsigned: true, artifactsDir: dir });
    assert.equal(ok.ok, true, messages(ok));

    manifest.artifacts[0].path = 'missing.wasm';
    const missing = validate(manifest, { allowUnsigned: true, artifactsDir: dir });
    assert.equal(missing.ok, false);
    assert.match(messages(missing), /could not read artifact "release" bytes/);
  });
});

describe('Deployment manifest CLI', () => {
  it('exits 0 for the valid example manifest', () => {
    const proc = runCli(['--profile', 'testnet']);
    assert.equal(proc.status, 0, proc.stderr);
    assert.match(proc.stdout, /PASS: deployment manifest/);
  });

  it('exits non-zero with actionable output for a tampered manifest', () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'predifi-manifest-'));
    const tampered = clone(baseManifest);
    delete tampered.signature;
    tampered.artifacts[0].sha256 = '0'.repeat(64);
    const file = path.join(dir, 'manifest.json');
    fs.writeFileSync(file, JSON.stringify(tampered));

    const proc = runCli([
      '--manifest',
      file,
      '--profile',
      'testnet',
      '--allow-unsigned',
    ]);

    assert.equal(proc.status, 1);
    assert.match(proc.stderr, /checksum mismatch/);
    assert.match(proc.stderr, /deployment preflight error/);
  });

  it('exits non-zero when the manifest file cannot be read', () => {
    const missing = path.join(os.tmpdir(), 'predifi-does-not-exist.json');
    const proc = runCli(['--manifest', missing, '--profile', 'testnet']);
    assert.notEqual(proc.status, 0);
    assert.match(proc.stderr, /FAIL: could not read/);
  });

  it('exits 2 for unknown arguments', () => {
    const proc = runCli(['--not-a-flag']);
    assert.equal(proc.status, 2);
    assert.match(proc.stderr, /unknown argument/);
  });
});
