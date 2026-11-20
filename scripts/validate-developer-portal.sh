#!/usr/bin/env bash

set -euo pipefail

PORTAL_FILE="${1:-docs/DEVELOPER_PORTAL.md}"
if [[ ! -f "${PORTAL_FILE}" ]]; then
    echo "Error: developer portal not found: ${PORTAL_FILE}" >&2
    exit 1
fi

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python3 - "${PORTAL_FILE}" "${REPOSITORY_ROOT}" <<'PY'
from pathlib import Path
import re
import sys

portal = Path(sys.argv[1]).resolve()
repository = Path(sys.argv[2]).resolve()
text = portal.read_text(encoding="utf-8")
lines = text.splitlines()
errors = []
in_code = False

for line_number, line in enumerate(lines, start=1):
    if line.startswith("```"):
        if not in_code:
            preceding = lines[max(0, line_number - 3):line_number - 1]
            label = next((line for line in reversed(preceding) if line.strip()), "")
            if not label.startswith("**Example target:**"):
                errors.append(f"line {line_number}: code example needs a network/version label")
            elif not re.search(r"Testnet|network-independent|Configurable Stellar network", label, re.I):
                errors.append(f"line {line_number}: example label needs a network target")
            elif not re.search(r"\d+\.\d+\.\d+|Event Schema v\d+", label):
                errors.append(f"line {line_number}: example label needs a component/schema version")
        in_code = not in_code
        continue

    if in_code:
        continue

    for link in re.findall(r"\[[^\]]+\]\(([^)]+)\)", line):
        if link.startswith(("http://", "https://", "mailto:", "#")):
            continue
        target = link.split("#", 1)[0]
        if not target:
            continue
        resolved = (portal.parent / target).resolve()
        try:
            resolved.relative_to(repository)
        except ValueError:
            errors.append(f"line {line_number}: link points outside the repository: {link}")
            continue
        if not resolved.exists():
            errors.append(f"line {line_number}: local link target does not exist: {link}")

if in_code:
    errors.append("unclosed fenced code block")

for required in (
    "## Quick Navigation",
    "## Getting Started",
    "## Wallet Integration",
    "## API Integration",
    "## Testnet",
    "## Support & Community",
    "NEXT_PUBLIC_STELLAR_NETWORK",
    "quickstart.md",
    "Authentication",
    "Rate Limiting",
    "Event Schemas",
):
    if required not in text:
        errors.append(f"required setup content is missing: {required}")

if errors:
    print("Developer portal validation failed:")
    for error in errors:
        print(f"- {error}")
    sys.exit(1)

print("Developer portal validation passed (links, setup sections, and example labels).")
PY
