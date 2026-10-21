#!/usr/bin/env python3
"""Block migrations that would break rollback to the previous application release.

Policy: docs/migrations/expand-migrate-contract.md

Checks, for migrations newer than POLICY_BASELINE (older files predate the policy):
  1. Every migration declares a phase header:
         -- migration-phase: expand | migrate | contract
         -- compatible-app-versions: <which releases work with this schema>
  2. Statements that break the previous release (drops, renames, type changes,
     new NOT NULL constraints, deletes) are only allowed in a `contract`
     migration.
  3. A `contract` migration names the expand migration it finishes
     (`-- contracts: <file>.sql`). That file must already exist on the base
     branch, so the expand step shipped in an earlier release.
  4. A `contract` migration links an approved recovery plan
     (`-- recovery-plan: docs/migrations/recovery/<file>.md`) that has every
     required section and a non-empty `Approved-by:` line.

With --base, it also checks the pull request's diff:
  5. Already-merged migration files are never edited, renamed, or deleted.
  6. New files use a fresh numeric prefix higher than every existing one.

Exits 1 with a list of problems, 0 otherwise. Standard library only.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

MIGRATIONS_DIR = "backend/migrations"
RECOVERY_DIR = "docs/migrations/recovery"
# Highest migration version that existed before this policy. Files at or below
# it are grandfathered: their history is documented in
# docs/MIGRATION_DEPLOYMENT_GUIDE.md and they must not be edited.
POLICY_BASELINE = 13

PHASES = {"expand", "migrate", "contract"}
RECOVERY_SECTIONS = (
    "## Change",
    "## Compatible application versions",
    "## Pre-flight",
    "## Recovery",
    "## Verification",
)

# (pattern, explanation). Patterns run against SQL with comments and string
# literals removed, so a word inside a comment does not trigger them.
INCOMPATIBLE = [
    (r"\bDROP\s+TABLE\b", "DROP TABLE removes data the previous release reads"),
    (r"\bDROP\s+COLUMN\b", "DROP COLUMN breaks queries in the previous release"),
    (r"\bDROP\s+SCHEMA\b", "DROP SCHEMA removes objects the previous release uses"),
    (r"\bDROP\s+(?:VIEW|MATERIALIZED\s+VIEW|FUNCTION|TYPE)\b", "dropping a view/function/type the previous release may use"),
    (r"\bTRUNCATE\b", "TRUNCATE deletes data irreversibly"),
    (r"\bDELETE\s+FROM\b", "DELETE removes data irreversibly"),
    (r"\bRENAME\s+(?:COLUMN\s+)?\w+\s+TO\b|\bRENAME\s+TO\b", "renaming breaks queries in the previous release"),
    (r"\bALTER\s+(?:COLUMN\s+)?\w+\s+(?:SET\s+DATA\s+)?TYPE\b", "changing a column type can break the previous release's reads and writes"),
    (r"\bALTER\s+(?:COLUMN\s+)?\w+\s+SET\s+NOT\s+NULL\b", "SET NOT NULL rejects inserts from a release that does not write the column"),
    (r"\bALTER\s+(?:COLUMN\s+)?\w+\s+DROP\s+DEFAULT\b", "DROP DEFAULT breaks inserts from a release that relies on the default"),
]
# Clause boundaries inside an ALTER TABLE statement, used to find
# "ADD COLUMN ... NOT NULL" without a DEFAULT (the previous release's INSERTs fail).
ALTER_TABLE = re.compile(r"\bALTER\s+TABLE\b", re.I)
CLAUSE_SPLIT = re.compile(r"\b(?=ADD\b|ALTER\b|DROP\b)", re.I)
NOT_A_COLUMN = re.compile(r"^ADD\s+(?:CONSTRAINT|PRIMARY|UNIQUE|FOREIGN|CHECK|EXCLUDE)\b", re.I)

HEADER = re.compile(r"^\s*--\s*([a-z-]+)\s*:\s*(.+?)\s*$", re.I | re.M)
VERSION = re.compile(r"^(\d+)_.+\.sql$")


def strip_sql_noise(sql: str) -> str:
    sql = re.sub(r"/\*.*?\*/", " ", sql, flags=re.S)
    sql = re.sub(r"--[^\n]*", " ", sql)
    sql = re.sub(r"\$(\w*)\$.*?\$\1\$", " ", sql, flags=re.S)  # dollar-quoted bodies
    sql = re.sub(r"'(?:[^']|'')*'", "''", sql)
    return sql


def headers(sql: str) -> dict[str, str]:
    found: dict[str, str] = {}
    for key, value in HEADER.findall(sql):
        found.setdefault(key.lower(), value)
    return found


def version_of(name: str) -> int | None:
    match = VERSION.match(name)
    return int(match.group(1)) if match else None


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def exists_at(ref: str, path: str) -> bool:
    return subprocess.run(
        ["git", "cat-file", "-e", f"{ref}:{path}"], capture_output=True
    ).returncode == 0


def incompatible_statements(sql: str) -> list[str]:
    clean = strip_sql_noise(sql)
    problems = [why for pattern, why in INCOMPATIBLE if re.search(pattern, clean, re.I)]
    for statement in clean.split(";"):
        if not ALTER_TABLE.search(statement):
            continue
        for clause in CLAUSE_SPLIT.split(statement):
            clause = clause.strip()
            if not clause.upper().startswith("ADD") or NOT_A_COLUMN.match(clause):
                continue
            if re.search(r"\bNOT\s+NULL\b", clause, re.I) and not re.search(r"\bDEFAULT\b", clause, re.I):
                problems.append(
                    "ADD COLUMN ... NOT NULL without DEFAULT rejects inserts from the previous release"
                )
    return problems


def check_recovery_plan(root: Path, rel: str, where: str, errors: list[str]) -> None:
    if not rel.startswith(RECOVERY_DIR + "/") or not rel.endswith(".md"):
        errors.append(f"{where}: recovery-plan must be a file under {RECOVERY_DIR}/")
        return
    path = root / rel
    if not path.is_file():
        errors.append(f"{where}: recovery plan {rel} does not exist")
        return
    text = path.read_text(encoding="utf-8")
    missing = [section for section in RECOVERY_SECTIONS if section not in text]
    if missing:
        errors.append(f"{rel}: missing sections {missing}")
    approved = re.search(r"^Approved-by:\s*(\S.*)$", text, re.M)
    if not approved or "<" in approved.group(1):
        errors.append(
            f"{rel}: needs an 'Approved-by: @handle (date)' line filled in by a reviewer "
            "from the owning team before this migration can merge"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default=".", help="repository root")
    parser.add_argument("--base", help="git ref of the target branch (enables diff checks 3, 5, 6)")
    args = parser.parse_args()
    root = Path(args.root)
    directory = root / MIGRATIONS_DIR
    errors: list[str] = []

    files = sorted(p.name for p in directory.glob("*.sql"))
    for name in files:
        if version_of(name) is None:
            errors.append(f"{MIGRATIONS_DIR}/{name}: name must be <NNN>_<description>.sql")

    if args.base:
        diff = git("diff", "--name-status", "-M", f"{args.base}...HEAD", "--", MIGRATIONS_DIR)
        base_files = {
            Path(p).name
            for p in git("ls-tree", "-r", "--name-only", args.base, "--", MIGRATIONS_DIR).split()
            if p.endswith(".sql")
        }
        added: list[str] = []
        for line in diff.splitlines():
            parts = line.split("\t")
            status, paths = parts[0], parts[1:]
            if not paths[-1].endswith(".sql") and not paths[0].endswith(".sql"):
                continue
            if status == "A":
                added.append(Path(paths[0]).name)
            else:
                errors.append(
                    f"{paths[0]}: merged migrations are immutable (status {status}). "
                    "sqlx checksums applied files; add a new migration instead."
                )
        highest_base = max((version_of(n) or 0 for n in base_files), default=0)
        used = {version_of(n) for n in base_files}
        for name in added:
            version = version_of(name)
            if version is None:
                continue
            if version in used:
                errors.append(f"{MIGRATIONS_DIR}/{name}: prefix {version:03d} is already used")
            elif version <= highest_base:
                errors.append(
                    f"{MIGRATIONS_DIR}/{name}: prefix must be higher than the latest merged "
                    f"migration ({highest_base:03d})"
                )
            used.add(version)

    for name in files:
        version = version_of(name)
        if version is None or version <= POLICY_BASELINE:
            continue
        where = f"{MIGRATIONS_DIR}/{name}"
        sql = (directory / name).read_text(encoding="utf-8")
        meta = headers(sql)

        phase = meta.get("migration-phase", "").lower()
        if phase not in PHASES:
            errors.append(f"{where}: add '-- migration-phase: expand|migrate|contract'")
        if not meta.get("compatible-app-versions"):
            errors.append(
                f"{where}: add '-- compatible-app-versions: ...' naming the releases that work "
                "with the schema after this migration (e.g. 'previous and current')"
            )

        problems = incompatible_statements(sql)
        if problems and phase != "contract":
            for why in sorted(set(problems)):
                errors.append(f"{where}: {why}; only allowed in a 'contract' migration")

        if phase == "contract":
            target = meta.get("contracts", "")
            if not target:
                errors.append(f"{where}: add '-- contracts: <expand migration>.sql'")
            elif not (directory / target).is_file():
                errors.append(f"{where}: contracts target {target} does not exist")
            elif args.base and not exists_at(args.base, f"{MIGRATIONS_DIR}/{target}"):
                errors.append(
                    f"{where}: {target} is not on the base branch yet. Ship the expand "
                    "migration in an earlier release before contracting it."
                )
            plan = meta.get("recovery-plan", "")
            if not plan:
                errors.append(f"{where}: add '-- recovery-plan: {RECOVERY_DIR}/<file>.md'")
            else:
                check_recovery_plan(root, plan, where, errors)

    if errors:
        print(f"Migration safety check failed ({len(errors)} problem(s)):", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        print("See docs/migrations/expand-migrate-contract.md", file=sys.stderr)
        return 1
    print(f"OK: {len(files)} migration file(s) follow the expand-migrate-contract policy.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
