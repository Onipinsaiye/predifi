#!/usr/bin/env python3
"""Scoped mutation testing for PrediFi's security-critical logic (issue #128).

Single source of truth for *what* is mutated and *which tests* must kill it.
CI (`.github/workflows/mutation-testing.yml`) and local runs both go through
this script so results are repeatable.

    python mutation-testing/run.py --list                 # show mutants, run nothing
    python mutation-testing/run.py                        # full baseline, all suites
    python mutation-testing/run.py --suite contract-math  # one suite
    python mutation-testing/run.py --diff-base origin/main  # only mutants on changed lines
    python mutation-testing/run.py --shard 0/4            # CI sharding
    python mutation-testing/run.py --report-only          # re-evaluate existing results

Gate: every mutant that is *missed* (tests still pass) or *unviable* is fine
only if it is listed in `survivors.toml` with a reviewed disposition. Any
unlisted missed mutant fails the run. Requires cargo-mutants (see README.md).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HERE = ROOT / "mutation-testing"
DEFAULT_OUT = HERE / "out"
SURVIVORS = HERE / "survivors.toml"

# Pin the tool so mutant generation is identical everywhere (see README.md).
CARGO_MUTANTS_VERSION = "27.1.0"


@dataclass
class Suite:
    name: str
    workspace: str  # relative to repo root
    files: list[str]  # relative to the workspace
    # Libtest name filters (OR-ed). Empty = run every lib test of the package.
    test_filters: list[str] = field(default_factory=list)
    examine_re: list[str] = field(default_factory=list)
    exclude_re: list[str] = field(default_factory=list)
    # Passed to both the build (`cargo test --no-run`) and the test run, so
    # only the lib test harness is rebuilt per mutant (not cdylib/tests/*.rs).
    cargo_args: list[str] = field(default_factory=lambda: ["--lib"])


SUITES = [
    # Fee deduction, payout pool, proportional winnings, referral cut, odds.
    # Tests are restricted to the payout/safe-math modules: the full
    # predifi-contract suite takes ~10 min per run and is not needed to judge
    # pure arithmetic.
    Suite(
        name="contract-math",
        workspace="contract",
        files=[
            "contracts/predifi-contract/src/payouts.rs",
            "contracts/predifi-contract/src/safe_math.rs",
        ],
        test_filters=["payouts::", "safe_math::", "payout_proptests"],
        # PayoutRoundingAudit is a test/audit helper with no on-chain caller.
        exclude_re=["PayoutRoundingAudit"],
    ),
    # Authorization + pool-state guards inside the main contract: role checks
    # (cross-contract has_role), pause switch, fee-bps validation, and the
    # "pool is active" gate used by predict/resolve/cancel paths.
    Suite(
        name="contract-guards",
        workspace="contract",
        files=["contracts/predifi-contract/src/lib.rs"],
        examine_re=[
            r"PredifiContract::(has_role|require_[a-z_]+|is_paused|is_valid_fee_bps"
            r"|is_pool_active|is_valid_state_transition) "
        ],
        test_filters=[
            "mutation_kill_guards",
            "access_control_audit_tests",
            "pause_unpause_boundary_tests",
            "is_pool_active",
            "fee_bps",
            "unauthorized",
            "paused",
            "non_admin",
            "non_operator",
            "canceled_pool",
            "resolved_pool",
        ],
    ),
    # Role storage, assignment, revocation, transfer and admin hand-over.
    Suite(
        name="access-control",
        workspace="contract",
        files=["contracts/access-control/src/lib.rs"],
    ),
    # Backend authorization boundary: JWT signature/alg/exp/type/key-version
    # checks and bearer-header parsing.
    Suite(
        name="backend-jwt",
        workspace="backend",
        files=["src/jwt.rs"],
        test_filters=["jwt::"],
    ),
]
SUITES_BY_NAME = {s.name: s for s in SUITES}

# Outcome summaries emitted by cargo-mutants in outcomes.json.
CAUGHT, MISSED, TIMEOUT, UNVIABLE = "CaughtMutant", "MissedMutant", "Timeout", "Unviable"

LINE_COL = re.compile(r"^(?P<file>[^:]+):\d+:\d+: (?P<desc>.*)$")


def mutant_key(name: str) -> str:
    """`path:line:col: description` -> `path: description` (stable across edits)."""
    m = LINE_COL.match(name)
    return f"{m['file']}: {m['desc']}" if m else name


def original_line(diff: str) -> str:
    """The unmutated source line, from a mutant's diff."""
    for line in diff.splitlines():
        if line.startswith("-") and not line.startswith("---"):
            return line[1:].strip()
    return ""


def cargo_mutants_args(
    suite: Suite, args: argparse.Namespace, out_dir: Path, diff: Path | None
) -> list[str]:
    # No --cap-lints: it works by setting RUSTFLAGS, which would override
    # target rustflags from .cargo/config.toml. No crate uses #![deny].
    cmd = ["cargo", "mutants", "--no-shuffle"]
    cmd += ["--timeout-multiplier", "3", "--minimum-test-timeout", "60"]
    for a in suite.cargo_args:
        cmd.append(f"--cargo-arg={a}")
    for f in suite.files:
        cmd += ["--file", f]
    for r in suite.examine_re:
        cmd += ["--re", r]
    for r in suite.exclude_re:
        cmd += ["--exclude-re", r]
    if args.list:
        cmd.append("--list")
    else:
        cmd += ["--output", str(out_dir), "--jobs", str(args.jobs)]
        if args.shard:
            cmd += ["--shard", args.shard]
        if diff:
            cmd += ["--in-diff", str(diff)]
    if suite.test_filters:
        cmd += ["--", "--", *suite.test_filters]
    return cmd


def check_tool() -> None:
    try:
        out = subprocess.run(
            ["cargo", "mutants", "--version"], capture_output=True, text=True, check=True
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        sys.exit(
            "cargo-mutants not found. Install with:\n"
            f"  cargo install cargo-mutants --locked --version {CARGO_MUTANTS_VERSION}"
        )
    if CARGO_MUTANTS_VERSION not in out:
        print(f"warning: expected cargo-mutants {CARGO_MUTANTS_VERSION}, found '{out}'")


def suite_diff(suite: Suite, base: str, out_dir: Path) -> Path | None:
    """Diff of the suite's files vs `base`, with paths relative to its workspace."""
    diff = subprocess.run(
        ["git", "diff", "--unified=0", f"--relative={suite.workspace}", f"{base}...HEAD",
         "--", *suite.files],
        cwd=ROOT / suite.workspace, capture_output=True, text=True, check=True,
    ).stdout
    if not diff.strip():
        return None
    path = out_dir / "changes.diff"
    path.write_text(diff, encoding="utf-8")
    return path


def run_suite(suite: Suite, args: argparse.Namespace) -> bool:
    """Run one suite; returns False if it was skipped (no changed lines)."""
    out_dir = Path(args.out).resolve() / suite.name
    if not args.list:
        shutil.rmtree(out_dir, ignore_errors=True)
        out_dir.mkdir(parents=True, exist_ok=True)
    diff = None
    if args.diff_base:
        diff = suite_diff(suite, args.diff_base, out_dir)
        if diff is None:
            print(f"\n=== {suite.name} === skipped: no scoped lines changed vs {args.diff_base}")
            (out_dir / "SKIPPED").touch()
            return False
    cmd = cargo_mutants_args(suite, args, out_dir, diff)
    print(f"\n=== {suite.name} ===\n$ (cd {suite.workspace} && {' '.join(cmd)})", flush=True)
    # Debuginfo dominates link time for the soroban test binaries and is not
    # needed to tell whether a test failed.
    env = {**os.environ, "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_TEST_DEBUG": "0"}
    rc = subprocess.run(cmd, cwd=ROOT / suite.workspace, env=env).returncode
    # 0 = all caught, 2 = some missed, 3 = some timed out: judged by the gate
    # below. Anything else (1 usage/parse error, 4 baseline tests failing)
    # fails the suite, but the remaining suites still run.
    if rc not in (0, 2, 3):
        print(f"cargo-mutants failed for suite '{suite.name}' (exit {rc})", flush=True)
        if not args.list:
            (out_dir / "FAILED").write_text(str(rc), encoding="utf-8")
    return True


def load_survivors() -> list[dict]:
    data = tomllib.loads(SURVIVORS.read_text(encoding="utf-8"))
    entries = []
    for s in data.get("survivor", []):
        for k in ("suite", "mutant", "line_contains", "disposition", "rationale"):
            if not s.get(k):
                sys.exit(f"survivors.toml: entry {s!r} is missing '{k}'")
        if s["disposition"] not in ("equivalent", "unreachable", "accepted-risk"):
            sys.exit(f"survivors.toml: bad disposition {s['disposition']!r}")
        entries.append(s)
    return entries


def find_survivor(survivors: list[dict], suite: str, key: str, line: str) -> int | None:
    for i, s in enumerate(survivors):
        if s["suite"] == suite and s["mutant"] == key and s["line_contains"] in line:
            return i
    return None


def report(suites: list[Suite], args: argparse.Namespace) -> int:
    survivors = load_survivors()
    rows, problems, explained = [], [], set()
    for suite in suites:
        suite_out = Path(args.out) / suite.name
        if (suite_out / "SKIPPED").exists():
            rows.append((suite.name, "skipped (unchanged)", "", "", "", "", ""))
            continue
        if (suite_out / "FAILED").exists():
            rows.append((suite.name, "did not run", "", "", "", "", ""))
            problems.append(f"{suite.name}: cargo-mutants failed (unmutated build/tests broken?)"
                            f" - see {suite_out / 'mutants.out' / 'log' / 'baseline.log'}")
            continue
        outcomes_file = suite_out / "mutants.out" / "outcomes.json"
        if not outcomes_file.exists():
            problems.append(f"{suite.name}: no results ({outcomes_file} missing)")
            continue
        outcomes = json.loads(outcomes_file.read_text(encoding="utf-8"))["outcomes"]
        # outcomes.json omits the diff; mutants.json has it.
        diffs = {
            m["name"]: m.get("diff", "")
            for m in json.loads((outcomes_file.parent / "mutants.json").read_text(encoding="utf-8"))
        }
        counts = {CAUGHT: 0, MISSED: 0, TIMEOUT: 0, UNVIABLE: 0}
        for o in outcomes:
            scenario = o["scenario"]
            if not isinstance(scenario, dict):  # the unmutated baseline build/test
                continue
            summary = o["summary"]
            counts[summary] = counts.get(summary, 0) + 1
            if summary == MISSED:
                mutant = scenario["Mutant"]
                key = mutant_key(mutant["name"])
                line = original_line(diffs.get(mutant["name"], ""))
                idx = find_survivor(survivors, suite.name, key, line)
                if idx is None:
                    problems.append(f"{suite.name}: UNREVIEWED SURVIVOR `{key}` on `{line}`")
                else:
                    explained.add(idx)
        detected = counts[CAUGHT] + counts[TIMEOUT]
        viable = detected + counts[MISSED]
        score = f"{100 * detected / viable:.1f}%" if viable else "n/a"
        rows.append((suite.name, viable, counts[CAUGHT], counts[TIMEOUT], counts[MISSED],
                     counts[UNVIABLE], score))

    # Only meaningful for complete runs: a diff/shard run sees a subset.
    full_run = not (args.diff_base or args.shard) and not args.report_only
    ran = {s.name for s in suites}
    stale = [
        f"{s['suite']}: `{s['mutant']}` on `{s['line_contains']}`"
        for i, s in enumerate(survivors)
        if full_run and s["suite"] in ran and i not in explained
    ]

    lines = [
        "### Mutation testing (issue #128)",
        "",
        "| Suite | Viable | Caught | Timeout | Missed | Unviable | Score |",
        "|---|---:|---:|---:|---:|---:|---:|",
        *(f"| {' | '.join(map(str, r))} |" for r in rows),
        "",
        f"Reviewed survivors matched: {len(explained)} (see `mutation-testing/survivors.toml`).",
    ]
    if stale:
        lines += ["", "Stale survivors.toml entries (mutant now caught or gone; remove them):",
                  *(f"- {x}" for x in stale)]
    if problems:
        lines += ["", "**Gate failed:**", *(f"- {p}" for p in problems), "",
                  "Kill each survivor with a test, or document it in survivors.toml."]
    else:
        lines += ["", "Gate passed: every surviving mutant has a reviewed disposition."]
    text = "\n".join(lines)
    print("\n" + text)
    if summary_path := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary_path, "a", encoding="utf-8") as fh:
            fh.write(text + "\n")
    return 1 if problems else 0


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    p.add_argument("--suite", action="append", choices=list(SUITES_BY_NAME),
                   help="suite(s) to run (default: all)")
    p.add_argument("--list", action="store_true", help="list mutants without testing")
    p.add_argument("--diff-base", help="only mutate lines changed vs this git ref (PR mode)")
    p.add_argument("--shard", help="k/n: run only shard k of n")
    p.add_argument("--jobs", type=int, default=2, help="parallel cargo-mutants jobs")
    p.add_argument("--out", default=str(DEFAULT_OUT), help="results directory")
    p.add_argument("--report-only", action="store_true", help="skip running; gate existing results")
    args = p.parse_args()

    suites = [SUITES_BY_NAME[n] for n in args.suite] if args.suite else SUITES
    if not args.report_only:
        check_tool()
        for suite in suites:
            run_suite(suite, args)
    if args.list:
        return 0
    return report(suites, args)


if __name__ == "__main__":
    sys.exit(main())
