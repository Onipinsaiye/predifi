#!/usr/bin/env python3
"""Validate GitHub Actions path filters and branch conditions against the
cross-component dependency matrix in .github/ci/workflow-matrix.json.

Why this exists
---------------
Path filters are evaluated by GitHub before a workflow starts. If a shared
boundary (a path dependency, an OpenAPI schema, a build script) is missing from
a dependent workflow's ``paths`` list, its checks are silently skipped and a
broken change can merge. Reviewing YAML by eye does not catch this reliably.

This script loads the real workflow files, re-implements GitHub's path filter
semantics for the patterns used in this repository, and asserts:

1. every workflow triggers on the same integration branches for ``push`` and
   ``pull_request``;
2. every shared boundary sample schedules *exactly* its declared dependents
   (no missing dependents, no unrelated builds);
3. the named pull-request fixtures produce the expected trigger set, including
   docs-only changes producing no builds;
4. deployment gates remain scoped to their intended branch and event.

Exit status is non-zero on the first category of failure, after reporting all
failures found. Only PyYAML is required.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from functools import lru_cache
from pathlib import Path
from typing import Any, Iterable

try:
    import yaml
except ImportError:  # pragma: no cover - exercised by the CI workflow
    sys.exit(
        "error: PyYAML is required (python -m pip install pyyaml). "
        "The workflow-validation workflow installs it before running this script."
    )

ROOT = Path(__file__).resolve().parents[2]
MATRIX_FILE = ROOT / ".github" / "ci" / "workflow-matrix.json"

EVENTS = ("push", "pull_request")


# ---------------------------------------------------------------------------
# YAML helpers
# ---------------------------------------------------------------------------


def load_yaml(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as handle:
        data = yaml.safe_load(handle)
    if not isinstance(data, dict):
        raise ValueError(f"{path} does not contain a YAML mapping")
    return data


def on_config(workflow: dict[str, Any]) -> dict[str, Any]:
    """Return the ``on:`` mapping.

    PyYAML implements YAML 1.1, where an unquoted ``on`` key resolves to the
    boolean ``True``. Accept both spellings so the script does not depend on
    the loader used by the caller.
    """
    for key in ("on", True):
        if key in workflow:
            config = workflow[key]
            if not isinstance(config, dict):
                raise ValueError("workflow 'on:' is not a mapping")
            return config
    raise ValueError("workflow has no 'on:' mapping")


def as_list(value: Any) -> list[str]:
    if value is None:
        return []
    if isinstance(value, str):
        return [value]
    if isinstance(value, list):
        return [str(item) for item in value]
    raise ValueError(f"expected a string or list, got {type(value).__name__}")


# ---------------------------------------------------------------------------
# GitHub path filter semantics
# ---------------------------------------------------------------------------


@lru_cache(maxsize=None)
def compile_pattern(pattern: str) -> re.Pattern[str]:
    """Translate a GitHub Actions filter pattern into a regex.

    Supports ``*`` (any run of characters except ``/``), ``**`` (any run
    including ``/``), ``**/`` (zero or more directories), ``?`` (zero or one
    character) and character classes. Brace expansion is not supported by
    GitHub's filters, so braces are treated literally.

    This covers every pattern used by the workflows in this repository; the
    script fails loudly rather than silently mis-matching when it encounters
    syntax it does not understand.
    """
    out: list[str] = ["^"]
    index = 0
    while index < len(pattern):
        char = pattern[index]
        if char == "*":
            if index + 1 < len(pattern) and pattern[index + 1] == "*":
                index += 2
                if index < len(pattern) and pattern[index] == "/":
                    # ``**/`` also matches zero directories.
                    out.append("(?:.*/)?")
                    index += 1
                else:
                    out.append(".*")
            else:
                out.append("[^/]*")
                index += 1
        elif char == "?":
            # GitHub: zero or one of the preceding character. No pattern in
            # this repository uses it; an optional single character is the
            # closest safe interpretation.
            out.append("[^/]?")
            index += 1
        elif char == "[":
            end = pattern.find("]", index + 1)
            if end == -1:
                raise ValueError(f"unterminated character class in pattern {pattern!r}")
            out.append(pattern[index : end + 1])
            index = end + 1
        elif char in "{}":
            raise ValueError(
                f"brace expansion in {pattern!r} is not supported by GitHub "
                "Actions filters; express it as separate path entries"
            )
        else:
            out.append(re.escape(char))
            index += 1
    out.append("$")
    return re.compile("".join(out))


def path_matches(pattern: str, path: str) -> bool:
    return bool(compile_pattern(pattern).match(path))


def event_triggers(
    config: dict[str, Any],
    event: str,
    branch: str,
    changed: Iterable[str],
) -> bool:
    """Return True if GitHub would start the workflow for this event/branch/files."""
    event_config = config.get(event)
    if event_config is None:
        return False
    if not isinstance(event_config, dict):
        # ``push:`` with no filters means "all branches, all files".
        return True

    branches = as_list(event_config.get("branches"))
    if branches and branch not in branches:
        return False

    changed = list(changed)
    paths = as_list(event_config.get("paths"))
    paths_ignore = as_list(event_config.get("paths-ignore"))

    if paths and not any(
        path_matches(pattern, path) for pattern in paths for path in changed
    ):
        return False
    if paths_ignore and any(
        path_matches(pattern, path) for pattern in paths_ignore for path in changed
    ):
        return False
    return True


def path_filters_match(config: dict[str, Any], changed: Iterable[str]) -> bool:
    """Return True if the workflow's file filters admit ``changed``.

    Used for boundary coverage, which is about paths only; branch conditions are
    checked separately against every branch.
    """
    event_config = config.get("push")
    if not isinstance(event_config, dict):
        return True
    changed = list(changed)
    paths = as_list(event_config.get("paths"))
    paths_ignore = as_list(event_config.get("paths-ignore"))
    if paths and not any(
        path_matches(pattern, path) for pattern in paths for path in changed
    ):
        return False
    if paths_ignore and any(
        path_matches(pattern, path) for pattern in paths_ignore for path in changed
    ):
        return False
    return True


def triggered_workflows(
    workflows: dict[str, dict[str, Any]],
    event: str,
    branch: str,
    changed: Iterable[str],
) -> set[str]:
    return {
        name
        for name, workflow in workflows.items()
        if event_triggers(on_config(workflow), event, branch, changed)
    }


# ---------------------------------------------------------------------------
# Checks
# ---------------------------------------------------------------------------


class Report:
    def __init__(self) -> None:
        self.errors: list[str] = []
        self.notes: list[str] = []

    def error(self, message: str) -> None:
        self.errors.append(message)

    def ok(self, message: str) -> None:
        self.notes.append(message)

    @staticmethod
    def table(headers: list[str], rows: list[list[str]]) -> list[str]:
        widths = [len(header) for header in headers]
        for row in rows:
            widths = [max(width, len(cell)) for width, cell in zip(widths, row)]
        lines = [
            "  ".join(header.ljust(width) for header, width in zip(headers, widths)),
            "  ".join("-" * width for width in widths),
        ]
        for row in rows:
            lines.append(
                "  ".join(cell.ljust(width) for cell, width in zip(row, widths))
            )
        return lines


def check_branch_conditions(
    matrix: dict[str, Any],
    workflows: dict[str, dict[str, Any]],
    report: Report,
    branch: str,
) -> None:
    expected = set(matrix["branches"])
    for name, workflow in workflows.items():
        config = on_config(workflow)
        for event in EVENTS:
            event_config = config.get(event)
            if not isinstance(event_config, dict):
                report.error(
                    f"{name}: '{event}' has no branch filter; expected "
                    f"{sorted(expected)}"
                )
                continue
            actual = set(as_list(event_config.get("branches")))
            missing = expected - actual
            unexpected = actual - expected
            if missing:
                report.error(
                    f"{name}: '{event}' is missing branches {sorted(missing)}"
                )
            if unexpected:
                report.error(
                    f"{name}: '{event}' declares branches outside the matrix "
                    f"({sorted(unexpected)}); update .github/ci/workflow-matrix.json "
                    "if this is intentional"
                )
    report.ok(
        f"branch filters: {len(workflows)} workflows declare "
        f"{sorted(expected)} for {', '.join(EVENTS)}"
    )


def check_boundaries(
    matrix: dict[str, Any],
    workflows: dict[str, dict[str, Any]],
    report: Report,
) -> None:
    seen: set[str] = set()
    for boundary in matrix["boundaries"]:
        boundary_id = boundary["id"]
        if boundary_id in seen:
            report.error(f"duplicate boundary id {boundary_id!r} in workflow matrix")
            continue
        seen.add(boundary_id)

        expected = set(boundary["dependents"])
        unknown = expected - set(workflows)
        if unknown:
            report.error(
                f"boundary {boundary_id!r} references unknown workflows {sorted(unknown)}"
            )
            continue

        for sample in boundary["samples"]:
            # Path coverage only; branch conditions are checked separately.
            actual = {
                name
                for name, workflow in workflows.items()
                if path_filters_match(on_config(workflow), [sample])
            }
            if actual != expected:
                missing = expected - actual
                extra = actual - expected
                details = []
                if missing:
                    details.append(f"missing triggers for {sorted(missing)}")
                if extra:
                    details.append(f"unexpected triggers for {sorted(extra)}")
                report.error(
                    f"boundary {boundary_id!r} sample {sample!r}: "
                    + "; ".join(details)
                )
    report.ok(f"boundary coverage: {len(matrix['boundaries'])} boundaries verified")


def check_fixtures(
    matrix: dict[str, Any],
    workflows: dict[str, dict[str, Any]],
    report: Report,
    branch: str,
) -> list[list[str]]:
    order = list(workflows)
    rows: list[list[str]] = []
    for fixture in matrix["fixtures"]:
        changed = fixture["changed"]
        expected = set(fixture["expect"])
        unknown = expected - set(workflows)
        if unknown:
            report.error(
                f"fixture {fixture['id']!r} references unknown workflows "
                f"{sorted(unknown)}"
            )
            continue

        for event in EVENTS:
            actual = triggered_workflows(workflows, event, branch, changed)
            if actual != expected:
                missing = expected - actual
                extra = actual - expected
                details = []
                if missing:
                    details.append(f"did not trigger {sorted(missing)}")
                if extra:
                    details.append(f"unexpectedly triggered {sorted(extra)}")
                report.error(
                    f"fixture {fixture['id']!r} ({event}, branch {branch}): "
                    + "; ".join(details)
                )

        rows.append(
            [
                fixture["id"],
                *["yes" if name in expected else "-" for name in order],
            ]
        )
    report.ok(f"fixtures: {len(matrix['fixtures'])} scenarios verified on {branch}")
    return rows


def check_docs_only(
    matrix: dict[str, Any],
    workflows: dict[str, dict[str, Any]],
    report: Report,
) -> None:
    """Docs-only changes must never schedule a component workflow."""
    docs_changes = ["docs/faq.md", "README.md", "CONTRIBUTING.md"]
    for event in EVENTS:
        triggered = triggered_workflows(workflows, event, "main", docs_changes)
        if triggered:
            report.error(
                f"docs-only change would schedule {sorted(triggered)} on {event}; "
                "path filters must keep documentation out of build workflows"
            )
    report.ok("docs-only changes schedule no component workflows")


def check_deploy_gates(
    matrix: dict[str, Any],
    workflows: dict[str, dict[str, Any]],
    report: Report,
) -> None:
    for gate in matrix["deploy_gates"]:
        name = gate["workflow"]
        job_name = gate["job"]
        workflow = workflows.get(name)
        if workflow is None:
            report.error(f"deploy gate references unknown workflow {name!r}")
            continue
        jobs = workflow.get("jobs") or {}
        job = jobs.get(job_name)
        if job is None:
            report.error(f"{name}: deploy job {job_name!r} not found")
            continue
        condition = str(job.get("if", ""))
        if f"refs/heads/{gate['branch']}" not in condition:
            report.error(
                f"{name}: deploy job {job_name!r} is not scoped to branch "
                f"{gate['branch']!r} (if: {condition!r})"
            )
        if gate["event"] not in condition:
            report.error(
                f"{name}: deploy job {job_name!r} is not scoped to "
                f"'{gate['event']}' events (if: {condition!r})"
            )
    report.ok(f"deploy gates: {len(matrix['deploy_gates'])} gates verified")


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--branch",
        help="only evaluate this branch (the CI matrix runs one job per branch)",
    )
    args = parser.parse_args()

    with MATRIX_FILE.open(encoding="utf-8") as handle:
        matrix = json.load(handle)

    branches = matrix["branches"]
    if args.branch:
        if args.branch not in branches:
            print(
                f"error: branch {args.branch!r} is not in the matrix "
                f"({', '.join(branches)})",
                file=sys.stderr,
            )
            return 2
        branches = [args.branch]

    workflows: dict[str, dict[str, Any]] = {}
    for name, spec in matrix["workflows"].items():
        path = ROOT / spec["file"]
        try:
            workflows[name] = load_yaml(path)
        except (OSError, ValueError, yaml.YAMLError) as exc:
            print(f"error: cannot load {spec['file']}: {exc}", file=sys.stderr)
            return 2

    report = Report()
    print("Workflow trigger matrix validation")
    print(f"  matrix:   {MATRIX_FILE.relative_to(ROOT)}")
    print(f"  branches: {', '.join(branches)}")
    for name, spec in matrix["workflows"].items():
        print(f"  workflow: {name:<16} {spec['file']}")
    print()

    for branch in branches:
        print(f"[branch: {branch}]")
        branch_report = Report()
        check_branch_conditions(matrix, workflows, branch_report, branch)
        check_boundaries(matrix, workflows, branch_report)
        rows = check_fixtures(matrix, workflows, branch_report, branch)
        check_docs_only(matrix, workflows, branch_report)
        check_deploy_gates(matrix, workflows, branch_report)

        for note in branch_report.notes:
            print(f"  {note}")
        if rows:
            print()
            print("  fixture trigger matrix")
            for line in Report.table(["fixture", *workflows.keys()], rows):
                print(f"  {line}")
        print()
        report.errors.extend(branch_report.errors)

        if branch_report.errors:
            for error in branch_report.errors:
                print(f"  FAIL: {error}", file=sys.stderr)
            print(file=sys.stderr)

    if report.errors:
        print(
            f"FAILED: {len(report.errors)} workflow trigger problem(s). "
            "Fix .github/ci/workflow-matrix.json or the workflow filters.",
            file=sys.stderr,
        )
        return 1

    print("OK: every shared boundary schedules its dependents; docs-only changes build nothing.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
