#!/usr/bin/env node
/**
 * Standalone offline release metadata validator for PrediFi release manifests.
 *
 * Runs in clean environments using standard Node.js without third-party dependencies.
 * Verifies machine-readable build provenance, toolchain, workflow identity,
 * artifact digests, schemas, deployments, and client compatibility.
 *
 * Usage:
 *   node scripts/validate-release.mjs [path/to/release.json]
 */

import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.join(__dirname, '..');

export function requireCond(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

export function validateRelease(example) {
  requireCond(example.example_only === true, 'synthetic example marker required');
  requireCond(example.format_version === 1, 'unsupported manifest format');

  const version = example.version;
  requireCond(version === '0.1.0-rc.1', 'unexpected fixture release version');

  const source = example.source;
  requireCond(source != null, 'missing source');
  requireCond(source.tag === `contracts-v${version}`, 'tag/version mismatch');
  requireCond(/^[0-9a-f]{40}$/.test(source.commit), 'invalid source SHA');

  // Verify build provenance & toolchain / workflow identity binding
  if (example.build) {
    const build = example.build;
    if (build.workflow) {
      requireCond(build.workflow.id != null, 'missing build workflow id');
      requireCond(build.workflow.run_id != null, 'missing build workflow run_id');
      requireCond(build.toolchain != null, 'missing build toolchain');
    }
  }

  const artifacts = {};
  for (const item of example.artifacts) {
    requireCond(!artifacts[item.id], 'duplicate artifact ID');
    artifacts[item.id] = item;
  }
  const artifactIds = Object.keys(artifacts).sort();
  requireCond(
    JSON.stringify(artifactIds) === JSON.stringify(['access-control', 'predifi-contract']),
    'missing artifact'
  );

  for (const artifact of Object.values(artifacts)) {
    const hash = crypto
      .createHash('sha256')
      .update(Buffer.from(artifact.fixture_wasm_hex, 'hex'))
      .digest('hex');
    requireCond(hash === artifact.sha256, 'artifact digest mismatch');
    requireCond(hash === artifact.uploaded_wasm_hash, 'upload hash mismatch');
    requireCond(artifact.storage_schema_version === 1, 'unexpected fixture schema');
    requireCond(
      JSON.stringify(artifact.readable_storage_schemas) === JSON.stringify([1]),
      'unknown readable schema'
    );
  }

  const seen = new Set();
  const covered = new Set();
  for (const record of example.deployments) {
    requireCond(!seen.has(record.record_id), 'duplicate deployment record');
    seen.add(record.record_id);
    requireCond(artifacts[record.artifact_id] != null, 'unknown deployment artifact');

    const artifact = artifacts[record.artifact_id];
    covered.add(record.artifact_id);

    requireCond(record.release_tag === source.tag, 'deployment tag mismatch');
    requireCond(record.source_commit === source.commit, 'deployment source mismatch');
    requireCond(record.observed_wasm_hash === artifact.sha256, 'deployment hash mismatch');
    requireCond(
      record.storage_schema_version === artifact.storage_schema_version,
      'deployment schema mismatch'
    );
    requireCond(record.status === 'synthetic-confirmed', 'fixture must not claim live status');
    requireCond(
      record.network_passphrase === 'PrediFi offline fixture network; NOT a live network',
      'wrong fixture network'
    );
    requireCond(/^C[A-Z2-7]{55}$/.test(record.contract_address), 'address shape');
    requireCond(/^[0-9a-f]{64}$/.test(record.transaction_hash), 'transaction shape');
  }

  requireCond(covered.size === Object.keys(artifacts).length, 'artifact has no deployment record');

  for (const client of example.clients) {
    requireCond(client.tested_contract_version === version, 'untested client version');
    requireCond(
      JSON.stringify(client.supported_contract_versions) === JSON.stringify([version]),
      'unsupported client version'
    );
  }
}

export function runValidationSuite(targetFile) {
  const filePath = targetFile || path.join(ROOT, 'docs', 'releases', 'contract-release.example.json');
  const example = JSON.parse(fs.readFileSync(filePath, 'utf8'));

  validateRelease(example);
  console.log('PASS: synthetic tag/source/artifact/schema/deployment/client mapping');

  const cases = [
    ['artifacts', 0, 'sha256', '0'.repeat(64), 'artifact digest mismatch'],
    ['deployments', 0, 'artifact_id', 'unknown', 'unknown deployment artifact'],
    ['deployments', 0, 'storage_schema_version', 2, 'deployment schema mismatch'],
    ['deployments', 0, 'source_commit', '2'.repeat(40), 'deployment source mismatch'],
    ['clients', 0, 'supported_contract_versions', ['0.2.0'], 'unsupported client version'],
  ];

  for (const [section, index, key, value, expected] of cases) {
    const changed = JSON.parse(JSON.stringify(example));
    changed[section][index][key] = value;
    try {
      validateRelease(changed);
    } catch (err) {
      requireCond(err.message === expected, `unexpected rejection: ${err.message}`);
      console.log(`PASS: rejected ${expected}`);
    }
  }

  // Negative test case: missing build toolchain
  const changedToolchain = JSON.parse(JSON.stringify(example));
  changedToolchain.build.toolchain = null;
  try {
    validateRelease(changedToolchain);
  } catch (err) {
    requireCond(err.message === 'missing build toolchain', `unexpected rejection: ${err.message}`);
    console.log('PASS: rejected missing build toolchain');
  }

  console.log('No WASM executed and no network deployment performed.');
}

if (process.argv[1] && process.argv[1].endsWith('validate-release.mjs')) {
  try {
    const target = process.argv[2];
    runValidationSuite(target);
  } catch (err) {
    console.error(`FAIL: ${err.message}`);
    process.exit(1);
  }
}
