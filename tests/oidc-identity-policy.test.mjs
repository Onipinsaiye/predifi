import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { evaluateOidcTrustPolicy, DEFAULT_TRUST_POLICY } from '../scripts/oidc-policy-evaluator.mjs';

describe('OIDC Identity Policy and Negative Security Tests', () => {
  const validStagingClaims = {
    repository: 'Daluworks/predifi',
    ref: 'refs/heads/main',
    event_name: 'push',
    sub: 'repo:Daluworks/predifi:ref:refs/heads/main',
    aud: 'sts.amazonaws.com',
  };

  it('allows valid GitHub OIDC token from main branch push on trusted repository', () => {
    const audit = evaluateOidcTrustPolicy({
      policy: DEFAULT_TRUST_POLICY,
      claims: validStagingClaims,
      requestedDurationSeconds: 900,
    });

    assert.equal(audit.decision, 'allow');
    assert.equal(audit.reasons.length, 0);
  });

  it('negative: rejects pull request event attempts to assume deploy role', () => {
    const prClaims = {
      ...validStagingClaims,
      event_name: 'pull_request',
      ref: 'refs/pull/42/merge',
      sub: 'repo:Daluworks/predifi:ref:refs/pull/42/merge',
    };

    const audit = evaluateOidcTrustPolicy({
      policy: DEFAULT_TRUST_POLICY,
      claims: prClaims,
      requestedDurationSeconds: 900,
    });

    assert.equal(audit.decision, 'deny');
    assert.ok(
      audit.reasons.some((r) => r.includes('untrusted pull-request workflow')),
      'Should reject pull request workflow',
    );
  });

  it('negative: rejects untrusted feature branch', () => {
    const featureBranchClaims = {
      ...validStagingClaims,
      ref: 'refs/heads/feature/untrusted-patch',
      sub: 'repo:Daluworks/predifi:ref:refs/heads/feature/untrusted-patch',
    };

    const audit = evaluateOidcTrustPolicy({
      policy: DEFAULT_TRUST_POLICY,
      claims: featureBranchClaims,
      requestedDurationSeconds: 900,
    });

    assert.equal(audit.decision, 'deny');
    assert.ok(
      audit.reasons.some((r) => r.includes('untrusted ref')),
      'Should reject non-main branch',
    );
  });

  it('negative: rejects external forked or malicious repository', () => {
    const attackerClaims = {
      repository: 'malicious-attacker/predifi',
      ref: 'refs/heads/main',
      event_name: 'push',
      sub: 'repo:malicious-attacker/predifi:ref:refs/heads/main',
      aud: 'sts.amazonaws.com',
    };

    const audit = evaluateOidcTrustPolicy({
      policy: DEFAULT_TRUST_POLICY,
      claims: attackerClaims,
      requestedDurationSeconds: 900,
    });

    assert.equal(audit.decision, 'deny');
    assert.ok(
      audit.reasons.some((r) => r.includes('untrusted repository')),
      'Should reject foreign repository',
    );
  });

  it('negative: rejects invalid audience', () => {
    const wrongAudienceClaims = {
      ...validStagingClaims,
      aud: 'https://evil-sts.net',
    };

    const audit = evaluateOidcTrustPolicy({
      policy: DEFAULT_TRUST_POLICY,
      claims: wrongAudienceClaims,
      requestedDurationSeconds: 900,
    });

    assert.equal(audit.decision, 'deny');
    assert.ok(
      audit.reasons.some((r) => r.includes('invalid audience')),
      'Should reject wrong audience',
    );
  });

  it('negative: rejects session duration exceeding maximum boundary', () => {
    const audit = evaluateOidcTrustPolicy({
      policy: DEFAULT_TRUST_POLICY,
      claims: validStagingClaims,
      requestedDurationSeconds: 7200, // 2 hours exceeds max allowable boundary of 3600s
    });

    assert.equal(audit.decision, 'deny');
    assert.ok(
      audit.reasons.some((r) => r.includes('session duration out of boundaries')),
      'Should reject overly long session duration',
    );
  });

  it('negative: fail-closed on missing claims payload', () => {
    const audit = evaluateOidcTrustPolicy({
      policy: DEFAULT_TRUST_POLICY,
      claims: null,
    });

    assert.equal(audit.decision, 'deny');
    assert.ok(
      audit.reasons.some((r) => r.includes('missing OIDC claims payload')),
      'Should fail closed on missing claims',
    );
  });
});
