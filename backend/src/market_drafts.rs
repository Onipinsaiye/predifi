//! Reviewable draft-to-publish workflow for market creators (#238).
//!
//! Lifecycle: `Draft` (private to the creator) -> `Submitted` (explicitly
//! submitted, frozen, reviewable) -> `Published` (payload sealed). Publishing
//! revalidates against the *current* contract rules and seals the transaction
//! payload with a digest so it cannot silently change afterwards.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::market_citations::{CitationHistory, CitationIssue, SourceCitation};
use crate::market_templates::{DraftField, TemplateError, TemplateRegistry};

/// Contract/API limits a draft must satisfy at publish time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractRules {
    /// Bumped whenever the contract configuration changes.
    pub config_version: u64,
    pub min_outcomes: usize,
    pub max_outcomes: usize,
    pub min_duration_secs: i64,
    pub max_duration_secs: i64,
    pub max_question_len: usize,
}

impl Default for ContractRules {
    fn default() -> Self {
        Self {
            config_version: 1,
            min_outcomes: 2,
            max_outcomes: 8,
            min_duration_secs: 3600,
            max_duration_secs: 365 * 24 * 3600,
            max_question_len: 280,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftFields {
    pub question: String,
    pub outcomes: Vec<String>,
    /// Unix seconds.
    pub end_time: Option<i64>,
    pub category: Option<String>,
    pub citation: Option<SourceCitation>,
}

/// Partial edit; `None` leaves a field untouched.
#[derive(Debug, Clone, Default)]
pub struct DraftPatch {
    pub question: Option<String>,
    pub outcomes: Option<Vec<String>>,
    pub end_time: Option<i64>,
    pub category: Option<String>,
    pub citation: Option<SourceCitation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftStatus {
    Draft,
    Submitted,
    Published,
}

/// The exact payload sent on-chain, sealed at publish time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedPayload {
    pub question: String,
    pub outcomes: Vec<String>,
    pub end_time: i64,
    pub category: Option<String>,
    pub citation: SourceCitation,
    pub template_id: String,
    pub template_version: u32,
    pub contract_config_version: u64,
}

impl PublishedPayload {
    pub fn digest(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("payload serializes");
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedPayload {
    pub payload: PublishedPayload,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketDraft {
    pub id: u64,
    pub owner: String,
    pub template_id: String,
    pub template_version: u32,
    pub fields: DraftFields,
    /// Incremented on every mutation; used for optimistic concurrency.
    pub revision: u64,
    pub status: DraftStatus,
    pub sealed: Option<SealedPayload>,
    pub citation_history: CitationHistory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldIssue {
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftError {
    /// Missing or owned by someone else: indistinguishable on purpose.
    NotFound,
    Template(TemplateError),
    Invalid(Vec<FieldIssue>),
    /// The caller edited a stale revision.
    Conflict { expected: u64, actual: u64 },
    /// The draft is no longer in a state that allows this transition.
    InvalidState(DraftStatus),
}

impl From<TemplateError> for DraftError {
    fn from(e: TemplateError) -> Self {
        DraftError::Template(e)
    }
}

fn field_issue(field: &str, message: impl Into<String>) -> FieldIssue {
    FieldIssue {
        field: field.into(),
        message: message.into(),
    }
}

/// Validate `fields` against the bound template and the current contract rules.
pub fn validate_draft(
    registry: &TemplateRegistry,
    draft: &MarketDraft,
    rules: &ContractRules,
    now: i64,
) -> Result<PublishedPayload, DraftError> {
    let template = registry.resolve(&draft.template_id, draft.template_version)?;
    let f = &draft.fields;
    let mut issues = Vec::new();

    for required in &template.required_fields {
        let missing = match required {
            DraftField::Question => f.question.trim().is_empty(),
            DraftField::Outcomes => f.outcomes.is_empty(),
            DraftField::EndTime => f.end_time.is_none(),
            DraftField::Category => f.category.as_deref().map_or(true, |c| c.trim().is_empty()),
            DraftField::Citation => f.citation.is_none(),
        };
        if missing {
            issues.push(field_issue(required.as_str(), "is required by the template"));
        }
    }

    if f.question.chars().count() > rules.max_question_len {
        issues.push(field_issue(
            "question",
            format!("must be at most {} characters", rules.max_question_len),
        ));
    }

    let outcomes: Vec<String> = f.outcomes.iter().map(|o| o.trim().to_string()).collect();
    if outcomes.len() < rules.min_outcomes || outcomes.len() > rules.max_outcomes {
        issues.push(field_issue(
            "outcomes",
            format!(
                "must have between {} and {} outcomes",
                rules.min_outcomes, rules.max_outcomes
            ),
        ));
    }
    if outcomes.iter().any(|o| o.is_empty()) {
        issues.push(field_issue("outcomes", "must not contain blank labels"));
    }
    let mut seen = std::collections::HashSet::new();
    if !outcomes.iter().all(|o| seen.insert(o.to_lowercase())) {
        issues.push(field_issue("outcomes", "must be unique"));
    }

    if let Some(end) = f.end_time {
        let duration = end - now;
        if duration < rules.min_duration_secs || duration > rules.max_duration_secs {
            issues.push(field_issue(
                "end_time",
                format!(
                    "must be between {} and {} seconds from now",
                    rules.min_duration_secs, rules.max_duration_secs
                ),
            ));
        }
    }

    if let Some(citation) = &f.citation {
        if let Err(citation_issues) = citation.validate() {
            issues.extend(
                citation_issues
                    .into_iter()
                    .map(|CitationIssue { field, message }| {
                        field_issue(&format!("citation.{field}"), message)
                    }),
            );
        }
    }

    if !issues.is_empty() {
        return Err(DraftError::Invalid(issues));
    }

    Ok(PublishedPayload {
        question: f.question.trim().to_string(),
        outcomes,
        end_time: f.end_time.expect("checked"),
        category: f.category.as_ref().map(|c| c.trim().to_string()),
        citation: f.citation.clone().expect("checked").normalized(),
        template_id: draft.template_id.clone(),
        template_version: draft.template_version,
        contract_config_version: rules.config_version,
    })
}

/// In-memory draft repository with owner scoping and optimistic concurrency.
pub struct DraftStore {
    registry: TemplateRegistry,
    inner: Mutex<StoreInner>,
}

#[derive(Default)]
struct StoreInner {
    next_id: u64,
    drafts: HashMap<u64, MarketDraft>,
}

impl DraftStore {
    pub fn new(registry: TemplateRegistry) -> Self {
        Self {
            registry,
            inner: Mutex::new(StoreInner::default()),
        }
    }

    pub fn registry(&self) -> &TemplateRegistry {
        &self.registry
    }

    /// Create a draft bound to an exact template version, applying its defaults.
    pub fn create(
        &self,
        owner: &str,
        template_id: &str,
        template_version: u32,
        now: i64,
    ) -> Result<MarketDraft, DraftError> {
        let template = self.registry.resolve(template_id, template_version)?;
        let mut inner = self.inner.lock().expect("store lock");
        inner.next_id += 1;
        let draft = MarketDraft {
            id: inner.next_id,
            owner: owner.to_string(),
            template_id: template.id.clone(),
            template_version: template.version,
            fields: DraftFields {
                question: String::new(),
                outcomes: template.defaults.outcomes.clone(),
                end_time: Some(now + template.defaults.duration_secs as i64),
                category: template.defaults.category.clone(),
                citation: None,
            },
            revision: 1,
            status: DraftStatus::Draft,
            sealed: None,
            citation_history: CitationHistory::default(),
        };
        inner.drafts.insert(draft.id, draft.clone());
        Ok(draft)
    }

    /// Drafts are private: only the owner can read one until it is submitted.
    pub fn get(&self, requester: &str, id: u64) -> Result<MarketDraft, DraftError> {
        let inner = self.inner.lock().expect("store lock");
        match inner.drafts.get(&id) {
            Some(d) if d.owner == requester || d.status != DraftStatus::Draft => Ok(d.clone()),
            _ => Err(DraftError::NotFound),
        }
    }

    pub fn list_for_owner(&self, owner: &str) -> Vec<MarketDraft> {
        let inner = self.inner.lock().expect("store lock");
        let mut drafts: Vec<_> = inner
            .drafts
            .values()
            .filter(|d| d.owner == owner)
            .cloned()
            .collect();
        drafts.sort_by_key(|d| d.id);
        drafts
    }

    fn with_owned<T>(
        &self,
        requester: &str,
        id: u64,
        f: impl FnOnce(&mut MarketDraft) -> Result<T, DraftError>,
    ) -> Result<T, DraftError> {
        let mut inner = self.inner.lock().expect("store lock");
        match inner.drafts.get_mut(&id) {
            Some(d) if d.owner == requester => f(d),
            _ => Err(DraftError::NotFound),
        }
    }

    /// Apply a patch if `expected_revision` is still current.
    pub fn update(
        &self,
        requester: &str,
        id: u64,
        expected_revision: u64,
        patch: DraftPatch,
        _now: i64,
    ) -> Result<MarketDraft, DraftError> {
        self.with_owned(requester, id, |d| {
            if d.status != DraftStatus::Draft {
                return Err(DraftError::InvalidState(d.status));
            }
            if d.revision != expected_revision {
                return Err(DraftError::Conflict {
                    expected: expected_revision,
                    actual: d.revision,
                });
            }
            if let Some(q) = patch.question {
                d.fields.question = q;
            }
            if let Some(o) = patch.outcomes {
                d.fields.outcomes = o;
            }
            if let Some(t) = patch.end_time {
                d.fields.end_time = Some(t);
            }
            if let Some(c) = patch.category {
                d.fields.category = Some(c);
            }
            if let Some(c) = patch.citation {
                d.fields.citation = Some(c);
            }
            d.revision += 1;
            Ok(d.clone())
        })
    }

    /// Move a template-bound draft to a newer template version, revalidating
    /// that nothing the new version requires is missing is deferred to submit.
    pub fn upgrade_template(
        &self,
        requester: &str,
        id: u64,
        expected_revision: u64,
        to_version: u32,
    ) -> Result<MarketDraft, DraftError> {
        self.with_owned(requester, id, |d| {
            if d.status != DraftStatus::Draft {
                return Err(DraftError::InvalidState(d.status));
            }
            if d.revision != expected_revision {
                return Err(DraftError::Conflict {
                    expected: expected_revision,
                    actual: d.revision,
                });
            }
            let target =
                self.registry
                    .check_upgrade(&d.template_id, d.template_version, to_version)?;
            if d.fields.category.is_none() {
                d.fields.category = target.defaults.category.clone();
            }
            d.template_version = target.version;
            d.revision += 1;
            Ok(d.clone())
        })
    }

    /// Explicit submission: validates, then freezes the draft for review.
    pub fn submit(
        &self,
        requester: &str,
        id: u64,
        expected_revision: u64,
        rules: &ContractRules,
        now: i64,
    ) -> Result<MarketDraft, DraftError> {
        self.with_owned(requester, id, |d| {
            if d.status != DraftStatus::Draft {
                return Err(DraftError::InvalidState(d.status));
            }
            if d.revision != expected_revision {
                return Err(DraftError::Conflict {
                    expected: expected_revision,
                    actual: d.revision,
                });
            }
            validate_draft(&self.registry, d, rules, now)?;
            d.status = DraftStatus::Submitted;
            d.revision += 1;
            Ok(d.clone())
        })
    }

    /// Revalidate against the *current* rules and seal the payload.
    pub fn publish(
        &self,
        requester: &str,
        id: u64,
        rules: &ContractRules,
        now: i64,
    ) -> Result<SealedPayload, DraftError> {
        self.with_owned(requester, id, |d| {
            match d.status {
                DraftStatus::Submitted => {}
                other => return Err(DraftError::InvalidState(other)),
            }
            let payload = validate_draft(&self.registry, d, rules, now)?;
            let citation = payload.citation.clone();
            d.citation_history
                .record_create(citation, now)
                .map_err(|_| DraftError::NotFound)?;
            let sealed = SealedPayload {
                digest: payload.digest(),
                payload,
            };
            d.sealed = Some(sealed.clone());
            d.status = DraftStatus::Published;
            d.revision += 1;
            Ok(sealed)
        })
    }
}

/// True when the sealed payload still matches its recorded digest.
pub fn payload_is_intact(sealed: &SealedPayload) -> bool {
    sealed.payload.digest() == sealed.digest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market_templates::BINARY_TEMPLATE_ID;

    const NOW: i64 = 1_000_000;

    fn store() -> DraftStore {
        DraftStore::new(TemplateRegistry::default())
    }

    fn citation() -> SourceCitation {
        SourceCitation {
            source_url: Some("https://example.org/results".into()),
            source_name: None,
            resolution_rule: "Resolves YES if the official results say so.".into(),
        }
    }

    fn filled(store: &DraftStore, owner: &str, version: u32) -> MarketDraft {
        let d = store.create(owner, BINARY_TEMPLATE_ID, version, NOW).unwrap();
        store
            .update(
                owner,
                d.id,
                d.revision,
                DraftPatch {
                    question: Some("Will it rain tomorrow?".into()),
                    category: Some("other".into()),
                    citation: Some(citation()),
                    ..Default::default()
                },
                NOW,
            )
            .unwrap()
    }

    #[test]
    fn drafts_are_private_until_submitted() {
        let s = store();
        let d = filled(&s, "alice", 2);
        assert_eq!(s.get("bob", d.id), Err(DraftError::NotFound));
        assert!(s.get("alice", d.id).is_ok());
        assert_eq!(
            s.update("bob", d.id, d.revision, DraftPatch::default(), NOW),
            Err(DraftError::NotFound)
        );
        s.submit("alice", d.id, d.revision, &ContractRules::default(), NOW)
            .unwrap();
        assert_eq!(s.get("bob", d.id).unwrap().status, DraftStatus::Submitted);
        assert!(s.list_for_owner("bob").is_empty());
    }

    #[test]
    fn invalid_drafts_are_rejected_with_field_issues() {
        let s = store();
        let d = s.create("alice", BINARY_TEMPLATE_ID, 2, NOW).unwrap();
        match s.submit("alice", d.id, d.revision, &ContractRules::default(), NOW) {
            Err(DraftError::Invalid(issues)) => {
                let fields: Vec<_> = issues.iter().map(|i| i.field.as_str()).collect();
                assert!(fields.contains(&"question"));
                assert!(fields.contains(&"citation"));
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
        assert_eq!(s.get("alice", d.id).unwrap().status, DraftStatus::Draft);
    }

    #[test]
    fn rejects_bad_outcomes_and_citation() {
        let s = store();
        let d = filled(&s, "alice", 2);
        let d = s
            .update(
                "alice",
                d.id,
                d.revision,
                DraftPatch {
                    outcomes: Some(vec!["Yes".into(), "yes".into()]),
                    citation: Some(SourceCitation {
                        source_url: Some("ftp://x".into()),
                        source_name: None,
                        resolution_rule: "x".into(),
                    }),
                    ..Default::default()
                },
                NOW,
            )
            .unwrap();
        let err = s
            .submit("alice", d.id, d.revision, &ContractRules::default(), NOW)
            .unwrap_err();
        let DraftError::Invalid(issues) = err else {
            panic!("expected Invalid")
        };
        let fields: Vec<_> = issues.iter().map(|i| i.field.as_str()).collect();
        assert!(fields.contains(&"outcomes"));
        assert!(fields.contains(&"citation.source_url"));
        assert!(fields.contains(&"citation.resolution_rule"));
    }

    #[test]
    fn concurrent_edits_on_stale_revision_conflict() {
        let s = store();
        let d = filled(&s, "alice", 2);
        s.update(
            "alice",
            d.id,
            d.revision,
            DraftPatch {
                question: Some("First writer".into()),
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
        let stale = s.update(
            "alice",
            d.id,
            d.revision,
            DraftPatch {
                question: Some("Second writer".into()),
                ..Default::default()
            },
            NOW,
        );
        assert_eq!(
            stale,
            Err(DraftError::Conflict {
                expected: d.revision,
                actual: d.revision + 1
            })
        );
        assert_eq!(s.get("alice", d.id).unwrap().fields.question, "First writer");
        assert!(matches!(
            s.submit("alice", d.id, d.revision, &ContractRules::default(), NOW),
            Err(DraftError::Conflict { .. })
        ));
    }

    #[test]
    fn concurrent_threads_cannot_both_win_one_revision() {
        use std::sync::Arc;
        let s = Arc::new(store());
        let d = filled(&s, "alice", 2);
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let s = Arc::clone(&s);
                std::thread::spawn(move || {
                    s.update(
                        "alice",
                        d.id,
                        d.revision,
                        DraftPatch {
                            question: Some(format!("writer {i}")),
                            ..Default::default()
                        },
                        NOW,
                    )
                    .is_ok()
                })
            })
            .collect();
        let wins = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|ok| *ok)
            .count();
        assert_eq!(wins, 1);
    }

    #[test]
    fn publish_requires_submission_and_seals_payload() {
        let s = store();
        let d = filled(&s, "alice", 2);
        assert_eq!(
            s.publish("alice", d.id, &ContractRules::default(), NOW),
            Err(DraftError::InvalidState(DraftStatus::Draft))
        );
        let submitted = s
            .submit("alice", d.id, d.revision, &ContractRules::default(), NOW)
            .unwrap();
        let sealed = s
            .publish("alice", submitted.id, &ContractRules::default(), NOW)
            .unwrap();
        assert!(payload_is_intact(&sealed));
        assert_eq!(sealed.payload.template_version, 2);
        assert_eq!(sealed.payload.question, "Will it rain tomorrow?");

        let published = s.get("alice", d.id).unwrap();
        assert_eq!(published.status, DraftStatus::Published);
        assert_eq!(published.citation_history.snapshots().len(), 1);
    }

    #[test]
    fn published_draft_cannot_mutate_payload() {
        let s = store();
        let d = filled(&s, "alice", 2);
        let d = s
            .submit("alice", d.id, d.revision, &ContractRules::default(), NOW)
            .unwrap();
        let sealed = s
            .publish("alice", d.id, &ContractRules::default(), NOW)
            .unwrap();
        let after = s.get("alice", d.id).unwrap();

        assert_eq!(
            s.update(
                "alice",
                d.id,
                after.revision,
                DraftPatch {
                    question: Some("Changed".into()),
                    ..Default::default()
                },
                NOW
            ),
            Err(DraftError::InvalidState(DraftStatus::Published))
        );
        assert_eq!(
            s.upgrade_template("alice", d.id, after.revision, 2),
            Err(DraftError::InvalidState(DraftStatus::Published))
        );
        assert_eq!(
            s.publish("alice", d.id, &ContractRules::default(), NOW),
            Err(DraftError::InvalidState(DraftStatus::Published))
        );
        assert_eq!(s.get("alice", d.id).unwrap().sealed, Some(sealed.clone()));

        let mut tampered = sealed;
        tampered.payload.question = "Tampered".into();
        assert!(!payload_is_intact(&tampered));
    }

    #[test]
    fn publish_revalidates_after_contract_config_change() {
        let s = store();
        let d = filled(&s, "alice", 2);
        let d = s
            .submit("alice", d.id, d.revision, &ContractRules::default(), NOW)
            .unwrap();

        // Contract now demands at least 3 outcomes and a longer minimum window.
        let tightened = ContractRules {
            config_version: 2,
            min_outcomes: 3,
            ..ContractRules::default()
        };
        let err = s.publish("alice", d.id, &tightened, NOW).unwrap_err();
        assert!(matches!(&err, DraftError::Invalid(i) if i.iter().any(|x| x.field == "outcomes")));
        let still = s.get("alice", d.id).unwrap();
        assert_eq!(still.status, DraftStatus::Submitted);
        assert!(still.sealed.is_none());

        // A compatible config change publishes and records the config version.
        let relaxed = ContractRules {
            config_version: 3,
            ..ContractRules::default()
        };
        let sealed = s.publish("alice", d.id, &relaxed, NOW).unwrap();
        assert_eq!(sealed.payload.contract_config_version, 3);
    }

    #[test]
    fn publish_fails_if_deadline_passed_while_submitted() {
        let s = store();
        let d = filled(&s, "alice", 2);
        let d = s
            .submit("alice", d.id, d.revision, &ContractRules::default(), NOW)
            .unwrap();
        let later = NOW + 30 * 24 * 3600;
        assert!(matches!(
            s.publish("alice", d.id, &ContractRules::default(), later),
            Err(DraftError::Invalid(i)) if i.iter().any(|x| x.field == "end_time")
        ));
    }

    #[test]
    fn drafts_bind_to_template_version_and_reject_unsupported() {
        let s = store();
        let d = s.create("alice", BINARY_TEMPLATE_ID, 1, NOW).unwrap();
        assert_eq!(d.template_version, 1);
        assert!(matches!(
            s.create("alice", BINARY_TEMPLATE_ID, 7, NOW),
            Err(DraftError::Template(TemplateError::UnsupportedVersion { requested: 7, .. }))
        ));
        assert!(matches!(
            s.create("alice", "missing", 1, NOW),
            Err(DraftError::Template(TemplateError::UnknownTemplate(_)))
        ));
    }

    #[test]
    fn v1_draft_does_not_require_category_until_upgraded() {
        let s = store();
        let d = s.create("alice", BINARY_TEMPLATE_ID, 1, NOW).unwrap();
        assert_eq!(d.fields.category, None);
        let d = s
            .update(
                "alice",
                d.id,
                d.revision,
                DraftPatch {
                    question: Some("Will it rain tomorrow?".into()),
                    citation: Some(citation()),
                    ..Default::default()
                },
                NOW,
            )
            .unwrap();
        assert!(validate_draft(s.registry(), &d, &ContractRules::default(), NOW).is_ok());

        let upgraded = s.upgrade_template("alice", d.id, d.revision, 2).unwrap();
        assert_eq!(upgraded.template_version, 2);
        assert_eq!(upgraded.fields.category.as_deref(), Some("other"));
        assert!(validate_draft(s.registry(), &upgraded, &ContractRules::default(), NOW).is_ok());
        assert!(matches!(
            s.upgrade_template("alice", d.id, upgraded.revision, 1),
            Err(DraftError::Template(TemplateError::Downgrade { .. }))
        ));
    }

    #[test]
    fn sealed_payload_round_trips_through_json() {
        let s = store();
        let d = filled(&s, "alice", 2);
        let d = s
            .submit("alice", d.id, d.revision, &ContractRules::default(), NOW)
            .unwrap();
        let sealed = s
            .publish("alice", d.id, &ContractRules::default(), NOW)
            .unwrap();
        let back: SealedPayload =
            serde_json::from_str(&serde_json::to_string(&sealed).unwrap()).unwrap();
        assert_eq!(back, sealed);
        assert!(payload_is_intact(&back));
    }
}
