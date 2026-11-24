# Localized Market Descriptions & Metadata Change History

> Owning package: `backend` (`backend/src/market_localization.rs`, `backend/src/market_metadata_history.rs`)  
> Issues: #246, #247

This document specifies the schema, invariants, authorization boundaries, and recovery flows for localized market descriptions (#246) and append-only market metadata change history (#247).

---

## 1. Localized Market Descriptions with a Canonical Question (#246)

### Authoritative Invariants

1. **Single immutable canonical question**:
   - Every `LocalizedMarketRecord` anchors on one `canonical_question`, one `canonical_locale`, and a domain-separated SHA-256 `canonical_question_digest` (`predifi-canonical-question-v1`).
   - Settlement, market identity, and API resolution responses always return `canonical_question` unchanged; localized entries provide translated descriptions and optional summaries only.
   - Pre-publish canonical question edits are restricted to the market creator or an admin and automatically mark existing non-canonical translations as stale (`is_stale_translation = true`) via digest mismatch until re-verified.
   - Once `publish` is called, `canonical_question` is permanently immutable (`LocalizationError::CanonicalQuestionImmutable`).
2. **Strict BCP-47 locale tag validation**:
   - `LocaleTag::parse` enforces ASCII hyphen-separated BCP-47 tags (`language[-Script][-REGION]`, e.g. `en`, `en-US`, `es-419`, `zh-Hans-CN`), normalizes casing, and rejects underscores, invalid subtags, or tags exceeding 35 characters.
   - Markets are bounded to `MAX_LOCALES_PER_MARKET` (64) localized descriptions.
   - Removing `canonical_locale` is always rejected (`LocalizationError::CannotRemoveCanonicalLocale`).
3. **Deterministic fallback chain with explicit reporting**:
   - `LocalizedMarketRecord::resolve(&preferred_locales)` checks each requested tag in order using progressive prefix fallbacks (`zh-Hans-CN` -> `zh-Hans` -> `zh`) before falling back to `canonical_locale`.
   - Every `ResolvedMarketContent` includes `fallback_reason`:
     - `exact_match`
     - `language_fallback`
     - `canonical_fallback`
     - `invalid_requested_locale_fallback`
   - Operators and translation tooling can inspect `stale_locales()` to locate translations authored against an outdated canonical question digest.
4. **Role scoping & optimistic concurrency**:
   - `Translator` actors may create/update non-canonical locales but cannot overwrite the canonical locale description or remove translations.
   - `Creator` (matching `creator_id`), `Moderator`, and `Admin` enforce `expected_revision` checks (`LocalizationError::RevisionConflict`) so concurrent edits require reload-and-retry.

---

## 2. Append-Only Market Metadata Change History (#247)

### Authoritative Invariants

1. **Tamper-evident hash chain**:
   - `MarketMetadataHistory` records every metadata state as an immutable `MetadataChangeEntry` containing a strictly monotonic 1-based `version`, `timestamp_unix_ms`, `actor`, `actor_role`, `origin`, mandatory audit `reason`, sorted `MetadataFieldDiff` entries, full normalized `MarketMetadataSnapshot`, `prev_hash`, and domain-separated SHA-256 `entry_hash` (`predifi-market-metadata-history-v1`).
   - No-op updates with an empty diff are rejected (`MetadataHistoryError::NoChangesDetected`).
   - Non-monotonic timestamps are rejected (`MetadataHistoryError::NonMonotonicTimestamp`).
2. **Lifecycle and role boundaries**:
   - `Translator` edits are restricted strictly to `localized_descriptions.*` fields.
   - Once `published == true`, core contract fields (`canonical_question`, `close_time_unix_secs`) are locked and any edit attempting to change them fails with `MetadataHistoryError::ImmutableFieldModified`.
   - Once `resolved == true`, normal edits are rejected (`MetadataHistoryError::MarketResolved`); only `Admin` actors may append a `ChangeOrigin::Correction` entry (still respecting post-publish field immutability).
3. **Optimistic concurrency & point-in-time queries**:
   - All updates and rollbacks validate `expected_version` (`MetadataHistoryError::VersionConflict`).
   - Callers can inspect `at_version(v)`, bounded pages via `page(offset, limit)` (`limit <= 100`), and arbitrary range diffs via `diff_between(from_version, to_version)`.

### Verification & Recovery Procedures

- **Append-only rollback**:
  - If an erroneous metadata update is published, a `Moderator` or `Admin` invokes `rollback_to_version(target_version, expected_version, reason, timestamp_unix_ms)`.
  - Prior history entries are never deleted or modified; a new entry with `origin: ChangeOrigin::Rollback` and `supersedes_version: Some(target_version)` is appended.
  - Rollbacks cannot revert immutable post-publish fields (`canonical_question`, `close_time_unix_secs`) after publication.
- **Integrity audit**:
  - Run `verify_integrity()` on persisted histories to detect version gaps, timestamp regressions, broken `prev_hash` links, invalid `supersedes_version` pointers, or modified entry payloads (`MetadataHistoryError::IntegrityFailure`).
