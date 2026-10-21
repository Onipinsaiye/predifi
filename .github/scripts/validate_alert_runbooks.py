#!/usr/bin/env python3
"""Fail if any production alert is unowned or lacks an actionable runbook.

For every alerting rule in ALERT_FILES this checks that:
  * labels.severity is one of SEVERITIES and labels.owner is in ALLOWED_OWNERS;
  * annotations summary, impact, runbook_url and dashboard_url are non-empty;
  * runbook_url points at docs/runbooks/<file>.md#<anchor>, the file exists,
    and it has a "## <Heading>" whose GitHub anchor matches;
  * that runbook section contains every field in REQUIRED_FIELDS and states the
    same owner and severity as the rule;
  * the alert is listed in the docs/runbooks/README.md inventory;
  * an alert defined in several files has the same owner and severity in each.

Usage: python .github/scripts/validate_alert_runbooks.py [--root PATH]
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

import yaml

ALERT_FILES = [
    "backend/grafana/alerts/predifi_alerts.yml",
    "backend/grafana/alerts/predifi_slo_rules.yml",
    "docker/grafana/provisioning/alerting/rules.yaml",
    "docker/loki/rules/fake/predifi-log-alerts.yml",
]

# Keep in sync with the "Owners" table in docs/runbooks/README.md.
ALLOWED_OWNERS = {"backend-oncall", "infra-oncall", "contract-oncall"}
SEVERITIES = {"critical", "warning"}
REQUIRED_ANNOTATIONS = ("summary", "impact", "runbook_url", "dashboard_url")
REQUIRED_FIELDS = (
    "**Owner:**",
    "**Severity:**",
    "**User impact:**",
    "**Dashboards & queries:**",
    "**Diagnosis:**",
    "**Safe mitigation:**",
    "**Verification:**",
    "**Rollback:**",
    "**Escalation:**",
)
RUNBOOK_PREFIX = "https://github.com/Onipinsaiye/predifi/blob/main/"
RUNBOOK_DIR = "docs/runbooks/"
INVENTORY = "docs/runbooks/README.md"


def github_anchor(heading: str) -> str:
    """Approximate GitHub's heading-to-anchor rule."""
    slug = heading.strip().lower()
    slug = re.sub(r"[^\w\- ]", "", slug)
    return slug.replace(" ", "-")


def runbook_sections(path: Path) -> dict[str, str]:
    """Map anchor -> section text for every level-2 heading in a runbook."""
    sections: dict[str, str] = {}
    current: str | None = None
    lines: list[str] = []
    in_fence = False
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.lstrip().startswith("```"):
            in_fence = not in_fence
        if not in_fence and line.startswith("## "):
            if current is not None:
                sections[current] = "\n".join(lines)
            current = github_anchor(line[3:])
            lines = []
        elif current is not None:
            lines.append(line)
    if current is not None:
        sections[current] = "\n".join(lines)
    return sections


def iter_alert_rules(path: Path):
    """Yield (name, labels, annotations) for each alerting rule in a rules file."""
    data = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    for group in data.get("groups", []) or []:
        for rule in group.get("rules", []) or []:
            if "record" in rule:
                continue  # recording rules do not page anyone
            name = rule.get("alert") or rule.get("title")
            if not name:
                yield None, rule.get("labels") or {}, rule.get("annotations") or {}
                continue
            yield name, rule.get("labels") or {}, rule.get("annotations") or {}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default=".", help="repository root")
    args = parser.parse_args()
    root = Path(args.root)

    errors: list[str] = []
    seen: dict[str, tuple[str, str, str]] = {}
    section_cache: dict[Path, dict[str, str]] = {}

    inventory_path = root / INVENTORY
    inventory = inventory_path.read_text(encoding="utf-8") if inventory_path.exists() else ""
    if not inventory:
        errors.append(f"{INVENTORY}: missing; it must list every alert")

    total = 0
    for rel in ALERT_FILES:
        path = root / rel
        if not path.exists():
            errors.append(f"{rel}: alert file not found")
            continue
        for name, labels, annotations in iter_alert_rules(path):
            total += 1
            where = f"{rel}: {name or '<unnamed rule>'}"
            if not name:
                errors.append(f"{where}: rule has no alert/title name")
                continue

            owner = str(labels.get("owner", "")).strip()
            severity = str(labels.get("severity", "")).strip()
            if owner not in ALLOWED_OWNERS:
                errors.append(
                    f"{where}: owner label {owner!r} is not one of {sorted(ALLOWED_OWNERS)}"
                )
            if severity not in SEVERITIES:
                errors.append(f"{where}: severity {severity!r} is not one of {sorted(SEVERITIES)}")
            for key in REQUIRED_ANNOTATIONS:
                if not str(annotations.get(key, "")).strip():
                    errors.append(f"{where}: missing annotation {key!r}")

            previous = seen.get(name)
            if previous and (previous[0], previous[1]) != (owner, severity):
                errors.append(
                    f"{where}: owner/severity ({owner}, {severity}) disagree with "
                    f"{previous[2]} ({previous[0]}, {previous[1]})"
                )
            seen.setdefault(name, (owner, severity, rel))

            if f"`{name}`" not in inventory:
                errors.append(f"{where}: not listed in the {INVENTORY} inventory table")

            url = str(annotations.get("runbook_url", "")).strip()
            if not url:
                continue
            if not url.startswith(RUNBOOK_PREFIX + RUNBOOK_DIR) or "#" not in url:
                errors.append(
                    f"{where}: runbook_url must look like {RUNBOOK_PREFIX}{RUNBOOK_DIR}<file>.md#<anchor>"
                )
                continue
            doc_rel, anchor = url[len(RUNBOOK_PREFIX):].split("#", 1)
            doc = root / doc_rel
            if not doc.is_file():
                errors.append(f"{where}: runbook file {doc_rel} does not exist")
                continue
            sections = section_cache.setdefault(doc, runbook_sections(doc))
            section = sections.get(anchor)
            if section is None:
                errors.append(f"{where}: no '## ' heading in {doc_rel} produces anchor #{anchor}")
                continue
            missing = [field for field in REQUIRED_FIELDS if field not in section]
            if missing:
                errors.append(f"{where}: runbook section {doc_rel}#{anchor} is missing {missing}")
            if owner and f"**Owner:** {owner}" not in section:
                errors.append(f"{where}: runbook section does not name owner {owner!r}")
            if severity and f"**Severity:** {severity}" not in section:
                errors.append(f"{where}: runbook section does not state severity {severity!r}")

    if errors:
        print(f"Alert runbook check failed ({len(errors)} problem(s)):", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return 1

    print(f"OK: {total} alert rule(s) across {len(ALERT_FILES)} file(s) have owners and runbooks.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
