#!/usr/bin/env node
/**
 * Evaluates container image vulnerability scans against security policy.
 * Enforces blocking severities, fix SLA windows, and time-limited exception records.
 * Reports affected OS package paths and ensures unexempted findings fail the gate.
 *
 * Usage:
 *   node scripts/evaluate-container-vulnerabilities.mjs --policy deploy/security/vulnerability-policy.json --reports .scan-reports
 *   node scripts/evaluate-container-vulnerabilities.mjs --policy deploy/security/vulnerability-policy.json --report path/to/scan.json
 */

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.join(__dirname, '..');

export const DEFAULT_POLICY_PATH = path.join(ROOT, 'deploy/security/vulnerability-policy.json');

/**
 * Load and validate vulnerability policy configuration.
 */
export function loadPolicy(policyPath = DEFAULT_POLICY_PATH) {
  if (!fs.existsSync(policyPath)) {
    throw new Error(`Vulnerability policy not found at: ${policyPath}`);
  }
  const content = fs.readFileSync(policyPath, 'utf8');
  const policy = JSON.parse(content);

  if (!policy.severities?.blocking || !Array.isArray(policy.severities.blocking)) {
    throw new Error('Policy must define severities.blocking array');
  }
  if (!policy.fixSlaDays || typeof policy.fixSlaDays !== 'object') {
    throw new Error('Policy must define fixSlaDays mapping');
  }

  return policy;
}

/**
 * Normalizes scan findings from Trivy or standard JSON container scan reports.
 */
export function extractFindings(reportData, defaultArtifactName = 'unknown-image') {
  const artifactName = reportData.ArtifactName || defaultArtifactName;
  const findings = [];

  const results = Array.isArray(reportData.Results) ? reportData.Results : [];
  for (const res of results) {
    const target = res.Target || artifactName;
    const targetClass = res.Class || 'os-pkgs';
    const targetType = res.Type || '';

    const vulnerabilities = Array.isArray(res.Vulnerabilities) ? res.Vulnerabilities : [];
    for (const v of vulnerabilities) {
      const vulnId = v.VulnerabilityID || v.id || 'UNKNOWN-VULN';
      const pkgName = v.PkgName || v.package || 'unknown-pkg';
      const installedVersion = v.InstalledVersion || v.version || 'unknown-version';
      const fixedVersion = v.FixedVersion || v.fixed_version || null;
      const severity = (v.Severity || v.severity || 'UNKNOWN').toUpperCase();
      const publishedDate = v.PublishedDate || v.published_date || null;
      const title = v.Title || v.title || '';
      const primaryUrl = v.PrimaryURL || v.url || '';

      // Determine package path (e.g. /usr/lib/..., /lib/..., dpkg/apk package path)
      let packagePath = v.PkgPath || v.package_path || null;
      if (!packagePath) {
        if (targetType === 'debian' || targetType === 'ubuntu') {
          packagePath = `/var/lib/dpkg/info/${pkgName}.list`;
        } else if (targetType === 'alpine') {
          packagePath = `/lib/apk/db/installed (${pkgName})`;
        } else {
          packagePath = `${target} [${targetClass}/${pkgName}]`;
        }
      }

      findings.push({
        artifactName,
        target,
        targetClass,
        targetType,
        vulnerabilityId: vulnId,
        packageName: pkgName,
        installedVersion,
        fixedVersion,
        packagePath,
        severity,
        publishedDate,
        title,
        primaryUrl,
      });
    }
  }

  return findings;
}

/**
 * Evaluates vulnerabilities against policy, checking blocking severity, SLA, and time-limited exceptions.
 */
export function evaluateVulnerabilities({
  policy,
  findings,
  asOfDate = new Date(),
}) {
  const now = new Date(asOfDate);
  const blockingSeverities = new Set((policy.severities?.blocking || ['CRITICAL', 'HIGH']).map((s) => s.toUpperCase()));
  const fixSlaDays = policy.fixSlaDays || { CRITICAL: 7, HIGH: 14, MEDIUM: 30, LOW: 90 };
  const exceptions = Array.isArray(policy.exceptions) ? policy.exceptions : [];

  const evaluation = {
    evaluatedAt: now.toISOString(),
    totalFindings: findings.length,
    decision: 'allow',
    violationsCount: 0,
    exemptionsCount: 0,
    expiredExceptionsCount: 0,
    findings: [],
    violations: [],
    exempted: [],
    summaryBySeverity: {
      CRITICAL: 0,
      HIGH: 0,
      MEDIUM: 0,
      LOW: 0,
      UNKNOWN: 0,
    },
  };

  for (const item of findings) {
    const sev = item.severity || 'UNKNOWN';
    evaluation.summaryBySeverity[sev] = (evaluation.summaryBySeverity[sev] || 0) + 1;

    let isBlockingSeverity = blockingSeverities.has(sev);
    let isSlaBreach = false;
    let slaDaysLimit = fixSlaDays[sev] ?? null;
    let ageInDays = null;

    if (item.publishedDate) {
      const pubDate = new Date(item.publishedDate);
      if (!isNaN(pubDate.getTime())) {
        const diffMs = now.getTime() - pubDate.getTime();
        ageInDays = Math.max(0, Math.floor(diffMs / (1000 * 60 * 60 * 24)));
        if (slaDaysLimit !== null && ageInDays > slaDaysLimit) {
          isSlaBreach = true;
        }
      }
    }

    // Check exceptions matching this finding
    const matchingException = exceptions.find((exc) => {
      if (exc.vulnerabilityId !== item.vulnerabilityId) return false;
      if (exc.target && exc.target !== '*' && exc.target !== item.artifactName && !item.target.includes(exc.target)) {
        return false;
      }
      if (exc.packageName && exc.packageName !== item.packageName) {
        return false;
      }
      if (exc.packagePath && item.packagePath && exc.packagePath !== item.packagePath) {
        return false;
      }
      return true;
    });

    let status = 'allowed';
    let exceptionStatus = null;
    let exemptionReason = null;
    let exceptionId = null;

    if (matchingException) {
      exceptionId = matchingException.id;
      const expiry = new Date(matchingException.expiresAt);
      if (!isNaN(expiry.getTime()) && expiry.getTime() >= now.getTime()) {
        status = 'exempted';
        exceptionStatus = 'active';
        exemptionReason = matchingException.reason;
        evaluation.exemptionsCount++;
      } else {
        status = 'exception_expired';
        exceptionStatus = 'expired';
        exemptionReason = `Exception ${matchingException.id} expired on ${matchingException.expiresAt}`;
        evaluation.expiredExceptionsCount++;
      }
    } else if (isBlockingSeverity || isSlaBreach) {
      status = 'violation';
    }

    const processedFinding = {
      ...item,
      ageInDays,
      slaDaysLimit,
      isSlaBreach,
      isBlockingSeverity,
      status,
      exceptionId,
      exceptionStatus,
      exemptionReason,
    };

    evaluation.findings.push(processedFinding);

    if (status === 'violation' || status === 'exception_expired') {
      evaluation.violationsCount++;
      evaluation.violations.push(processedFinding);
    } else if (status === 'exempted') {
      evaluation.exempted.push(processedFinding);
    }
  }

  evaluation.decision = evaluation.violationsCount > 0 ? 'deny' : 'allow';
  return evaluation;
}

/**
 * Generate human-readable console report.
 */
export function formatReport(evaluation) {
  const lines = [];
  lines.push('================================================================================');
  lines.push('               CONTAINER IMAGE VULNERABILITY POLICY REPORT                     ');
  lines.push('================================================================================');
  lines.push(`Decision:      ${evaluation.decision.toUpperCase()}`);
  lines.push(`Evaluated at:  ${evaluation.evaluatedAt}`);
  lines.push(`Total scanned: ${evaluation.totalFindings} vulnerabilities`);
  lines.push(`Violations:    ${evaluation.violationsCount}`);
  lines.push(`Active Exempt: ${evaluation.exemptionsCount}`);
  lines.push(`Expired Excs:  ${evaluation.expiredExceptionsCount}`);
  lines.push('--------------------------------------------------------------------------------');
  lines.push('Severities: ' + Object.entries(evaluation.summaryBySeverity).map(([k, v]) => `${k}:${v}`).join(' '));
  lines.push('================================================================================');

  if (evaluation.violations.length > 0) {
    lines.push('\n[!] POLICY VIOLATIONS (FAIL GATES):');
    for (const v of evaluation.violations) {
      const slaNote = v.isSlaBreach ? ` [SLA EXCEEDED: ${v.ageInDays}d > ${v.slaDaysLimit}d]` : '';
      const expNote = v.status === 'exception_expired' ? ` [${v.exemptionReason}]` : '';
      lines.push(` - [${v.severity}] ${v.vulnerabilityId} in ${v.packageName} (${v.installedVersion})`);
      lines.push(`   Image target: ${v.target}`);
      lines.push(`   Package Path: ${v.packagePath}`);
      if (v.fixedVersion) lines.push(`   Fixed version: ${v.fixedVersion}`);
      if (slaNote) lines.push(`   SLA Status: ${slaNote.trim()}`);
      if (expNote) lines.push(`   Exception:  ${expNote.trim()}`);
    }
  }

  if (evaluation.exempted.length > 0) {
    lines.push('\n[*] ACTIVE EXCEPTIONS (TIME-LIMITED & AUDITED):');
    for (const e of evaluation.exempted) {
      lines.push(` - [EXEMPTED] ${e.vulnerabilityId} in ${e.packageName} (${e.installedVersion})`);
      lines.push(`   Exception ID: ${e.exceptionId}`);
      lines.push(`   Package Path: ${e.packagePath}`);
      lines.push(`   Reason:       ${e.exemptionReason}`);
    }
  }

  lines.push('================================================================================\n');
  return lines.join('\n');
}

/**
 * Main CLI entry point.
 */
function arg(name) {
  const i = process.argv.indexOf(`--${name}`);
  if (i >= 0 && process.argv[i + 1]) return process.argv[i + 1];
  return undefined;
}

export function runCli() {
  const policyFile = arg('policy') || DEFAULT_POLICY_PATH;
  const reportsDir = arg('reports');
  const reportFile = arg('report');
  const asOf = arg('as-of');

  let policy;
  try {
    policy = loadPolicy(policyFile);
  } catch (err) {
    console.error(`FAIL: Unable to load policy: ${err.message}`);
    process.exit(1);
  }

  const reportPaths = [];
  if (reportFile) {
    reportPaths.push(reportFile);
  } else if (reportsDir && fs.existsSync(reportsDir)) {
    const entries = fs.readdirSync(reportsDir);
    for (const entry of entries) {
      if (entry.endsWith('.json')) {
        reportPaths.push(path.join(reportsDir, entry));
      }
    }
  } else {
    console.error('Error: specify --report <file.json> or --reports <dir>');
    process.exit(2);
  }

  let allFindings = [];
  for (const p of reportPaths) {
    try {
      const data = JSON.parse(fs.readFileSync(p, 'utf8'));
      const artifactName = path.basename(p, '.json');
      const findings = extractFindings(data, artifactName);
      allFindings = allFindings.concat(findings);
    } catch (err) {
      console.error(`Error reading scan report ${p}: ${err.message}`);
      process.exit(1);
    }
  }

  const evaluation = evaluateVulnerabilities({
    policy,
    findings: allFindings,
    asOfDate: asOf ? new Date(asOf) : new Date(),
  });

  const reportOutput = formatReport(evaluation);
  console.log(reportOutput);

  if (process.env.GITHUB_STEP_SUMMARY) {
    try {
      fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, `\`\`\`\n${reportOutput}\n\`\`\`\n`);
    } catch {}
  }

  if (evaluation.decision === 'deny') {
    console.error(`FAIL: Container vulnerability policy gate failed with ${evaluation.violationsCount} violation(s).`);
    process.exit(1);
  }

  console.log('SUCCESS: Container vulnerability policy gate passed.');
  process.exit(0);
}

const isMain = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  runCli();
}
