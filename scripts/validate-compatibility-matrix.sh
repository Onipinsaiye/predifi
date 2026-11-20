#!/usr/bin/env bash

set -euo pipefail

MATRIX_FILE="${1:-docs/INTEGRATION_COMPATIBILITY_MATRIX.md}"
if [[ ! -f "${MATRIX_FILE}" ]]; then
    echo "Error: compatibility matrix not found: ${MATRIX_FILE}" >&2
    exit 1
fi

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python3 - "${MATRIX_FILE}" "${REPOSITORY_ROOT}" <<'PY'
from pathlib import Path
import re
import sys
from urllib.parse import urldefrag

matrix = Path(sys.argv[1]).resolve()
repository = Path(sys.argv[2]).resolve()
lines = matrix.read_text(encoding="utf-8").splitlines()
headers = []
errors = []
checked = 0
found_evidence_table = False

def cells(line):
    return [cell.strip() for cell in line.strip().strip("|").split("|")]

def is_separator(row):
    return all(re.fullmatch(r":?-{3,}:?", cell.replace(" ", "")) for cell in row)

for line_number, line in enumerate(lines, start=1):
    if not line.startswith("|"):
        headers = []
        continue

    row = cells(line)
    if any("validation evidence" in cell.lower() for cell in row):
        found_evidence_table = True
        headers = row
        evidence_index = next(
            index for index, cell in enumerate(headers)
            if "validation evidence" in cell.lower()
        )
        status_index = next(
            (index for index, cell in enumerate(headers) if cell.lower() == "status"),
            None,
        )
        continue
    if not headers or is_separator(row):
        continue

    if len(row) <= evidence_index:
        errors.append(f"line {line_number}: row is missing its evidence column")
        continue

    status = row[status_index].lower() if status_index is not None and len(row) > status_index else ""
    if "planned" in status:
        continue

    checked += 1
    evidence = row[evidence_index]
    links = re.findall(r"\[[^\]]+\]\(([^)]+)\)", evidence)
    valid_reference = False
    for link in links:
        target, _ = urldefrag(link)
        if re.fullmatch(r"https://github\.com/[^/]+/[^/]+/releases/tag/[^/]+", target):
            valid_reference = True
            break
        if target.startswith(("https://", "http://")):
            continue

        evidence_path = (matrix.parent / target).resolve()
        try:
            evidence_path.relative_to(repository)
        except ValueError:
            continue
        if evidence_path.is_file() and (
            ".github/workflows" in evidence_path.as_posix()
            or "test" in evidence_path.name.lower()
            or "tests" in evidence_path.parts
        ):
            valid_reference = True
            break

    if not valid_reference:
        errors.append(
            f"line {line_number}: supported/deprecated entry must link to an existing test, "
            "workflow, or GitHub release"
        )

if not found_evidence_table:
    errors.append("no compatibility table with a Validation Evidence column was found")
elif checked == 0:
    errors.append("no supported or deprecated compatibility entries were checked")

if errors:
    print("Compatibility matrix validation failed:")
    for error in errors:
        print(f"- {error}")
    sys.exit(1)

print(f"Compatibility matrix validation passed ({checked} supported/deprecated entries).")
PY
