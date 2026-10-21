# Terraform State and Policy

## Remote state

Staging and production use separate keys in a private S3 bucket. Before the first `terraform init`, an infrastructure administrator must provision the bucket with versioning enabled, Block Public Access enabled, default SSE-KMS encryption, and a bucket policy restricted to the Terraform operator roles. The KMS key policy must grant those same roles encrypt/decrypt access. Terraform 1.10 or later uses the S3 backend's native lockfile to serialize plans and applies; grant each role access to read/write/delete only the matching `.tflock` object. Keep the bootstrap administrator separate from the roles that operate application infrastructure.

Copy the matching `backend.hcl.example` to `backend.hcl`, replace the bucket and KMS placeholders, and initialize with:

```sh
terraform init -backend-config=backend.hcl
```

Do not commit `backend.hcl`, local state, state backups, plan files, or `terraform show` output. Terraform state can contain passwords and other secrets even when outputs are marked sensitive. Never upload state or plan JSON as a CI artifact, print it in logs, or expose it in a pull-request comment. The PR plan job reports resource addresses and actions only.

Grant each environment role access only to its exact state key, the lock-table item operations it needs, and the KMS key. Keep the state bucket and database credentials separate from application credentials. Review changes to backend policy and KMS access with the infrastructure owners.

## Locking and recovery exercise

Run this exercise in staging with two operator sessions and no production apply:

1. In session A, run a reviewed `terraform plan` against staging and hold the S3 lockfile using a non-production test operation.
2. Start a second plan in session B. Confirm it waits or exits with a lock error and reports the lock identifier without revealing state values.
3. Release session A and confirm session B can acquire and release the lock.
4. Record the S3 state object's current version ID. Restore a prior version to a separate recovery key, initialize a temporary staging workspace against that key, and verify the state can be read. Do not overwrite the live key during the exercise.
5. Record operator, timestamps, lock result, tested object version IDs, and cleanup confirmation in the infrastructure exercise record.

For an actual recovery, stop all Terraform operations first, identify the last known-good S3 object version, and have a second infrastructure owner review the target key and version before restoration. Preserve the damaged version for investigation. Reinitialize only after the restored object has been verified and the lock table is clear.

## Pull-request controls

The Terraform workflow formats and validates the root and environment configurations, then runs Checkov on changed Terraform files. Exceptions must be inline Checkov skip annotations with a concrete justification; reviewers should reject blanket or unexplained skips. Configure the `TERRAFORM_PLAN_*` repository variables to enable the read-only staging plan job. Its AWS OIDC role must be restricted to the repository's pull-request identity, staging resources, staging state key, lock table, and state KMS key. The job writes a resource/action-only summary and retains no plan artifact.

The plan role, source state, and network access are prerequisites; without those repository variables the static format, validation, and policy gates still run, while the plan job is skipped. Configure `TERRAFORM_PLAN_ROLE_ARN`, `TERRAFORM_PLAN_AWS_REGION`, `TERRAFORM_STATE_BUCKET`, `TERRAFORM_STATE_REGION`, `TERRAFORM_STATE_KMS_KEY_ID`, and the `TERRAFORM_PLAN_VPC_ID`, `TERRAFORM_PLAN_PUBLIC_SUBNET_IDS`, `TERRAFORM_PLAN_PRIVATE_SUBNET_IDS`, `TERRAFORM_PLAN_AMI_ID`, `TERRAFORM_PLAN_DOMAIN_NAME`, and `TERRAFORM_PLAN_ROUTE53_ZONE_ID` staging variables. Subnet ID variables must be JSON arrays. The plan job is restricted to same-repository pull requests; never grant state-reading OIDC credentials to fork pull requests, whose code and workflow changes are untrusted. The workflow's OIDC trust must allow only the trusted repository identity. The role may read staging infrastructure and state and write only the staging `.tflock` object; it must not apply infrastructure changes. Fork pull requests still receive formatting, validation, and policy scanning, but no cloud plan.

## Drift detection

A scheduled, read-only refresh-only plan reports infrastructure changed outside Terraform and opens an issue for the owners. It never applies changes; remediation goes through a reviewed PR. See [DRIFT.md](DRIFT.md).
