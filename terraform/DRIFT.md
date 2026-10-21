# Terraform Drift Detection and Review

**Drift** is any difference between real infrastructure and Terraform state
caused by a change made outside Terraform: a console edit, a CLI hotfix during
an incident, or an AWS-initiated change. Drift is not always wrong. An
emergency change may be the only thing keeping production up, so drift is
**reported and reviewed, never overwritten automatically**.

## How detection works

[`.github/workflows/terraform-drift.yml`](../.github/workflows/terraform-drift.yml)
runs at 06:23 UTC on weekdays (and on demand) for staging and production:

1. Assumes a **read-only** AWS role through GitHub OIDC.
2. Runs `terraform plan -refresh-only -lock=false -detailed-exitcode`. A
   refresh-only plan compares real infrastructure with state and proposes no
   configuration changes. Nothing is written back to state, and no lock is
   taken, so it never blocks or interferes with a real apply.
3. Converts the plan to JSON, then
   [`.github/scripts/classify_terraform_drift.py`](../.github/scripts/classify_terraform_drift.py)
   classifies every drifted object and writes a report containing **resource
   addresses and changed attribute names only**. Values are never printed, and
   the plan and its JSON are deleted from the runner straight after.
4. Opens (or comments on) a `Terraform drift: <environment>` issue labelled
   `terraform-drift`, mentioning `@Web3Novalabs/infra-reviewers`. When a later
   run finds no drift, it comments on the open issue so someone can close it.
5. Marks the run failed when any **sensitive** drift exists, so it shows up red
   in the Actions tab as well as in the issue.

### Guarantees that it cannot change infrastructure

- The workflow contains no mutating Terraform command. The
  *Terraform Safety* workflow fails any PR that adds `apply`, `destroy`,
  `import`, `taint`, `force-unlock`, or `state rm/mv/push` to it, or removes
  `-refresh-only` / `-lock=false`.
- The AWS role it assumes has read-only permissions (below). Even a modified
  workflow could not change resources with it.
- The GitHub environments it runs in (`terraform-drift-staging`,
  `terraform-drift-production`) hold only that role and state location — no
  deploy credentials.

## Classification

| Level | Meaning | Examples | Response time |
|---|---|---|---|
| **sensitive** | A security boundary, identity, encryption, or data-durability setting changed, or a resource was deleted outside Terraform | Security group rules, IAM roles/policies, KMS keys, S3 bucket policy / public access, RDS public access, backups, deletion protection, TLS listeners, DNS records | Triage within **1 business day**; if unexplained, treat as a security incident |
| **standard** | A functional setting changed | Instance type, ASG capacity, Redis node count, parameter values | Triage within **5 business days** |
| **cosmetic** | Only `tags` / `tags_all` changed | Someone added a cost-centre tag | Fold into the next routine Terraform PR |

Sensitive resource types and attribute hints are listed in the classifier
script. Add to them in the same PR as any new security-relevant resource.

## Review and remediation process

No one runs `apply` to make drift go away until this process says so.

1. **Acknowledge.** An infra owner assigns the drift issue to themselves.
2. **Find the source.** Use CloudTrail (filter on the resource ID and the time
   window between the last clean run and this one) to find who changed it and
   why. Check the incident log for emergency changes.
3. **Decide** — record the decision in the issue:
   - **Keep (codify):** the change is intentional, e.g. an incident hotfix.
     Open a PR that updates the Terraform configuration so a normal plan shows
     *no changes* for that resource. For a resource created by hand, use an
     `import` block in the PR. The drift issue links the PR.
   - **Revert:** the change was accidental or unauthorized. Open a PR (it may
     contain no code change) whose staging plan shows the revert, and state in
     the description which drifted attributes will be reset.
   - **Unexplained sensitive drift:** treat as a possible compromise. Page
     infra-oncall and `@Web3Novalabs/maintainers`, preserve CloudTrail
     evidence, and do not revert until the security review is done — reverting
     can destroy evidence.
4. **Approve.** Remediation PRs follow normal Terraform review (CODEOWNERS:
   `@Web3Novalabs/infra-reviewers`) plus:
   - **staging:** one infra reviewer approves the PR and the plan summary.
   - **production:** two approvals, at least one from an infra reviewer who
     did not make the original change. The apply runs through the protected
     deploy path, never from the drift workflow.
   - **revert of an emergency change:** also needs sign-off from the incident
     owner, confirming the system no longer depends on the change.
5. **Apply** through the normal, reviewed pipeline after merge.
6. **Verify.** Run *Terraform Drift* manually for the environment. It must
   report no drift for the resource. Close the issue with links to the PR and
   the clean run.

## Setup

### GitHub environments

Create `terraform-drift-staging` and `terraform-drift-production` with **no
required reviewers** (the job is scheduled) and these environment variables:

| Variable | Value |
|---|---|
| `TERRAFORM_DRIFT_ROLE_ARN` | Read-only drift role for that environment |
| `TERRAFORM_DRIFT_AWS_REGION` | e.g. `us-east-1` |
| `TERRAFORM_STATE_BUCKET`, `TERRAFORM_STATE_REGION`, `TERRAFORM_STATE_KMS_KEY_ID` | State backend (see [STATE_AND_POLICY.md](STATE_AND_POLICY.md)) |
| `TERRAFORM_DRIFT_VPC_ID`, `TERRAFORM_DRIFT_AMI_ID`, `TERRAFORM_DRIFT_DOMAIN_NAME`, `TERRAFORM_DRIFT_ROUTE53_ZONE_ID` | Same values as that environment's `terraform.tfvars` |
| `TERRAFORM_DRIFT_PUBLIC_SUBNET_IDS`, `TERRAFORM_DRIFT_PRIVATE_SUBNET_IDS` | JSON arrays, e.g. `["subnet-a","subnet-b"]` |

Until these are set, the job reports *not configured* and exits cleanly.

### Read-only AWS role

Trust policy: GitHub OIDC, `aud = sts.amazonaws.com`, and
`sub = repo:<org>/<repo>:environment:terraform-drift-<env>` — so only this
workflow, in that environment, can assume it. Never trust `pull_request`.

Permissions:

- AWS managed policy `ReadOnlyAccess` (or a narrower Describe/Get/List policy
  for the services in `terraform/modules/`).
- `s3:GetObject` on `arn:aws:s3:::<state-bucket>/<env>/terraform.tfstate` and
  `s3:ListBucket` on the bucket — **no** `PutObject` / `DeleteObject`, not even
  on the `.tflock` object (the plan runs with `-lock=false`).
- `kms:Decrypt` on the state KMS key.
- An explicit `Deny` on `*:Create*`, `*:Delete*`, `*:Put*`, `*:Update*`,
  `*:Modify*`, `*:Attach*`, `*:Detach*`, and `iam:PassRole` as defense in
  depth.

## Validation in a non-production workspace

Run this in **staging** before enabling the schedule, and after any change to
the workflow or role:

1. Note the time. Make a known manual change, e.g. add the tag
   `DriftTest=<date>` and an ingress rule `tcp/9999` from `10.255.255.255/32` to
   the monitoring security group:
   ```sh
   aws ec2 create-tags --resources <sg-id> --tags Key=DriftTest,Value=$(date +%F)
   aws ec2 authorize-security-group-ingress --group-id <sg-id> --protocol tcp --port 9999 --cidr 10.255.255.255/32
   ```
2. Run *Actions → Terraform Drift → Run workflow* with `staging`.
3. Expected:
   - The issue `Terraform drift: staging` exists and lists
     `module.predifi.module.monitoring.aws_security_group.monitoring` as
     **sensitive**, with `ingress` and `tags`/`tags_all` as changed attributes.
   - No attribute values (CIDRs, IDs) appear in the issue or the logs.
   - The run is marked failed because of sensitive drift.
4. Confirm nothing was changed: the security group still has the test rule,
   the state object's S3 version ID is unchanged, and CloudTrail shows only
   read-only (`Describe*`, `Get*`, `List*`) calls from the drift role session
   (`predifi-drift-staging-<run id>`).
5. Remove the manual change by hand:
   ```sh
   aws ec2 revoke-security-group-ingress --group-id <sg-id> --protocol tcp --port 9999 --cidr 10.255.255.255/32
   aws ec2 delete-tags --resources <sg-id> --tags Key=DriftTest
   ```
6. Run the workflow again. It must comment *No drift detected* on the issue.
   Close the issue.
7. Record the operator, times, run URLs, and S3 version IDs in the
   infrastructure exercise record.
