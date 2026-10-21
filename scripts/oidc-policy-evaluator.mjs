#!/usr/bin/env node
/**
 * OIDC Identity Policy Evaluator for Cloud Credentials Exchange.
 * Validates that GitHub Actions identity claims strictly match trust policies:
 * - Only the intended repository (e.g. Daluworks/predifi)
 * - Only the intended branch/environment (e.g. refs/heads/main, staging)
 * - Only the intended audience (e.g. sts.amazonaws.com)
 * - Session duration within bounds (min 900s, max 3600s)
 * - Denies untrusted PRs, forks, pull_request refs, and unauthorized branches.
 */

export const DEFAULT_TRUST_POLICY = {
  authorizedRepository: 'Daluworks/predifi',
  authorizedBranch: 'main',
  authorizedAudience: 'sts.amazonaws.com',
  minDurationSeconds: 900,
  maxDurationSeconds: 3600,
  allowPullRequests: false,
};

export function evaluateOidcTrustPolicy({
  policy = DEFAULT_TRUST_POLICY,
  claims = {},
  requestedDurationSeconds = 900,
} = {}) {
  const audit = {
    at: new Date().toISOString(),
    decision: 'deny',
    reasons: [],
    claimsSummary: claims && typeof claims === 'object' ? {
      repository: claims.repository,
      ref: claims.ref,
      event_name: claims.event_name,
      sub: claims.sub,
      aud: claims.aud,
    } : null,
  };

  if (!claims || typeof claims !== 'object') {
    audit.reasons.push('missing OIDC claims payload — fail closed');
    return audit;
  }

  // 1. Audience verification
  if (claims.aud !== policy.authorizedAudience) {
    audit.reasons.push(
      `invalid audience: "${claims.aud}" does not match required "${policy.authorizedAudience}"`,
    );
  }

  // 2. Repository verification
  if (claims.repository !== policy.authorizedRepository) {
    audit.reasons.push(
      `untrusted repository: "${claims.repository}" does not match authorized "${policy.authorizedRepository}"`,
    );
  }

  // 3. Pull Request rejection
  const isPullRequestEvent =
    claims.event_name === 'pull_request' ||
    claims.event_name === 'pull_request_target';
  const isPullRequestRef =
    typeof claims.ref === 'string' && claims.ref.startsWith('refs/pull/');

  if (!policy.allowPullRequests && (isPullRequestEvent || isPullRequestRef)) {
    audit.reasons.push(
      `untrusted pull-request workflow: pull requests are strictly barred from assuming deployment roles`,
    );
  }

  // 4. Branch verification
  const expectedRef = `refs/heads/${policy.authorizedBranch}`;
  if (claims.ref !== expectedRef) {
    audit.reasons.push(
      `untrusted ref: "${claims.ref}" is not the authorized deployment branch "${expectedRef}"`,
    );
  }

  // 5. Subject claim verification (sub claim format in GitHub OIDC: repo:<org>/<repo>:ref:refs/heads/<branch>)
  const expectedSub = `repo:${policy.authorizedRepository}:ref:${expectedRef}`;
  if (claims.sub !== expectedSub) {
    audit.reasons.push(
      `invalid subject claim: "${claims.sub}" does not match strict expected subject "${expectedSub}"`,
    );
  }

  // 6. Session duration boundary check
  if (
    typeof requestedDurationSeconds !== 'number' ||
    requestedDurationSeconds < policy.minDurationSeconds ||
    requestedDurationSeconds > policy.maxDurationSeconds
  ) {
    audit.reasons.push(
      `session duration out of boundaries: requested ${requestedDurationSeconds}s (allowed: ${policy.minDurationSeconds}s - ${policy.maxDurationSeconds}s)`,
    );
  }

  if (audit.reasons.length === 0) {
    audit.decision = 'allow';
  }

  return audit;
}
