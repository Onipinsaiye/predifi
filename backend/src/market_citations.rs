//! Structured, verifiable market source citations (#239).
//!
//! Citation metadata lives apart from the user-facing question text. Every
//! create, update and resolution appends an immutable snapshot; earlier
//! snapshots are never rewritten.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_URL_LEN: usize = 2048;
pub const MAX_NAME_LEN: usize = 120;
pub const MIN_RULE_LEN: usize = 10;
pub const MAX_RULE_LEN: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCitation {
    pub source_url: Option<String>,
    pub source_name: Option<String>,
    pub resolution_rule: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CitationIssue {
    pub field: &'static str,
    pub message: String,
}

fn issue(field: &'static str, message: impl Into<String>) -> CitationIssue {
    CitationIssue {
        field,
        message: message.into(),
    }
}

fn is_valid_http_url(url: &str) -> bool {
    let rest = match url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        Some(rest) => rest,
        None => return false,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    !host.is_empty()
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
        && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

impl SourceCitation {
    /// Normalise surrounding whitespace and treat blank optionals as absent.
    /// The stored text is otherwise kept verbatim so it renders exactly.
    pub fn normalized(&self) -> SourceCitation {
        let clean = |v: &Option<String>| {
            v.as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        SourceCitation {
            source_url: clean(&self.source_url),
            source_name: clean(&self.source_name),
            resolution_rule: self.resolution_rule.trim().to_string(),
        }
    }

    pub fn validate(&self) -> Result<(), Vec<CitationIssue>> {
        let c = self.normalized();
        let mut issues = Vec::new();

        if c.source_url.is_none() && c.source_name.is_none() {
            issues.push(issue(
                "source",
                "a source URL or a named source is required",
            ));
        }
        if let Some(url) = &c.source_url {
            if url.len() > MAX_URL_LEN || !is_valid_http_url(url) {
                issues.push(issue(
                    "source_url",
                    "source URL must be a valid http(s) URL up to 2048 characters",
                ));
            }
        }
        if let Some(name) = &c.source_name {
            if name.chars().count() > MAX_NAME_LEN {
                issues.push(issue(
                    "source_name",
                    format!("source name must be at most {MAX_NAME_LEN} characters"),
                ));
            }
        }
        let rule_len = c.resolution_rule.chars().count();
        if !(MIN_RULE_LEN..=MAX_RULE_LEN).contains(&rule_len) {
            issues.push(issue(
                "resolution_rule",
                format!("resolution rule must be {MIN_RULE_LEN}-{MAX_RULE_LEN} characters"),
            ));
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }

    /// Stable content digest over the normalised citation.
    pub fn digest(&self) -> String {
        let c = self.normalized();
        let canonical = serde_json::json!([c.source_url, c.source_name, c.resolution_rule]);
        let hash = Sha256::digest(canonical.to_string().as_bytes());
        hash.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Exact display data for detail pages: no rewriting of the rule text.
    pub fn render(&self) -> RenderedCitation {
        let c = self.normalized();
        RenderedCitation {
            label: c
                .source_name
                .clone()
                .or_else(|| c.source_url.clone())
                .unwrap_or_default(),
            href: c.source_url,
            resolution_rule: c.resolution_rule,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenderedCitation {
    pub label: String,
    pub href: Option<String>,
    pub resolution_rule: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotStage {
    Create,
    Update,
    Resolution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationSnapshot {
    pub sequence: u32,
    pub stage: SnapshotStage,
    pub citation: SourceCitation,
    pub digest: String,
    pub captured_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryError {
    Invalid(Vec<CitationIssue>),
    /// A citation was recorded before the market's create snapshot.
    MissingCreate,
    /// The market was already resolved; history is closed.
    Resolved,
}

/// Append-only citation history for one market.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationHistory {
    snapshots: Vec<CitationSnapshot>,
}

impl CitationHistory {
    pub fn snapshots(&self) -> &[CitationSnapshot] {
        &self.snapshots
    }

    pub fn current(&self) -> Option<&CitationSnapshot> {
        self.snapshots.last()
    }

    fn push(
        &mut self,
        stage: SnapshotStage,
        citation: SourceCitation,
        at: i64,
    ) -> &CitationSnapshot {
        let citation = citation.normalized();
        let snapshot = CitationSnapshot {
            sequence: self.snapshots.len() as u32 + 1,
            stage,
            digest: citation.digest(),
            citation,
            captured_at: at,
        };
        self.snapshots.push(snapshot);
        self.snapshots.last().expect("just pushed")
    }

    pub fn record_create(
        &mut self,
        citation: SourceCitation,
        at: i64,
    ) -> Result<&CitationSnapshot, HistoryError> {
        citation.validate().map_err(HistoryError::Invalid)?;
        Ok(self.push(SnapshotStage::Create, citation, at))
    }

    pub fn record_update(
        &mut self,
        citation: SourceCitation,
        at: i64,
    ) -> Result<&CitationSnapshot, HistoryError> {
        if self.snapshots.is_empty() {
            return Err(HistoryError::MissingCreate);
        }
        if self.is_resolved() {
            return Err(HistoryError::Resolved);
        }
        citation.validate().map_err(HistoryError::Invalid)?;
        Ok(self.push(SnapshotStage::Update, citation, at))
    }

    /// Freeze the citation in force at resolution time, verbatim.
    pub fn record_resolution(&mut self, at: i64) -> Result<&CitationSnapshot, HistoryError> {
        let current = self
            .snapshots
            .last()
            .ok_or(HistoryError::MissingCreate)?
            .citation
            .clone();
        if self.is_resolved() {
            return Err(HistoryError::Resolved);
        }
        Ok(self.push(SnapshotStage::Resolution, current, at))
    }

    fn is_resolved(&self) -> bool {
        self.snapshots
            .iter()
            .any(|s| s.stage == SnapshotStage::Resolution)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn citation() -> SourceCitation {
        SourceCitation {
            source_url: Some("https://www.nasa.gov/launch-schedule".into()),
            source_name: Some("NASA".into()),
            resolution_rule: "Resolves YES if NASA reports a successful launch.".into(),
        }
    }

    #[test]
    fn accepts_url_or_name_alone() {
        let mut c = citation();
        c.source_name = None;
        assert!(c.validate().is_ok());
        let mut c = citation();
        c.source_url = None;
        assert!(c.validate().is_ok());
    }

    #[test]
    fn requires_a_source() {
        let mut c = citation();
        c.source_url = Some("   ".into());
        c.source_name = None;
        let issues = c.validate().unwrap_err();
        assert!(issues.iter().any(|i| i.field == "source"));
    }

    #[test]
    fn rejects_bad_urls() {
        for bad in ["ftp://example.com", "https://", "javascript:alert(1)", "https://a b.com"] {
            let mut c = citation();
            c.source_url = Some(bad.into());
            let issues = c.validate().unwrap_err();
            assert!(issues.iter().any(|i| i.field == "source_url"), "{bad}");
        }
    }

    #[test]
    fn rule_length_is_bounded() {
        let mut c = citation();
        c.resolution_rule = "short".into();
        assert!(c.validate().is_err());
        c.resolution_rule = "x".repeat(MAX_RULE_LEN + 1);
        assert!(c.validate().is_err());
        c.resolution_rule = "x".repeat(MAX_RULE_LEN);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn render_preserves_exact_text() {
        let rendered = citation().render();
        assert_eq!(rendered.label, "NASA");
        assert_eq!(rendered.href.as_deref(), Some("https://www.nasa.gov/launch-schedule"));
        assert_eq!(
            rendered.resolution_rule,
            "Resolves YES if NASA reports a successful launch."
        );
    }

    #[test]
    fn render_falls_back_to_url_label() {
        let mut c = citation();
        c.source_name = None;
        assert_eq!(c.render().label, "https://www.nasa.gov/launch-schedule");
    }

    #[test]
    fn digest_ignores_padding_but_tracks_content() {
        let mut padded = citation();
        padded.resolution_rule = format!("  {}  ", padded.resolution_rule);
        assert_eq!(padded.digest(), citation().digest());
        let mut changed = citation();
        changed.source_name = Some("ESA".into());
        assert_ne!(changed.digest(), citation().digest());
    }

    #[test]
    fn history_snapshots_are_immutable_through_lifecycle() {
        let mut history = CitationHistory::default();
        let original = citation();
        history.record_create(original.clone(), 100).unwrap();

        let mut updated = citation();
        updated.source_name = Some("NASA Launch Services".into());
        history.record_update(updated.clone(), 200).unwrap();
        history.record_resolution(300).unwrap();

        let snaps = history.snapshots();
        assert_eq!(snaps.len(), 3);
        assert_eq!(snaps[0].stage, SnapshotStage::Create);
        assert_eq!(snaps[0].citation, original);
        assert_eq!(snaps[1].citation, updated);
        assert_eq!(snaps[2].stage, SnapshotStage::Resolution);
        assert_eq!(snaps[2].citation, updated);
        assert_eq!(snaps[2].digest, snaps[1].digest);
        assert_eq!(
            snaps.iter().map(|s| s.sequence).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn history_rejects_invalid_and_post_resolution_changes() {
        let mut history = CitationHistory::default();
        assert_eq!(
            history.record_update(citation(), 1).unwrap_err(),
            HistoryError::MissingCreate
        );
        let mut bad = citation();
        bad.resolution_rule = String::new();
        assert!(matches!(
            history.record_create(bad, 1),
            Err(HistoryError::Invalid(_))
        ));
        history.record_create(citation(), 1).unwrap();
        history.record_resolution(2).unwrap();
        assert_eq!(
            history.record_update(citation(), 3).unwrap_err(),
            HistoryError::Resolved
        );
        assert_eq!(history.record_resolution(4).unwrap_err(), HistoryError::Resolved);
        assert_eq!(history.snapshots().len(), 2);
    }

    #[test]
    fn history_round_trips_through_json() {
        let mut history = CitationHistory::default();
        history.record_create(citation(), 1).unwrap();
        let json = serde_json::to_string(&history).unwrap();
        assert_eq!(serde_json::from_str::<CitationHistory>(&json).unwrap(), history);
    }
}
