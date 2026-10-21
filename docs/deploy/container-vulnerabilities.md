# Container Image Vulnerability Management & Gate

Policy: `deploy/security/vulnerability-policy.json`  
Evaluator: `scripts/evaluate-container-vulnerabilities.mjs`  
Gate: `.github/workflows/backend.yml` (`vulnerability-scan` job preceding `deploy-staging`)  
Tests: `tests/container-vulnerability-policy.test.mjs`

## Overview

PreDiFi enforces proactive detection of vulnerable base container images and transitive operating system packages prior to staging and production deployments. Built application images (`predifi-backend:latest`) and supporting services (`postgres:15-alpine`, `redis:7-alpine`) are scanned using Trivy and evaluated against strict security policies.

## Policy Specifications

### Blocking Severities

The deployment gate immediately halts deployments upon encountering unexempted findings matching blocking severities:
- `CRITICAL`
- `HIGH`

### Remediation Service Level Agreements (SLAs)

Remediation timelines are enforced based on vulnerability age:

| Severity   | Fix SLA | Gate Impact upon SLA Expiry |
|:-----------|:--------|:----------------------------|
| `CRITICAL` | 7 days  | Blocks deployment           |
| `HIGH`     | 14 days | Blocks deployment           |
| `MEDIUM`   | 30 days | Blocks deployment on breach |
| `LOW`      | 90 days | Advisory / Warning          |

### Time-Limited Exception Handling

Exceptions may be granted when upstream vendor patches are pending or compensating controls exist at network/infrastructure boundaries.

Every exception record in `deploy/security/vulnerability-policy.json` requires:
- `id`: Unique tracking reference (`EXC-YYYY-NNN`)
- `vulnerabilityId`: CVE or security identifier (`CVE-YYYY-NNNN`)
- `target`: Scoped container image target or `*`
- `packageName`: Target OS package name
- `packagePath`: File path or package registry identifier
- `reason`: Technical rationale and risk justification
- `approvedBy`: Auditor/security lead identifier
- `approvedAt`: ISO 8601 approval timestamp
- `expiresAt`: ISO 8601 expiration date (time-limited)

### Unsuppressed Unrelated Findings

The evaluator strictly isolates exception records to their specific CVE, target, and package. An active exception for one vulnerability does not suppress unrelated findings in the same image or across supporting services. If an unrelated vulnerability is detected or if an exception expires, the deployment gate fails closed.

## Evaluation & CLI Usage

Run evaluator locally against scan reports:
```bash
node scripts/evaluate-container-vulnerabilities.mjs \
  --policy deploy/security/vulnerability-policy.json \
  --reports .scan-reports
```

Run test suite:
```bash
node --test tests/container-vulnerability-policy.test.mjs
```
