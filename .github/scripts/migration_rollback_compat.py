#!/usr/bin/env python3
"""Check that adjacent application releases work against the migrated schema.

Rolling the application back after a migration means the *previous* release
runs against the *new* schema. This script reproduces that situation:

  1. Applies every migration from --new-ref to an empty database, in filename
     order, with psql (ON_ERROR_STOP).
  2. Collects the SQL each release sends — the sqlx offline cache
     (backend/.sqlx/*.json) plus string literals passed to sqlx::query*,
     query_as* and query_scalar* in backend/src (test modules excluded) — from
     --old-ref (previous release) and --new-ref (this release).
  3. PREPAREs every statement. PREPARE plans the query against the catalog
     without running it, so a missing table or column, or an incompatible
     type, fails here instead of in production after a rollback.

Statements PostgreSQL cannot PREPARE on its own (DDL, untyped parameters) are
reported as skipped, not failed. Dynamic SQL built at runtime is not covered.

Requires `git` and `psql`. Connection settings come from --database-url.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import PurePosixPath

MIGRATIONS_DIR = "backend/migrations"
SQLX_CACHE_DIR = "backend/.sqlx"
SOURCE_DIR = "backend/src"

# SQLSTATEs that mean "this release's query does not fit the schema".
INCOMPATIBLE_STATES = {
    "42P01": "undefined table",
    "42703": "undefined column",
    "42883": "undefined function or operator",
    "42704": "undefined object/type",
    "42804": "datatype mismatch",
    "42P10": "invalid column reference (e.g. ON CONFLICT target without matching unique index)",
    "428C9": "write to generated column",
}
PREPARABLE = re.compile(r"^\s*(?:WITH|SELECT|INSERT|UPDATE|DELETE|VALUES|MERGE)\b", re.I)
QUERY_CALL = re.compile(
    r"\b(?:sqlx::)?query(?:_as|_scalar)?(?:_with)?(?:::<[^>]*>)?\s*\(\s*"
    r"(?:r(?P<hashes>#*)\"(?P<raw>.*?)\"(?P=hashes)|\"(?P<plain>(?:[^\"\\]|\\.)*)\")",
    re.S,
)
TEST_MODULE = re.compile(r"#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+")


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def list_files(ref: str, directory: str) -> list[str]:
    out = git("ls-tree", "-r", "--name-only", ref, "--", directory)
    return [line for line in out.splitlines() if line]


def show(ref: str, path: str) -> str:
    return git("show", f"{ref}:{path}")


def is_test_file(path: str) -> bool:
    parts = PurePosixPath(path).parts
    name = PurePosixPath(path).name
    return (
        "test_support" in parts
        or name.endswith("_tests.rs")
        or name in {"tests.rs", "test_support.rs", "mock_rpc_helpers.rs"}
    )


def unescape_rust(text: str) -> str:
    # Handle the escapes that appear in SQL literals; drop line continuations.
    text = re.sub(r"\\\n\s*", "", text)
    return text.replace('\\"', '"').replace("\\n", "\n").replace("\\t", "\t").replace("\\\\", "\\")


def collect_queries(ref: str) -> dict[str, str]:
    """Return {statement: origin} for every static SQL statement at ref."""
    queries: dict[str, str] = {}
    for path in list_files(ref, SQLX_CACHE_DIR):
        if path.endswith(".json"):
            try:
                query = json.loads(show(ref, path)).get("query", "")
            except json.JSONDecodeError:
                continue
            if query.strip():
                queries.setdefault(query.strip().rstrip(";"), path)
    for path in list_files(ref, SOURCE_DIR):
        if not path.endswith(".rs") or is_test_file(path):
            continue
        source = show(ref, path)
        test_start = TEST_MODULE.search(source)
        if test_start:
            source = source[: test_start.start()]
        for match in QUERY_CALL.finditer(source):
            text = match.group("raw") if match.group("raw") is not None else unescape_rust(match.group("plain"))
            text = text.strip().rstrip(";")
            if text:
                line = source.count("\n", 0, match.start()) + 1
                queries.setdefault(text, f"{path}:{line}")
    return queries


def psql(database_url: str, sql: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["psql", database_url, "--no-psqlrc", "--quiet", "-v", "ON_ERROR_STOP=1",
         "-v", "VERBOSITY=verbose", "-c", sql],
        capture_output=True,
        text=True,
    )


def apply_migrations(database_url: str, ref: str) -> None:
    files = sorted(p for p in list_files(ref, MIGRATIONS_DIR) if p.endswith(".sql"))
    print(f"Applying {len(files)} migration(s) from {ref}")
    for path in files:
        result = subprocess.run(
            ["psql", database_url, "--no-psqlrc", "--quiet", "-v", "ON_ERROR_STOP=1", "-f", "-"],
            input=show(ref, path),
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            print(f"::error::migration {path} failed to apply\n{result.stderr}", file=sys.stderr)
            sys.exit(1)
        print(f"  applied {path}")


def prepare_all(database_url: str, label: str, queries: dict[str, str]) -> tuple[list[str], int, int]:
    failures: list[str] = []
    checked = skipped = 0
    for index, (sql, origin) in enumerate(sorted(queries.items(), key=lambda kv: kv[1])):
        if not PREPARABLE.match(sql):
            skipped += 1
            continue
        name = f"compat_{index}"
        result = psql(database_url, f"PREPARE {name} AS {sql};\nDEALLOCATE {name};")
        if result.returncode == 0:
            checked += 1
            continue
        state = re.search(r"ERROR:\s+([0-9A-Z]{5}):", result.stderr)
        code = state.group(1) if state else ""
        if code in INCOMPATIBLE_STATES:
            message = result.stderr.strip().splitlines()[0]
            failures.append(f"[{label}] {origin}: {INCOMPATIBLE_STATES[code]} — {message}")
        else:
            skipped += 1
            print(f"  skipped {origin} ({code or 'unknown'}): cannot be prepared standalone")
    return failures, checked, skipped


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--old-ref", required=True, help="previous release (rollback target)")
    parser.add_argument("--new-ref", default="HEAD", help="release that ships the migrations")
    parser.add_argument("--database-url", required=True, help="empty PostgreSQL database")
    args = parser.parse_args()

    apply_migrations(args.database_url, args.new_ref)

    all_failures: list[str] = []
    for label, ref in (("previous release", args.old_ref), ("new release", args.new_ref)):
        queries = collect_queries(ref)
        failures, checked, skipped = prepare_all(args.database_url, label, queries)
        print(f"{label} ({ref}): {checked} statement(s) prepared, {skipped} skipped, {len(failures)} incompatible")
        all_failures.extend(failures)

    if all_failures:
        print("\nSchema is not compatible with every adjacent release:", file=sys.stderr)
        for failure in all_failures:
            print(f"  - {failure}", file=sys.stderr)
        print(
            "\nSplit the change into expand / migrate / contract steps "
            "(docs/migrations/expand-migrate-contract.md).",
            file=sys.stderr,
        )
        return 1
    print("OK: previous and new releases both prepare cleanly against the migrated schema.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
