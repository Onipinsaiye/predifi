import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { validateRelease } from '../scripts/validate-release.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.join(__dirname, '..');
const examplePath = path.join(ROOT, 'docs', 'releases', 'contract-release.example.json');
const baseExample = JSON.parse(fs.readFileSync(examplePath, 'utf8'));

describe('Release metadata and build provenance validation', () => {
  it('validates a correct synthetic test release with toolchain and workflow provenance', () => {
    assert.doesNotThrow(() => {
      validateRelease(baseExample);
    });
  });

  it('rejects release if commit SHA does not match 40-char hex format', () => {
    const invalid = JSON.parse(JSON.stringify(baseExample));
    invalid.source.commit = 'invalid-sha';
    assert.throws(
      () => validateRelease(invalid),
      /invalid source SHA/
    );
  });

  it('rejects release if build workflow ID is missing when workflow block is present', () => {
    const invalid = JSON.parse(JSON.stringify(baseExample));
    delete invalid.build.workflow.id;
    assert.throws(
      () => validateRelease(invalid),
      /missing build workflow id/
    );
  });

  it('rejects release if build workflow run_id is missing', () => {
    const invalid = JSON.parse(JSON.stringify(baseExample));
    delete invalid.build.workflow.run_id;
    assert.throws(
      () => validateRelease(invalid),
      /missing build workflow run_id/
    );
  });

  it('rejects release if toolchain metadata is missing', () => {
    const invalid = JSON.parse(JSON.stringify(baseExample));
    invalid.build.toolchain = null;
    assert.throws(
      () => validateRelease(invalid),
      /missing build toolchain/
    );
  });

  it('rejects release if artifact digest does not match fixture bytes', () => {
    const invalid = JSON.parse(JSON.stringify(baseExample));
    invalid.artifacts[0].sha256 = '0'.repeat(64);
    assert.throws(
      () => validateRelease(invalid),
      /artifact digest mismatch/
    );
  });

  it('rejects release if client supported versions do not match release version', () => {
    const invalid = JSON.parse(JSON.stringify(baseExample));
    invalid.clients[0].supported_contract_versions = ['9.9.9'];
    assert.throws(
      () => validateRelease(invalid),
      /unsupported client version/
    );
  });
});
