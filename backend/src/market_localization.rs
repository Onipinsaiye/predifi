//! Localized market descriptions anchored to an immutable canonical question (#246).
//!
//! Every market has exactly one canonical question and canonical locale used
//! for identity, settlement semantics, and content digests. Localized
//! descriptions may be added for any valid BCP-47 locale tag and are resolved
//! deterministically (`exact -> language prefix -> canonical`) with explicit
//! fallback and translation-staleness reporting.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_LOCALE_TAG_LEN: usize = 35;
pub const MAX_QUESTION_LEN: usize = 280;
pub const MAX_DESCRIPTION_LEN: usize = 4000;
pub const MAX_SUMMARY_LEN: usize = 280;
pub const MAX_LOCALES_PER_MARKET: usize = 64;

const QUESTION_DIGEST_DOMAIN: &str = "predifi-canonical-question-v1";

/// Validated, canonically formatted BCP-47 locale tag (`language[-Script][-REGION]`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LocaleTag(String);

impl LocaleTag {
    /// Parse and normalize a BCP-47 tag (e.g. `"en-us"` -> `"en-US"`,
    /// `"zh-hans-cn"` -> `"zh-Hans-CN"`).
    pub fn parse(raw: &str) -> Result<Self, LocalizationError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.len() > MAX_LOCALE_TAG_LEN {
            return Err(LocalizationError::InvalidLocaleTag {
                raw: raw.to_string(),
                reason: "tag must be 1..=35 ASCII characters",
            });
        }
        if !trimmed.is_ascii() || trimmed.contains('_') {
            return Err(LocalizationError::InvalidLocaleTag {
                raw: raw.to_string(),
                reason: "tag must use ASCII hyphen-separated BCP-47 subtags",
            });
        }

        let parts: Vec<&str> = trimmed.split('-').collect();
        if parts.is_empty() || parts.len() > 3 {
            return Err(LocalizationError::InvalidLocaleTag {
                raw: raw.to_string(),
                reason: "tag must contain language and optional script/region subtags",
            });
        }

        let lang = parts[0];
        if !(2..=3).contains(&lang.len()) || !lang.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(LocalizationError::InvalidLocaleTag {
                raw: raw.to_string(),
                reason: "language subtag must be 2 or 3 ASCII letters",
            });
        }
        let mut normalized = lang.to_ascii_lowercase();

        let mut idx = 1;
        if idx < parts.len() && parts[idx].len() == 4 {
            let script = parts[idx];
            if !script.chars().all(|c| c.is_ascii_alphabetic()) {
                return Err(LocalizationError::InvalidLocaleTag {
                    raw: raw.to_string(),
                    reason: "script subtag must be 4 ASCII letters",
                });
            }
            let mut chars = script.chars();
            let first = chars.next().expect("length 4").to_ascii_uppercase();
            let rest: String = chars.map(|c| c.to_ascii_lowercase()).collect();
            normalized.push('-');
            normalized.push(first);
            normalized.push_str(&rest);
            idx += 1;
        }

        if idx < parts.len() {
            let region = parts[idx];
            let is_alpha2 = region.len() == 2 && region.chars().all(|c| c.is_ascii_alphabetic());
            let is_digit3 = region.len() == 3 && region.chars().all(|c| c.is_ascii_digit());
            if !is_alpha2 && !is_digit3 {
                return Err(LocalizationError::InvalidLocaleTag {
                    raw: raw.to_string(),
                    reason: "region subtag must be 2 ASCII letters or 3 ASCII digits",
                });
            }
            normalized.push('-');
            normalized.push_str(&region.to_ascii_uppercase());
            idx += 1;
        }

        if idx != parts.len() {
            return Err(LocalizationError::InvalidLocaleTag {
                raw: raw.to_string(),
                reason: "unexpected trailing subtags",
            });
        }

        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Return progressive fallback prefixes from most specific to base language.
    /// For `"zh-Hans-CN"`, returns `["zh-Hans-CN", "zh-Hans", "zh"]`.
    pub fn fallback_chain(&self) -> Vec<String> {
        let parts: Vec<&str> = self.0.split('-').collect();
        let mut chain = Vec::with_capacity(parts.len());
        for len in (1..=parts.len()).rev() {
            chain.push(parts[..len].join("-"));
        }
        chain
    }
}

impl fmt::Display for LocaleTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Deterministic SHA-256 hex digest of the canonical market question.
pub fn canonical_question_digest(question: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(QUESTION_DIGEST_DOMAIN.as_bytes());
    hasher.update(b"\n");
    hasher.update(question.trim().as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Authorization role for modifying localized market descriptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalizationRole {
    Viewer,
    Creator,
    Translator,
    Moderator,
    Admin,
}

/// Authenticated actor interacting with market localizations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizationActor {
    pub id: String,
    pub role: LocalizationRole,
}

impl LocalizationActor {
    pub fn new(id: impl Into<String>, role: LocalizationRole) -> Self {
        Self {
            id: id.into(),
            role,
        }
    }
}

/// A localized description bound to a specific canonical question digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizedDescription {
    pub locale: String,
    pub description: String,
    pub summary: Option<String>,
    pub canonical_question_digest: String,
    pub updated_by: String,
    pub updated_at_ms: i64,
    pub revision: u64,
}

/// Explains how a locale lookup resolved so callers can surface fallback UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackReason {
    ExactMatch,
    LanguageFallback,
    CanonicalFallback,
    InvalidRequestedLocaleFallback,
}

/// Resolved market view always anchored to the canonical question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedMarketContent {
    pub market_id: u64,
    pub canonical_locale: String,
    pub canonical_question: String,
    pub canonical_question_digest: String,
    pub requested_locale: Option<String>,
    pub resolved_locale: String,
    pub description: String,
    pub summary: Option<String>,
    pub fallback_reason: FallbackReason,
    pub is_stale_translation: bool,
}

/// Input payload for creating or updating a localized market description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertLocalizationRequest {
    pub locale: String,
    pub description: String,
    pub summary: Option<String>,
    pub expected_revision: u64,
    pub updated_at_ms: i64,
}

/// Errors produced at the localization boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalizationError {
    InvalidLocaleTag {
        raw: String,
        reason: &'static str,
    },
    Validation {
        field: &'static str,
        message: String,
    },
    Unauthorized {
        actor: String,
        action: &'static str,
    },
    CanonicalQuestionImmutable {
        market_id: u64,
    },
    CannotRemoveCanonicalLocale {
        locale: String,
    },
    LocaleNotFound {
        locale: String,
    },
    LocaleLimitExceeded {
        max: usize,
    },
    RevisionConflict {
        expected: u64,
        actual: u64,
    },
}

impl fmt::Display for LocalizationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LocalizationError::InvalidLocaleTag { raw, reason } => {
                write!(f, "invalid locale tag '{raw}': {reason}")
            }
            LocalizationError::Validation { field, message } => {
                write!(f, "invalid field '{field}': {message}")
            }
            LocalizationError::Unauthorized { actor, action } => {
                write!(f, "actor '{actor}' is not authorized to {action}")
            }
            LocalizationError::CanonicalQuestionImmutable { market_id } => {
                write!(f, "canonical question for published market #{market_id} is immutable")
            }
            LocalizationError::CannotRemoveCanonicalLocale { locale } => {
                write!(f, "cannot remove canonical locale '{locale}'")
            }
            LocalizationError::LocaleNotFound { locale } => {
                write!(f, "localized description for '{locale}' not found")
            }
            LocalizationError::LocaleLimitExceeded { max } => {
                write!(f, "market cannot exceed {max} localized descriptions")
            }
            LocalizationError::RevisionConflict { expected, actual } => {
                write!(f, "revision conflict: expected {expected}, actual {actual}")
            }
        }
    }
}

impl std::error::Error for LocalizationError {}

fn validate_text(
    field: &'static str,
    raw: &str,
    max_len: usize,
) -> Result<String, LocalizationError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(LocalizationError::Validation {
            field,
            message: "must not be empty".to_string(),
        });
    }
    if trimmed.chars().count() > max_len {
        return Err(LocalizationError::Validation {
            field,
            message: format!("must be at most {max_len} characters"),
        });
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(LocalizationError::Validation {
            field,
            message: "must not contain control characters".to_string(),
        });
    }
    Ok(trimmed.to_string())
}

/// Authoritative record storing a market's canonical question and localized
/// descriptions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizedMarketRecord {
    pub market_id: u64,
    pub creator_id: String,
    pub canonical_locale: String,
    pub canonical_question: String,
    pub canonical_question_digest: String,
    pub descriptions: BTreeMap<String, LocalizedDescription>,
    pub published: bool,
    pub revision: u64,
}

impl LocalizedMarketRecord {
    /// Create a new market localization record with its canonical question and
    /// canonical description.
    pub fn new(
        market_id: u64,
        creator_id: impl Into<String>,
        canonical_locale: &str,
        canonical_question: &str,
        canonical_description: &str,
        created_at_ms: i64,
    ) -> Result<Self, LocalizationError> {
        if market_id == 0 {
            return Err(LocalizationError::Validation {
                field: "market_id",
                message: "must be positive".to_string(),
            });
        }
        let creator_id = validate_text("creator_id", &creator_id.into(), 128)?;
        let locale = LocaleTag::parse(canonical_locale)?;
        let question = validate_text("canonical_question", canonical_question, MAX_QUESTION_LEN)?;
        let description = validate_text(
            "canonical_description",
            canonical_description,
            MAX_DESCRIPTION_LEN,
        )?;
        let digest = canonical_question_digest(&question);

        let mut descriptions = BTreeMap::new();
        descriptions.insert(
            locale.as_str().to_string(),
            LocalizedDescription {
                locale: locale.as_str().to_string(),
                description,
                summary: None,
                canonical_question_digest: digest.clone(),
                updated_by: creator_id.clone(),
                updated_at_ms: created_at_ms,
                revision: 1,
            },
        );

        Ok(Self {
            market_id,
            creator_id,
            canonical_locale: locale.as_str().to_string(),
            canonical_question: question,
            canonical_question_digest: digest,
            descriptions,
            published: false,
            revision: 1,
        })
    }

    /// Seal the canonical question by marking the market published.
    pub fn publish(
        &mut self,
        actor: &LocalizationActor,
        expected_revision: u64,
    ) -> Result<(), LocalizationError> {
        self.check_revision(expected_revision)?;
        let authorized = match actor.role {
            LocalizationRole::Admin => true,
            LocalizationRole::Creator => actor.id == self.creator_id,
            _ => false,
        };
        if !authorized {
            return Err(LocalizationError::Unauthorized {
                actor: actor.id.clone(),
                action: "publish market localization",
            });
        }
        self.published = true;
        self.revision += 1;
        Ok(())
    }

    /// Update the canonical question before publishing. Post-publish updates
    /// are rejected so settlement semantics cannot silently drift. Pre-publish
    /// updates recompute the question digest so existing non-canonical
    /// translations are flagged as stale until reviewed.
    pub fn update_canonical_question(
        &mut self,
        actor: &LocalizationActor,
        new_question: &str,
        expected_revision: u64,
    ) -> Result<(), LocalizationError> {
        self.check_revision(expected_revision)?;
        let authorized = match actor.role {
            LocalizationRole::Admin => true,
            LocalizationRole::Creator => actor.id == self.creator_id,
            _ => false,
        };
        if !authorized {
            return Err(LocalizationError::Unauthorized {
                actor: actor.id.clone(),
                action: "update canonical question",
            });
        }
        if self.published {
            return Err(LocalizationError::CanonicalQuestionImmutable {
                market_id: self.market_id,
            });
        }

        let question = validate_text("canonical_question", new_question, MAX_QUESTION_LEN)?;
        let digest = canonical_question_digest(&question);
        self.canonical_question = question;
        self.canonical_question_digest = digest.clone();
        if let Some(canonical_desc) = self.descriptions.get_mut(&self.canonical_locale) {
            canonical_desc.canonical_question_digest = digest;
        }
        self.revision += 1;
        Ok(())
    }

    /// Add or update a localized description for a validated BCP-47 locale.
    pub fn upsert_description(
        &mut self,
        actor: &LocalizationActor,
        req: UpsertLocalizationRequest,
    ) -> Result<LocalizedDescription, LocalizationError> {
        self.check_revision(req.expected_revision)?;
        let actor_id = validate_text("actor.id", &actor.id, 128)?;
        let locale = LocaleTag::parse(&req.locale)?;

        let is_canonical = locale.as_str() == self.canonical_locale;
        let authorized = match actor.role {
            LocalizationRole::Admin | LocalizationRole::Moderator => true,
            LocalizationRole::Translator => !is_canonical,
            LocalizationRole::Creator => actor_id == self.creator_id,
            LocalizationRole::Viewer => false,
        };
        if !authorized {
            return Err(LocalizationError::Unauthorized {
                actor: actor_id,
                action: "upsert localized description",
            });
        }

        if !self.descriptions.contains_key(locale.as_str())
            && self.descriptions.len() >= MAX_LOCALES_PER_MARKET
        {
            return Err(LocalizationError::LocaleLimitExceeded {
                max: MAX_LOCALES_PER_MARKET,
            });
        }

        let description = validate_text("description", &req.description, MAX_DESCRIPTION_LEN)?;
        let summary = match req.summary.as_deref() {
            Some(s) if !s.trim().is_empty() => {
                Some(validate_text("summary", s, MAX_SUMMARY_LEN)?)
            }
            _ => None,
        };

        let next_entry_rev = self
            .descriptions
            .get(locale.as_str())
            .map(|d| d.revision + 1)
            .unwrap_or(1);

        let entry = LocalizedDescription {
            locale: locale.as_str().to_string(),
            description,
            summary,
            canonical_question_digest: self.canonical_question_digest.clone(),
            updated_by: actor_id,
            updated_at_ms: req.updated_at_ms,
            revision: next_entry_rev,
        };

        self.descriptions.insert(locale.as_str().to_string(), entry.clone());
        self.revision += 1;
        Ok(entry)
    }

    /// Remove a non-canonical localized description. Removing the canonical
    /// locale is always rejected to guarantee a deterministic fallback.
    pub fn remove_description(
        &mut self,
        actor: &LocalizationActor,
        locale_raw: &str,
        expected_revision: u64,
    ) -> Result<(), LocalizationError> {
        self.check_revision(expected_revision)?;
        let locale = LocaleTag::parse(locale_raw)?;

        let authorized = match actor.role {
            LocalizationRole::Admin | LocalizationRole::Moderator => true,
            LocalizationRole::Creator => !self.published && actor.id == self.creator_id,
            LocalizationRole::Translator | LocalizationRole::Viewer => false,
        };
        if !authorized {
            return Err(LocalizationError::Unauthorized {
                actor: actor.id.clone(),
                action: "remove localized description",
            });
        }
        if locale.as_str() == self.canonical_locale {
            return Err(LocalizationError::CannotRemoveCanonicalLocale {
                locale: self.canonical_locale.clone(),
            });
        }
        if self.descriptions.remove(locale.as_str()).is_none() {
            return Err(LocalizationError::LocaleNotFound {
                locale: locale.as_str().to_string(),
            });
        }
        self.revision += 1;
        Ok(())
    }

    /// Resolve localized market content for a client's ordered preference list
    /// (e.g. `["pt-BR", "es"]`). Always preserves `canonical_question` and
    /// explicitly reports exact matches, language fallbacks, canonical
    /// fallbacks, and stale translations.
    pub fn resolve(&self, preferred_locales: &[&str]) -> ResolvedMarketContent {
        let mut first_valid_request: Option<String> = None;
        let mut saw_invalid = false;

        for raw in preferred_locales {
            let Ok(parsed) = LocaleTag::parse(raw) else {
                saw_invalid = true;
                continue;
            };
            if first_valid_request.is_none() {
                first_valid_request = Some(parsed.as_str().to_string());
            }
            let chain = parsed.fallback_chain();
            for (idx, candidate) in chain.iter().enumerate() {
                if let Some(entry) = self.descriptions.get(candidate) {
                    let fallback_reason = if idx == 0 {
                        FallbackReason::ExactMatch
                    } else {
                        FallbackReason::LanguageFallback
                    };
                    return self.build_resolved(
                        Some(parsed.as_str().to_string()),
                        entry,
                        fallback_reason,
                    );
                }
            }
        }

        let canonical_entry = self
            .descriptions
            .get(&self.canonical_locale)
            .expect("canonical locale entry always present");
        let reason = if first_valid_request.is_none() && saw_invalid {
            FallbackReason::InvalidRequestedLocaleFallback
        } else {
            FallbackReason::CanonicalFallback
        };
        self.build_resolved(first_valid_request, canonical_entry, reason)
    }

    /// Return normalized locale tags whose translations were authored against an
    /// older canonical question digest and require review.
    pub fn stale_locales(&self) -> Vec<String> {
        self.descriptions
            .values()
            .filter(|d| d.canonical_question_digest != self.canonical_question_digest)
            .map(|d| d.locale.clone())
            .collect()
    }

    /// Return all available normalized locale tags in deterministic order.
    pub fn available_locales(&self) -> Vec<String> {
        self.descriptions.keys().cloned().collect()
    }

    fn build_resolved(
        &self,
        requested_locale: Option<String>,
        entry: &LocalizedDescription,
        fallback_reason: FallbackReason,
    ) -> ResolvedMarketContent {
        let is_stale_translation =
            entry.canonical_question_digest != self.canonical_question_digest;
        ResolvedMarketContent {
            market_id: self.market_id,
            canonical_locale: self.canonical_locale.clone(),
            canonical_question: self.canonical_question.clone(),
            canonical_question_digest: self.canonical_question_digest.clone(),
            requested_locale,
            resolved_locale: entry.locale.clone(),
            description: entry.description.clone(),
            summary: entry.summary.clone(),
            fallback_reason,
            is_stale_translation,
        }
    }

    fn check_revision(&self, expected: u64) -> Result<(), LocalizationError> {
        if self.revision != expected {
            return Err(LocalizationError::RevisionConflict {
                expected,
                actual: self.revision,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_market() -> LocalizedMarketRecord {
        LocalizedMarketRecord::new(
            101,
            "creator-1",
            "en-us",
            "Will BTC close above $100,000 on Dec 31?",
            "Resolves YES if the Coinbase BTC-USD daily close is strictly above $100,000.",
            1_000,
        )
        .unwrap()
    }

    #[test]
    fn locale_tag_normalizes_and_validates_bcp47() {
        assert_eq!(LocaleTag::parse("EN-us").unwrap().as_str(), "en-US");
        assert_eq!(LocaleTag::parse("zh-hans-cn").unwrap().as_str(), "zh-Hans-CN");
        assert_eq!(LocaleTag::parse("es-419").unwrap().as_str(), "es-419");
        assert_eq!(
            LocaleTag::parse("zh-Hans-CN").unwrap().fallback_chain(),
            vec!["zh-Hans-CN", "zh-Hans", "zh"]
        );

        for bad in ["", "en_US", "e", "english", "en-12", "en-US-extra-tag"] {
            assert!(
                matches!(LocaleTag::parse(bad), Err(LocalizationError::InvalidLocaleTag { .. })),
                "expected invalid tag for {bad:?}"
            );
        }
    }

    #[test]
    fn resolution_preserves_canonical_question_across_exact_prefix_and_canonical_fallback() {
        let mut record = sample_market();
        let translator = LocalizationActor::new("trans-1", LocalizationRole::Translator);

        record
            .upsert_description(
                &translator,
                UpsertLocalizationRequest {
                    locale: "es".into(),
                    description: "Resuelve SI si el cierre diario supera $100,000.".into(),
                    summary: Some("Resumen en espanol".into()),
                    expected_revision: record.revision,
                    updated_at_ms: 2_000,
                },
            )
            .unwrap();
        record
            .upsert_description(
                &translator,
                UpsertLocalizationRequest {
                    locale: "zh-hans".into(),
                    description: "BTC收盘价高于100,000美元则结算为是。".into(),
                    summary: None,
                    expected_revision: record.revision,
                    updated_at_ms: 3_000,
                },
            )
            .unwrap();

        // Exact match preserves canonical question.
        let exact = record.resolve(&["es"]);
        assert_eq!(exact.fallback_reason, FallbackReason::ExactMatch);
        assert_eq!(exact.resolved_locale, "es");
        assert_eq!(exact.canonical_question, record.canonical_question);
        assert_eq!(exact.summary.as_deref(), Some("Resumen en espanol"));
        assert!(!exact.is_stale_translation);

        // Regional tag falls back to script/language prefix.
        let prefix = record.resolve(&["zh-Hans-SG"]);
        assert_eq!(prefix.fallback_reason, FallbackReason::LanguageFallback);
        assert_eq!(prefix.resolved_locale, "zh-Hans");
        assert_eq!(prefix.canonical_question, record.canonical_question);

        // Missing locale falls back explicitly to canonical locale.
        let canonical = record.resolve(&["ja-JP"]);
        assert_eq!(canonical.fallback_reason, FallbackReason::CanonicalFallback);
        assert_eq!(canonical.resolved_locale, "en-US");
        assert_eq!(canonical.requested_locale.as_deref(), Some("ja-JP"));

        // Malformed requested locales safely fall back with explicit reason.
        let invalid = record.resolve(&["bad_locale"]);
        assert_eq!(invalid.fallback_reason, FallbackReason::InvalidRequestedLocaleFallback);
        assert_eq!(invalid.resolved_locale, "en-US");
    }

    #[test]
    fn canonical_question_immutable_after_publish_and_stale_tracking_before_publish() {
        let mut record = sample_market();
        let creator = LocalizationActor::new("creator-1", LocalizationRole::Creator);
        let translator = LocalizationActor::new("trans-1", LocalizationRole::Translator);

        record
            .upsert_description(
                &translator,
                UpsertLocalizationRequest {
                    locale: "fr".into(),
                    description: "Description initiale en francais.".into(),
                    summary: None,
                    expected_revision: record.revision,
                    updated_at_ms: 2_000,
                },
            )
            .unwrap();

        // Updating canonical question pre-publish flags French translation as stale.
        record
            .update_canonical_question(
                &creator,
                "Will BTC close above $110,000 on Dec 31?",
                record.revision,
            )
            .unwrap();
        assert_eq!(record.stale_locales(), vec!["fr"]);
        assert!(record.resolve(&["fr"]).is_stale_translation);

        // Refreshing the French translation clears staleness.
        record
            .upsert_description(
                &translator,
                UpsertLocalizationRequest {
                    locale: "fr".into(),
                    description: "Description mise a jour pour $110,000.".into(),
                    summary: None,
                    expected_revision: record.revision,
                    updated_at_ms: 3_000,
                },
            )
            .unwrap();
        assert!(record.stale_locales().is_empty());
        assert!(!record.resolve(&["fr"]).is_stale_translation);

        // Publish seals canonical question permanently.
        record.publish(&creator, record.revision).unwrap();
        assert_eq!(
            record
                .update_canonical_question(&creator, "Attempted mutation", record.revision)
                .unwrap_err(),
            LocalizationError::CanonicalQuestionImmutable { market_id: 101 }
        );
    }

    #[test]
    fn authorization_canonical_removal_and_revision_conflict_retry() {
        let mut record = sample_market();
        let viewer = LocalizationActor::new("viewer-1", LocalizationRole::Viewer);
        let translator = LocalizationActor::new("trans-1", LocalizationRole::Translator);
        let admin = LocalizationActor::new("admin-1", LocalizationRole::Admin);

        // Viewer cannot write.
        assert!(matches!(
            record.upsert_description(
                &viewer,
                UpsertLocalizationRequest {
                    locale: "de".into(),
                    description: "Deutsche Beschreibung".into(),
                    summary: None,
                    expected_revision: record.revision,
                    updated_at_ms: 2_000,
                },
            ),
            Err(LocalizationError::Unauthorized { .. })
        ));

        // Translator cannot overwrite canonical locale description.
        assert!(matches!(
            record.upsert_description(
                &translator,
                UpsertLocalizationRequest {
                    locale: "en-US".into(),
                    description: "Unauthorized canonical overwrite".into(),
                    summary: None,
                    expected_revision: record.revision,
                    updated_at_ms: 2_000,
                },
            ),
            Err(LocalizationError::Unauthorized { .. })
        ));

        // Canonical locale cannot be removed even by admin.
        assert_eq!(
            record
                .remove_description(&admin, "en-US", record.revision)
                .unwrap_err(),
            LocalizationError::CannotRemoveCanonicalLocale {
                locale: "en-US".into(),
            }
        );

        // Stale revision fails cleanly and retry with latest revision succeeds.
        let stale_rev = record.revision;
        record
            .upsert_description(
                &translator,
                UpsertLocalizationRequest {
                    locale: "de".into(),
                    description: "Erste Version".into(),
                    summary: None,
                    expected_revision: stale_rev,
                    updated_at_ms: 2_000,
                },
            )
            .unwrap();

        assert_eq!(
            record
                .upsert_description(
                    &translator,
                    UpsertLocalizationRequest {
                        locale: "de".into(),
                        description: "Zweite Version".into(),
                        summary: None,
                        expected_revision: stale_rev,
                        updated_at_ms: 2_500,
                    },
                )
                .unwrap_err(),
            LocalizationError::RevisionConflict {
                expected: 1,
                actual: 2,
            }
        );

        let updated = record
            .upsert_description(
                &translator,
                UpsertLocalizationRequest {
                    locale: "de".into(),
                    description: "Zweite Version".into(),
                    summary: None,
                    expected_revision: record.revision,
                    updated_at_ms: 2_600,
                },
            )
            .unwrap();
        assert_eq!(updated.revision, 2);

        // JSON round-trip preserves full record state.
        let json = serde_json::to_string(&record).unwrap();
        let back: LocalizedMarketRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, record);
    }
}
