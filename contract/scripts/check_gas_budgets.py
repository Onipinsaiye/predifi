#!/usr/bin/env python3
"""Compare per-entry-point Soroban resource measurements against budgets (#42).

Reads `GAS_BUDGET {json}` lines printed by the `gas_budget_tests` module of
predifi-contract and checks them against `contract/gas-budgets.json`:

- a metric at or above its ceiling            -> FAIL (exit 1)
- a budgeted entry point with no measurement  -> FAIL (exit 1)
- a metric at or above alert_threshold_pct     -> WARN
- a metric above its recorded baseline         -> reported as a regression

Usage:
    cargo test -p predifi-contract --lib gas_budget_tests -- --nocapture --test-threads=1 \
        | python3 scripts/check_gas_budgets.py [--budgets gas-budgets.json] [--record]

`--record` writes the measurements into each entry point's `baseline` instead
of checking them.
"""

import argparse
import json
import os
import re
import subprocess
import sys

LINE_RE = re.compile(r"GAS_BUDGET (\{.*\})")


def parse_measurements(text):
    measurements = {}
    for match in LINE_RE.finditer(text):
        record = json.loads(match.group(1))
        measurements[record.pop("entry_point")] = record
    return measurements


def toolchain_versions(lockfile):
    try:
        rustc = subprocess.run(
            ["rustc", "--version"], capture_output=True, text=True, check=True
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        rustc = "rustc unknown"
    sdk = "unknown"
    if os.path.exists(lockfile):
        with open(lockfile, encoding="utf-8") as f:
            found = re.search(r'name = "soroban-sdk"\nversion = "([^"]+)"', f.read())
            if found:
                sdk = found.group(1)
    return f"{rustc}, soroban-sdk {sdk}"


def check(budgets, measurements):
    """Return (rows, failures, warnings); rows are deterministic table rows."""
    threshold = budgets["alert_threshold_pct"]
    rows, failures, warnings = [], [], []
    for entry, spec in sorted(budgets["entry_points"].items()):
        measured = measurements.get(entry)
        if measured is None:
            failures.append(f"{entry}: no measurement (gas_budget_tests missing?)")
            continue
        baseline = spec.get("baseline") or {}
        for metric in budgets["metrics"]:
            value = measured[metric]
            ceiling = spec["ceiling"][metric]
            pct = 100.0 * value / ceiling
            base = baseline.get(metric)
            delta = "" if base is None else f"{value - base:+d}"
            if value >= ceiling:
                status = "FAIL"
                failures.append(f"{entry}.{metric}: {value} >= ceiling {ceiling}")
            elif pct >= threshold:
                status = "WARN"
                warnings.append(f"{entry}.{metric}: {value} is {pct:.1f}% of ceiling {ceiling}")
            elif base is not None and value > base:
                status = "regressed"
            else:
                status = "ok"
            rows.append((entry, metric, value, base, delta, ceiling, f"{pct:.2f}%", status))
    for entry in sorted(set(measurements) - set(budgets["entry_points"])):
        warnings.append(f"{entry}: measured but has no budget")
    return rows, failures, warnings


def render(rows, toolchain):
    lines = [
        "## Soroban gas budgets",
        "",
        f"Toolchain: `{toolchain}`",
        "",
        "| entry point | metric | measured | baseline | delta | ceiling | % of ceiling | status |",
        "|---|---|---:|---:|---:|---:|---:|---|",
    ]
    for entry, metric, value, base, delta, ceiling, pct, status in rows:
        base = "-" if base is None else base
        lines.append(f"| {entry} | {metric} | {value} | {base} | {delta} | {ceiling} | {pct} | {status} |")
    return "\n".join(lines) + "\n"


def main(argv=None):
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--budgets", default=os.path.join(root, "gas-budgets.json"))
    parser.add_argument("--input", help="test output file (default: stdin)")
    parser.add_argument("--record", action="store_true", help="write measurements as baselines")
    args = parser.parse_args(argv)

    text = open(args.input, encoding="utf-8").read() if args.input else sys.stdin.read()
    measurements = parse_measurements(text)
    with open(args.budgets, encoding="utf-8") as f:
        budgets = json.load(f)

    if args.record:
        for entry, spec in budgets["entry_points"].items():
            if entry in measurements:
                spec["baseline"] = {m: measurements[entry][m] for m in budgets["metrics"]}
        with open(args.budgets, "w", encoding="utf-8") as f:
            json.dump(budgets, f, indent=2)
            f.write("\n")
        print(f"Recorded baselines for {len(measurements)} entry points in {args.budgets}")
        return 0

    rows, failures, warnings = check(budgets, measurements)
    report = render(rows, toolchain_versions(os.path.join(root, "Cargo.lock")))
    print(report)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write(report)
    for message in warnings:
        print(f"::warning::gas budget alert: {message}")
    for message in failures:
        print(f"::error::gas budget exceeded: {message}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
