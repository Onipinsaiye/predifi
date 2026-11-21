#!/usr/bin/env python3
"""Validate localization catalogs for completeness and pluralization correctness.

This script enforces the localization policy for the project:

* A source catalog (English) defines the required keys.
* Every supported locale must provide every required key.
* Pluralized entries must use valid CLDR plural categories for their locale.
* Contract identifiers and numeric values must remain unlocalized (i.e. they
  must not be altered by translation placeholders).

The script exits non-zero when any violation is found so CI fails the build.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, Iterable, List, Mapping, Optional, Sequence, Set, Tuple

# ---------------------------------------------------------------------------
# Locale policy
# ---------------------------------------------------------------------------

SOURCE_LOCALE = "en"

# Supported locales and the CLDR plural categories each one requires.
# A locale that does not appear here is not supported and will be rejected.
SUPPORTED_LOCALES: Dict[str, Tuple[str, ...]] = {
    "en": ("one", "other"),
    "de": ("one", "other"),
    "fr": ("one", "many", "other"),
    "es": ("one", "many", "other"),
    "it": ("one", "many", "other"),
    "pt": ("one", "many", "other"),
    "nl": ("one", "other"),
    "pl": ("one", "few", "many", "other"),
    "ru": ("one", "few", "many", "other"),
    "uk": ("one", "few", "many", "other"),
    "cs": ("one", "few", "many", "other"),
    "sk": ("one", "few", "many", "other"),
    "ar": ("zero", "one", "two", "few", "many", "other"),
    "he": ("one", "two", "many", "other"),
    "ja": ("other",),
    "ko": ("other",),
    "zh": ("other",),
    "tr": ("one", "other"),
    "hi": ("one", "other"),
}

# Right-to-left locales; used for snapshot/RTL validation.
RTL_LOCALES: Set[str] = {"ar", "he"}

# Placeholder syntax used in catalog strings, e.g. "{count}", "{amount}".
PLACEHOLDER_RE = re.compile(r"\{([a-zA-Z_][a-zA-Z0-9_]*)\}")

# Contract identifiers and numeric values must remain unlocalized. These
# patterns identify values that must be preserved verbatim in translations.
CONTRACT_ID_RE = re.compile(r"\b0x[0-9a-fA-F]{40}\b")
NUMERIC_LITERAL_RE = re.compile(r"(?<![\w.])-?\d+(?:[.,]\d+)?(?![\w.])")

# Market lifecycle states that require a contextual explanation in the shared
# catalog. Each state must document what actions remain possible, the next
# expected transition, and the authoritative source for that transition.
LIFECYCLE_STATES: Tuple[str, ...] = (
    "draft",
    "scheduled",
    "open",
    "paused",
    "closed",
    "resolving",
    "resolved",
    "disputed",
    "settled",
    "cancelled",
    "expired",
)

# Required explanation fields for each lifecycle state.
LIFECYCLE_EXPLANATION_FIELDS: Tuple[str, ...] = (
    "state",
    "next_transition",
    "source",
)

# Catalog key prefix under which lifecycle explanations live.
LIFECYCLE_KEY_PREFIX = "market.lifecycle"
=======


@dataclass
class Violation:
    locale: str
    key: str
    message: str

    def __str__(self) -> str:  # pragma: no cover - trivial
        return f"[{self.locale}] {self.key}: {self.message}"


@dataclass
class ValidationResult:
    violations: List[Violation] = field(default_factory=list)
    checked_keys: int = 0
    checked_locales: int = 0

    @property
    def ok(self) -> bool:
        return not self.violations


# ---------------------------------------------------------------------------
# Catalog loading
# ---------------------------------------------------------------------------


def load_catalog(path: Path) -> Mapping[str, Any]:
    try:
        with path.open("r", encoding="utf-8") as handle:
            data = json.load(handle)
    except FileNotFoundError:
        raise SystemExit(f"error: catalog not found: {path}")
    except json.JSONDecodeError as exc:
        raise SystemExit(f"error: invalid JSON in {path}: {exc}")
    if not isinstance(data, dict):
        raise SystemExit(f"error: catalog root must be an object: {path}")
    return data


def flatten_catalog(
    data: Mapping[str, Any], prefix: str = ""
) -> Dict[str, Any]:
    """Flatten nested catalogs into dotted keys.

    Plural entries are represented as objects whose keys are CLDR plural
    categories (e.g. ``{"one": "...", "other": "..."}``). Those objects are
    kept intact under their dotted key so plural validation can inspect them.
    """
    flat: Dict[str, Any] = {}
    for key, value in data.items():
        dotted = f"{prefix}.{key}" if prefix else key
        if isinstance(value, dict) and not _is_plural_object(value):
            flat.update(flatten_catalog(value, dotted))
        else:
            flat[dotted] = value
    return flat


def _is_plural_object(value: Mapping[str, Any]) -> bool:
    if not value:
        return False
    return all(
        isinstance(k, str) and k in _ALL_PLURAL_CATEGORIES
        for k in value.keys()
    )


_ALL_PLURAL_CATEGORIES: Set[str] = {
    "zero",
    "one",
    "two",
    "few",
    "many",
    "other",
}


# ---------------------------------------------------------------------------
# Validation
# ---------------------------------------------------------------------------


def validate(
    source: Mapping[str, Any],
    locales: Mapping[str, Mapping[str, Any]],
) -> ValidationResult:
    result = ValidationResult()
    source_flat = flatten_catalog(source)
    result.checked_keys = len(source_flat)

    for locale, catalog in locales.items():
        if locale not in SUPPORTED_LOCALES:
            result.violations.append(
                Violation(locale, "<catalog>", "locale is not supported by policy")
            )
            continue
        result.checked_locales += 1
        flat = flatten_catalog(catalog)
        _check_missing_keys(locale, source_flat, flat, result)
        _check_extra_keys(locale, source_flat, flat, result)
        _check_plural_forms(locale, source_flat, flat, result)
        _check_placeholders(locale, source_flat, flat, result)
        _check_unlocalized_values(locale, source_flat, flat, result)
        _check_lifecycle_explanations(locale, source_flat, flat, result)

    return result


def _check_lifecycle_explanations(
    locale: str,
    source_flat: Mapping[str, Any],
    flat: Mapping[str, Any],
    result: ValidationResult,
) -> None:
    """Every known lifecycle state must carry a reviewed explanation.

    The source catalog defines the reviewed copy. Each supported locale must
    provide the same explanation keys so the copy stays in one shared
    localization catalog.
    """
    for state in LIFECYCLE_STATES:
        for field_name in LIFECYCLE_EXPLANATION_FIELDS:
            key = f"{LIFECYCLE_KEY_PREFIX}.{state}.{field_name}"
            if key not in source_flat:
                result.violations.append(
                    Violation(
                        SOURCE_LOCALE,
                        key,
                        "missing lifecycle explanation in source catalog",
                    )
                )
                continue
            if key not in flat:
                result.violations.append(
                    Violation(locale, key, "missing lifecycle explanation")
                )
                continue
            value = flat[key]
            if not isinstance(value, str):
                result.violations.append(
                    Violation(locale, key, "lifecycle explanation must be a string")
                )
            elif not value.strip():
                result.violations.append(
                    Violation(locale, key, "lifecycle explanation must not be empty")
                )


def _check_missing_keys(
    locale: str,
    source_flat: Mapping[str, Any],
    flat: Mapping[str, Any],
    result: ValidationResult,
) -> None:
    for key in source_flat:
        if key not in flat:
            result.violations.append(
                Violation(locale, key, "missing required key")
            )


def _check_extra_keys(
    locale: str,
    source_flat: Mapping[str, Any],
    flat: Mapping[str, Any],
    result: ValidationResult,
) -> None:
    for key in flat:
        if key not in source_flat:
            result.violations.append(
                Violation(locale, key, "key not present in source catalog")
            )


def _check_plural_forms(
    locale: str,
    source_flat: Mapping[str, Any],
    flat: Mapping[str, Any],
    result: ValidationResult,
) -> None:
    required = set(SUPPORTED_LOCALES[locale])
    for key, source_value in source_flat.items():
        if not _is_plural_object(source_value):
            continue
        value = flat.get(key)
        if value is None:
            continue  # already reported as missing
        if not isinstance(value, dict):
            result.violations.append(
                Violation(locale, key, "expected plural object, got scalar")
            )
            continue
        categories = set(value.keys())
        invalid = categories - _ALL_PLURAL_CATEGORIES
        if invalid:
            result.violations.append(
                Violation(
                    locale,
                    key,
                    f"invalid plural categories: {sorted(invalid)}",
                )
            )
        missing = required - categories
        if missing:
            result.violations.append(
                Violation(
                    locale,
                    key,
                    f"missing plural categories: {sorted(missing)}",
                )
            )
        for category, text in value.items():
            if not isinstance(text, str):
                result.violations.append(
                    Violation(
                        locale,
                        f"{key}.{category}",
                        "plural form must be a string",
                    )
                )
            elif not text.strip():
                result.violations.append(
                    Violation(
                        locale,
                        f"{key}.{category}",
                        "plural form must not be empty",
                    )
                )


def _check_placeholders(
    locale: str,
    source_flat: Mapping[str, Any],
    flat: Mapping[str, Any],
    result: ValidationResult,
) -> None:
    for key, source_value in source_flat.items():
        value = flat.get(key)
        if value is None:
            continue
        source_texts = _texts(source_value)
        target_texts = _texts(value)
        source_placeholders = _placeholders(source_texts)
        target_placeholders = _placeholders(target_texts)
        if source_placeholders != target_placeholders:
            result.violations.append(
                Violation(
                    locale,
                    key,
                    "placeholder mismatch: "
                    f"expected {sorted(source_placeholders)}, "
                    f"got {sorted(target_placeholders)}",
                )
            )


def _texts(value: Any) -> List[str]:
    if isinstance(value, str):
        return [value]
    if isinstance(value, dict):
        return [v for v in value.values() if isinstance(v, str)]
    return []


def _placeholders(texts: Iterable[str]) -> Set[str]:
    found: Set[str] = set()
    for text in texts:
        found.update(PLACEHOLDER_RE.findall(text))
    return found


def _check_unlocalized_values(
    locale: str,
    source_flat: Mapping[str, Any],
    flat: Mapping[str, Any],
    result: ValidationResult,
) -> None:
    """Contract identifiers and numeric literals must be preserved verbatim."""
    for key, source_value in source_flat.items():
        value = flat.get(key)
        if value is None:
            continue
        source_texts = _texts(source_value)
        target_texts = _texts(value)
        source_ids = _extract(source_texts, CONTRACT_ID_RE)
        target_ids = _extract(target_texts, CONTRACT_ID_RE)
        if source_ids != target_ids:
            result.violations.append(
                Violation(
                    locale,
                    key,
                    "contract identifier must remain unlocalized",
                )
            )
        source_numbers = _extract(source_texts, NUMERIC_LITERAL_RE)
        target_numbers = _extract(target_texts, NUMERIC_LITERAL_RE)
        if source_numbers != target_numbers:
            result.violations.append(
                Violation(
                    locale,
                    key,
                    "numeric value must remain unlocalized",
                )
            )


def _extract(texts: Iterable[str], pattern: re.Pattern[str]) -> List[str]:
    found: List[str] = []
    for text in texts:
        found.extend(pattern.findall(text))
    return found


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def discover_locales(catalog_dir: Path) -> Dict[str, Path]:
    locales: Dict[str, Path] = {}
    for path in sorted(catalog_dir.glob("*.json")):
        locales[path.stem] = path
    return locales


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--catalog-dir",
        type=Path,
        default=Path("frontend/src/locales"),
        help="directory containing <locale>.json catalogs",
    )
    parser.add_argument(
        "--source-locale",
        default=SOURCE_LOCALE,
        help="locale that defines the required key set",
    )
    args = parser.parse_args(argv)

    catalog_dir: Path = args.catalog_dir
    if not catalog_dir.is_dir():
        print(f"error: catalog directory not found: {catalog_dir}", file=sys.stderr)
        return 2

    locale_paths = discover_locales(catalog_dir)
    if args.source_locale not in locale_paths:
        print(
            f"error: source catalog {args.source_locale}.json not found in "
            f"{catalog_dir}",
            file=sys.stderr,
        )
        return 2

    source = load_catalog(locale_paths[args.source_locale])
    locales: Dict[str, Mapping[str, Any]] = {}
    for locale, path in locale_paths.items():
        if locale == args.source_locale:
            continue
        locales[locale] = load_catalog(path)

    result = validate(source, locales)

    if result.ok:
        print(
            f"localization OK: {result.checked_keys} keys across "
            f"{result.checked_locales} locales"
        )
        return 0

    print(
        f"localization validation failed: {len(result.violations)} violation(s)",
        file=sys.stderr,
    )
    for violation in result.violations:
        print(f"  {violation}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())