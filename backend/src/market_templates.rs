//! Versioned market-creation templates (#240).
//!
//! A template pins typed defaults and the fields a creator must supply. Every
//! draft is bound to the exact `(template_id, version)` it was created from, so
//! later template changes never reinterpret drafts or published markets.

use std::fmt;

use serde::{Deserialize, Serialize};

pub const BINARY_TEMPLATE_ID: &str = "binary-yes-no";

/// Fields a template may require a creator to provide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftField {
    Question,
    Outcomes,
    EndTime,
    Category,
    Citation,
}

impl DraftField {
    pub fn as_str(&self) -> &'static str {
        match self {
            DraftField::Question => "question",
            DraftField::Outcomes => "outcomes",
            DraftField::EndTime => "end_time",
            DraftField::Category => "category",
            DraftField::Citation => "citation",
        }
    }
}

/// Typed defaults applied when a draft is created from a template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateDefaults {
    pub outcomes: Vec<String>,
    /// Default market duration in seconds, used to derive `end_time`.
    pub duration_secs: u64,
    pub category: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketTemplate {
    pub id: String,
    pub version: u32,
    pub defaults: TemplateDefaults,
    pub required_fields: Vec<DraftField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    UnknownTemplate(String),
    UnsupportedVersion {
        id: String,
        requested: u32,
        supported: Vec<u32>,
    },
    Downgrade {
        id: String,
        from: u32,
        to: u32,
    },
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TemplateError::UnknownTemplate(id) => write!(f, "unknown template '{id}'"),
            TemplateError::UnsupportedVersion {
                id,
                requested,
                supported,
            } => write!(
                f,
                "template '{id}' version {requested} is not supported (supported versions: {supported:?})"
            ),
            TemplateError::Downgrade { id, from, to } => {
                write!(f, "cannot downgrade template '{id}' from version {from} to {to}")
            }
        }
    }
}

impl std::error::Error for TemplateError {}

/// Immutable catalogue of every template version ever released. Versions are
/// only ever appended; an existing `(id, version)` never changes meaning.
#[derive(Debug, Clone)]
pub struct TemplateRegistry {
    templates: Vec<MarketTemplate>,
}

impl Default for TemplateRegistry {
    fn default() -> Self {
        let yes_no = || vec!["Yes".to_string(), "No".to_string()];
        Self {
            templates: vec![
                MarketTemplate {
                    id: BINARY_TEMPLATE_ID.into(),
                    version: 1,
                    defaults: TemplateDefaults {
                        outcomes: yes_no(),
                        duration_secs: 7 * 24 * 3600,
                        category: None,
                    },
                    required_fields: vec![
                        DraftField::Question,
                        DraftField::EndTime,
                        DraftField::Citation,
                    ],
                },
                // v2 additionally requires a governed category.
                MarketTemplate {
                    id: BINARY_TEMPLATE_ID.into(),
                    version: 2,
                    defaults: TemplateDefaults {
                        outcomes: yes_no(),
                        duration_secs: 7 * 24 * 3600,
                        category: Some("other".into()),
                    },
                    required_fields: vec![
                        DraftField::Question,
                        DraftField::EndTime,
                        DraftField::Category,
                        DraftField::Citation,
                    ],
                },
            ],
        }
    }
}

impl TemplateRegistry {
    pub fn supported_versions(&self, id: &str) -> Vec<u32> {
        self.templates
            .iter()
            .filter(|t| t.id == id)
            .map(|t| t.version)
            .collect()
    }

    /// Resolve an exact template version, rejecting unknown ids and versions.
    pub fn resolve(&self, id: &str, version: u32) -> Result<&MarketTemplate, TemplateError> {
        let supported = self.supported_versions(id);
        if supported.is_empty() {
            return Err(TemplateError::UnknownTemplate(id.to_string()));
        }
        self.templates
            .iter()
            .find(|t| t.id == id && t.version == version)
            .ok_or(TemplateError::UnsupportedVersion {
                id: id.to_string(),
                requested: version,
                supported,
            })
    }

    pub fn latest(&self, id: &str) -> Result<&MarketTemplate, TemplateError> {
        let latest = self
            .supported_versions(id)
            .into_iter()
            .max()
            .ok_or_else(|| TemplateError::UnknownTemplate(id.to_string()))?;
        self.resolve(id, latest)
    }

    /// Validate an explicit upgrade request. Upgrades only move forward and
    /// the target must exist; callers re-validate the draft against the new
    /// template's required fields afterwards.
    pub fn check_upgrade(
        &self,
        id: &str,
        from: u32,
        to: u32,
    ) -> Result<&MarketTemplate, TemplateError> {
        self.resolve(id, from)?;
        let target = self.resolve(id, to)?;
        if to < from {
            return Err(TemplateError::Downgrade {
                id: id.to_string(),
                from,
                to,
            });
        }
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_exact_versions() {
        let registry = TemplateRegistry::default();
        assert_eq!(registry.resolve(BINARY_TEMPLATE_ID, 1).unwrap().version, 1);
        assert_eq!(registry.latest(BINARY_TEMPLATE_ID).unwrap().version, 2);
    }

    #[test]
    fn rejects_unsupported_version_with_supported_list() {
        let err = TemplateRegistry::default()
            .resolve(BINARY_TEMPLATE_ID, 99)
            .unwrap_err();
        assert_eq!(
            err,
            TemplateError::UnsupportedVersion {
                id: BINARY_TEMPLATE_ID.into(),
                requested: 99,
                supported: vec![1, 2],
            }
        );
        assert!(err.to_string().contains("version 99 is not supported"));
    }

    #[test]
    fn rejects_unknown_template() {
        assert_eq!(
            TemplateRegistry::default().resolve("nope", 1).unwrap_err(),
            TemplateError::UnknownTemplate("nope".into())
        );
    }

    #[test]
    fn upgrades_forward_only() {
        let registry = TemplateRegistry::default();
        assert!(registry.check_upgrade(BINARY_TEMPLATE_ID, 1, 2).is_ok());
        assert!(matches!(
            registry.check_upgrade(BINARY_TEMPLATE_ID, 2, 1),
            Err(TemplateError::Downgrade { .. })
        ));
        assert!(matches!(
            registry.check_upgrade(BINARY_TEMPLATE_ID, 1, 3),
            Err(TemplateError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn newer_version_only_adds_requirements() {
        let registry = TemplateRegistry::default();
        let v1 = registry.resolve(BINARY_TEMPLATE_ID, 1).unwrap();
        let v2 = registry.resolve(BINARY_TEMPLATE_ID, 2).unwrap();
        assert!(v1.required_fields.iter().all(|f| v2.required_fields.contains(f)));
        assert!(v2.required_fields.contains(&DraftField::Category));
        assert!(!v1.required_fields.contains(&DraftField::Category));
    }

    #[test]
    fn template_round_trips_through_json() {
        let registry = TemplateRegistry::default();
        for version in registry.supported_versions(BINARY_TEMPLATE_ID) {
            let template = registry.resolve(BINARY_TEMPLATE_ID, version).unwrap();
            let json = serde_json::to_string(template).unwrap();
            let back: MarketTemplate = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, template);
        }
    }
}
