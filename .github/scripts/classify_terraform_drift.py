#!/usr/bin/env python3
"""Classify drift from a refresh-only Terraform plan without printing values.

Reads `terraform show -json <refresh-only plan>` on stdin. For every object
changed outside Terraform it reports the address, the kind of change, and the
*names* of changed attributes — never their values, because state and plan
JSON can contain secrets.

Each drifted object is classified:
  sensitive  security boundary, identity, encryption, data durability, or a
             resource deleted outside Terraform — owners review within 1 business day
  standard   any other functional change
  cosmetic   only tags changed

Writes a Markdown report to --report and a JSON summary to --summary, and
prints the summary. See terraform/DRIFT.md.
"""

from __future__ import annotations

import argparse
import json
import sys

SENSITIVE_TYPES = {
    "aws_security_group",
    "aws_security_group_rule",
    "aws_vpc_security_group_ingress_rule",
    "aws_vpc_security_group_egress_rule",
    "aws_iam_role",
    "aws_iam_role_policy",
    "aws_iam_role_policy_attachment",
    "aws_iam_policy",
    "aws_iam_openid_connect_provider",
    "aws_iam_instance_profile",
    "aws_kms_key",
    "aws_kms_alias",
    "aws_s3_bucket_policy",
    "aws_s3_bucket_public_access_block",
    "aws_s3_bucket_server_side_encryption_configuration",
    "aws_db_instance",
    "aws_db_parameter_group",
    "aws_db_subnet_group",
    "aws_elasticache_replication_group",
    "aws_elasticache_cluster",
    "aws_lb_listener",
    "aws_acm_certificate",
    "aws_acm_certificate_validation",
    "aws_route53_record",
    "aws_secretsmanager_secret",
    "aws_secretsmanager_secret_policy",
}
# Attribute names that are sensitive on any resource type.
SENSITIVE_ATTRIBUTE_HINTS = (
    "policy",
    "ingress",
    "egress",
    "cidr",
    "publicly_accessible",
    "public_ip",
    "encrypt",
    "kms",
    "deletion_protection",
    "backup",
    "iam",
    "role",
    "password",
    "certificate",
    "ssl",
    "security_group",
)
COSMETIC_ATTRIBUTES = {"tags", "tags_all"}


def changed_attributes(before: object, after: object) -> list[str]:
    if not isinstance(before, dict) or not isinstance(after, dict):
        return []
    keys = set(before) | set(after)
    return sorted(key for key in keys if before.get(key) != after.get(key))


def classify(resource_type: str, actions: list[str], attributes: list[str]) -> tuple[str, str]:
    if "delete" in actions:
        return "sensitive", "deleted outside Terraform"
    if attributes and set(attributes) <= COSMETIC_ATTRIBUTES:
        return "cosmetic", "tags only"
    if resource_type in SENSITIVE_TYPES:
        return "sensitive", f"{resource_type} is a security or durability boundary"
    hits = [a for a in attributes if any(h in a.lower() for h in SENSITIVE_ATTRIBUTE_HINTS)]
    if hits:
        return "sensitive", "changed " + ", ".join(hits)
    return "standard", "functional change"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--environment", required=True)
    parser.add_argument("--report", required=True, help="Markdown output path")
    parser.add_argument("--summary", required=True, help="JSON summary output path")
    parser.add_argument("--run-url", default="", help="link to the workflow run")
    parser.add_argument("--doc-url", default="terraform/DRIFT.md", help="link to the drift process doc")
    args = parser.parse_args()

    plan = json.load(sys.stdin)
    findings = []
    for item in plan.get("resource_drift", []) or []:
        change = item.get("change", {}) or {}
        actions = change.get("actions", []) or []
        if actions in (["no-op"], ["read"]):
            continue
        attributes = changed_attributes(change.get("before"), change.get("after"))
        level, reason = classify(item.get("type", ""), actions, attributes)
        findings.append(
            {
                "address": item.get("address", "unknown"),
                "type": item.get("type", ""),
                "actions": actions,
                "attributes": attributes,
                "level": level,
                "reason": reason,
            }
        )

    order = {"sensitive": 0, "standard": 1, "cosmetic": 2}
    findings.sort(key=lambda f: (order[f["level"]], f["address"]))
    counts = {level: sum(1 for f in findings if f["level"] == level) for level in order}

    lines = [f"## Terraform drift report — `{args.environment}`", ""]
    if args.run_url:
        lines += [f"Run: {args.run_url}", ""]
    if not findings:
        lines.append("No drift: every managed object matches Terraform state.")
    else:
        lines += [
            "| Sensitive | Standard | Cosmetic |",
            "|---:|---:|---:|",
            f"| {counts['sensitive']} | {counts['standard']} | {counts['cosmetic']} |",
            "",
            "| Level | Resource | Change | Attributes changed | Why |",
            "|---|---|---|---|---|",
        ]
        for f in findings:
            attrs = ", ".join(f"`{a}`" for a in f["attributes"]) or "—"
            lines.append(
                f"| **{f['level']}** | `{f['address']}` | {', '.join(f['actions'])} | {attrs} | {f['reason']} |"
            )
        lines += [
            "",
            "Nothing has been changed. Decide per resource whether to **codify** the change "
            "in Terraform or **revert** it, following the approval process in "
            f"[`terraform/DRIFT.md`]({args.doc_url}).",
        ]
    lines += ["", "Attribute values are intentionally omitted."]

    with open(args.report, "w", encoding="utf-8") as handle:
        handle.write("\n".join(lines) + "\n")
    summary = {
        "environment": args.environment,
        "drifted": len(findings),
        "sensitive": counts["sensitive"],
        "standard": counts["standard"],
        "cosmetic": counts["cosmetic"],
    }
    with open(args.summary, "w", encoding="utf-8") as handle:
        json.dump(summary, handle)
    print(json.dumps(summary))
    return 0


if __name__ == "__main__":
    sys.exit(main())
