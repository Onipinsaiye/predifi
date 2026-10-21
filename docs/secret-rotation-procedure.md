# Secret Rotation Procedure

This document defines the required steps when a credential is confirmed
leaked into the repository history or otherwise exposed. The goal is to revoke
the exposed credential, replace it with a new one, and prevent recurrence.

## Detection

Secrets are detected by the self-hosted scanner in `.github/scripts/scan_secrets.py`,
run by the `Workflow in `.github/workflows/secret-scanning.yml`. The workflow:

- Runs on every pull request and on pushes to `main`, `develop`, `contract`, and `frontend`.
- Scans only the lines added between the base and head commits.
- Fails the job when a confirmed leak is found.
- Runs a self-test against a synthetic canary secret to prove the scanner is
  functioning.

The scanner never writes the raw secret value to logs or to the report. Only the
rule id, file path, line number, and a short fingerprint hash are emitted.

## False-Positive suppression

Suppressions are managed in `.github/secret-scanning/allowlist.txt`. Each
entry is a fingerprint hash, not a secret value. To suppress a false positive:

1. Run the scanner locally against your branch:
   ```bash
   python .github/scripts/scan_secrets.py --base origin/main --head HEAD \
     --allowlist .github/secret-scanning/allowlist.txt
   ```
2. Copy the `fingerprint=` value from the failure output.
3. Add the fingerprint to the allowlist with a comment explaining why it is safe.
4. Request review from a security maintainer.

Never add a real credential to the allowlist. If the credential is genuinely leaked,
rotate it first and remove it from the commit.

## Rotation procedure

Follow these steps in order. Do not skip steps when the credential is confirmed leaked.

### 1. Contain

Revoke the exposed credential at the source immediately. Do not wait for the
pull request to merge.

- AWS: deactivate the access key in IAM, then delete it.
- GitHub: revoke the personal access token or deploy key.
- Stripe / Slack / Google: revoke the key in the provider console.

### 2. Rotate

Create a new credential with the minimum scope required. Store it in the
repository's secret store (GitHub Actions secrets, or the deployment platform's
secret manager). Never commit it to the repository.

### 3. Remove from history

Remove the leaked value from the commit history using `git filter-repo`(or the platform's equivalent). Force-push the cleaned branch and notify collaborators
to re-clone. Rotation is required even after history removal because the value
may already have been captured.

### 4. Verify

- Confirm the old credential is rejected by the provider.
- Confirm the new credential works in CI and in the deployment environment.
- Re-run the secret scanner against the affected branch to confirm it is clean.

### 5. Record

Document the incident in the security log (commit sha, credential type,
rotation time, and owner). Do not include the exposed value.

## Verification of the scanner

To prove the scanner fails controlled on a leak, the workflow runs a
self-test against a synthetic canary secret. To reproduce locally:

```bash
python .github/scripts/scan_secrets.py --self-test \
  --allowlist .github/secret-scanning/allowlist.txt
echo "exit code: $$"
```

The command must print `self-test OK: canary secret detected` and exit with
code 0. If it does not, the scanner is broken and the workflow will fail.
