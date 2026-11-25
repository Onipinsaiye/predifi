use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum allowed length for a feedback message (in characters).
pub const MAX_MESSAGE_LEN: usize = 4000;
/// Minimum allowed length for a feedback message (in characters).
pub const MIN_MESSAGE_LEN: usize = 10;
/// Maximum allowed length for optional diagnostic metadata (in characters).
pub const MAX_METADATA_LEN: usize = 2000;
/// Maximum allowed length for the optional route/context field (in characters).
pub const MAX_ROUTE_LEN: usize = 256;

/// Categories of product feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackCategory {
    Bug,
    FeatureRequest,
    Usability,
    Performance,
    Documentation,
    Other,
}

impl FeedbackCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            FeedbackCategory::Bug => "bug",
            FeedbackCategory::FeatureRequest => "feature_request",
            FeedbackCategory::Usability => "usability",
            FeedbackCategory::Performance => "performance",
            FeedbackCategory::Documentation => "documentation",
            FeedbackCategory::Other => "other",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "bug" => Some(FeedbackCategory::Bug),
            "feature_request" => Some(FeedbackCategory::FeatureRequest),
            "usability" => Some(FeedbackCategory::Usability),
            "performance" => Some(FeedbackCategory::Performance),
            "documentation" => Some(FeedbackCategory::Documentation),
            "other" => Some(FeedbackCategory::Other),
            _ => None,
        }
    }
}

impl fmt::Display for FeedbackCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Raw feedback submission as received from the client.
#[derive(Debug, Clone, Deserialize)]
pub struct FeedbackSubmission {
    /// Category of the feedback. Required.
    pub category: Option<String>,
    /// Free-text feedback message. Required.
    pub message: String,
    /// Whether the user consented to submitting feedback. Required to be true.
    #[serde(default)]
    pub consent: bool,
    /// Optional diagnostic metadata (e.g. app version, OS). Opt-in.
    pub diagnostic_metadata: Option<String>,
    /// Optional route/context where the feedback originated.
    pub route: Option<String>,
    /// Optional app version string.
    pub app_version: Option<String>,
}

/// A validated, sanitized feedback record ready for persistence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedFeedback {
    pub category: FeedbackCategory,
    pub message: String,
    pub diagnostic_metadata: Option<String>,
    pub route: Option<String>,
    pub app_version: Option<String>,
}

/// Errors produced while validating a feedback submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedbackError {
    MissingCategory,
    UnknownCategory(String),
    MessageTooShort { min: usize },
    MessageTooLong { max: usize },
    MetadataTooLong { max: usize },
    RouteTooLong { max: usize },
    ConsentRequired,
    SecretDetected { field: &'static str },
}

impl fmt::Display for FeedbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeedbackError::MissingCategory => write!(f, "category is required"),
            FeedbackError::UnknownCategory(c) => write!(f, "unknown category: {}", c),
            FeedbackError::MessageTooShort { min } => {
                write!(f, "message must be at least {} characters", min)
            }
            FeedbackError::MessageTooLong { max } => {
                write!(f, "message must be at most {} characters", max)
            }
            FeedbackError::MetadataTooLong { max } => {
                write!(f, "diagnostic metadata must be at most {} characters", max)
            }
            FeedbackError::RouteTooLong { max } => {
                write!(f, "route must be at most {} characters", max)
            }
            FeedbackError::ConsentRequired => write!(f, "consent is required to submit feedback"),
            FeedbackError::SecretDetected { field } => {
                write!(f, "secret-like content detected in field: {}", field)
            }
        }
    }
}

impl std::error::Error for FeedbackError {}

/// Returns true if the given character is a valid Stellar StrKey base32 character.
fn is_strkey_char(c: char) -> bool {
    matches!(c, 'A'..='Z' | '2'..='7')
}

/// Returns true if the string looks like a Stellar StrKey (public key, muxed account, or seed).
///
/// StrKeys are base32-encoded and begin with a version byte that renders as a known prefix:
/// - `G` for ed25519 public keys (56 chars)
/// - `M` for muxed accounts (69 chars)
/// - `S` for secret seeds (56 chars)
/// - `C` for contract addresses (56 chars)
fn looks_like_strkey(s: &str) -> bool {
    let len = s.chars().count();
    if len != 56 && len != 69 {
        return false;
    }
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !matches!(first, 'G' | 'M' | 'S' | 'C') {
        return false;
    }
    s.chars().all(is_strkey_char)
}

/// Replaces Stellar StrKey-like strings (wallet addresses, seeds, contract IDs) with `[redacted]`.
///
/// This is applied by default to all free-text fields so that wallet addresses are not
/// persisted or forwarded unless the user explicitly opts in.
pub fn redact_wallet_addresses(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut token = String::new();

    let flush = |token: &mut String, out: &mut String| {
        if token.is_empty() {
            return;
        }
        if looks_like_strkey(token) {
            out.push_str("[redacted]");
        } else {
            out.push_str(token);
        }
        token.clear();
    };

    for c in input.chars() {
        if c.is_ascii_alphanumeric() {
            token.push(c);
        } else {
            flush(&mut token, &mut out);
            out.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

/// Detects secret-like content that must never be submitted.
///
/// This is a conservative heuristic: it rejects obvious private key / seed material
/// so that users do not accidentally attach secrets to a feedback report.
fn contains_secret_like(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();

    // Common secret markers.
    const MARKERS: &[&str] = &[
        "private key",
        "private_key",
        "privatekey",
        "secret key",
        "secret_key",
        "secretkey",
        "seed phrase",
        "seed_phrase",
        "mnemonic",
        "-----begin",
        "begin private key",
        "begin rsa private key",
        "begin ec private key",
        "begin openssh private key",
    ];
    if MARKERS.iter().any(|m| lower.contains(m)) {
        return true;
    }

    // Stellar secret seeds start with 'S' and are 56-char StrKeys.
    for token in input.split(|c: char| !c.is_ascii_alphanumeric()) {
        if token.len() == 56 && token.starts_with('S') && token.chars().all(is_strkey_char) {
            return true;
        }
    }

    false
}

/// Validates and sanitizes a raw feedback submission.
///
/// Validation rules:
/// - `category` must be present and known.
/// - `message` must be within `[MIN_MESSAGE_LEN, MAX_MESSAGE_LEN]` characters.
/// - `consent` must be `true`.
/// - `diagnostic_metadata` and `route` are optional but bounded in length.
/// - Secret-like content is rejected.
/// - Wallet addresses in free-text fields are redacted by default.
pub fn validate_submission(sub: FeedbackSubmission) -> Result<ValidatedFeedback, FeedbackError> {
    // Consent is mandatory.
    if !sub.consent {
        return Err(FeedbackError::ConsentRequired);
    }

    // Category is required and must be known.
    let category_raw = sub.category.ok_or(FeedbackError::MissingCategory)?;
    let category = FeedbackCategory::from_str_opt(category_raw.trim())
        .ok_or_else(|| FeedbackError::UnknownCategory(category_raw))?;

    // Message length limits (count characters, not bytes).
    let message_len = sub.message.chars().count();
    if message_len < MIN_MESSAGE_LEN {
        return Err(FeedbackError::MessageTooShort {
            min: MIN_MESSAGE_LEN,
        });
    }
    if message_len > MAX_MESSAGE_LEN {
        return Err(FeedbackError::MessageTooLong {
            max: MAX_MESSAGE_LEN,
        });
    }

    // Reject secret-like content before redaction so we never persist it.
    if contains_secret_like(&sub.message) {
        return Err(FeedbackError::SecretDetected { field: "message" });
    }

    // Optional diagnostic metadata.
    let diagnostic_metadata = match sub.diagnostic_metadata {
        Some(m) => {
            if m.chars().count() > MAX_METADATA_LEN {
                return Err(FeedbackError::MetadataTooLong {
                    max: MAX_METADATA_LEN,
                });
            }
            if contains_secret_like(&m) {
                return Err(FeedbackError::SecretDetected {
                    field: "diagnostic_metadata",
                });
            }
            let trimmed = m.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(redact_wallet_addresses(trimmed))
            }
        }
        None => None,
    };

    // Optional route.
    let route = match sub.route {
        Some(r) => {
            if r.chars().count() > MAX_ROUTE_LEN {
                return Err(FeedbackError::RouteTooLong {
                    max: MAX_ROUTE_LEN,
                });
            }
            if contains_secret_like(&r) {
                return Err(FeedbackError::SecretDetected { field: "route" });
            }
            let trimmed = r.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(redact_wallet_addresses(trimmed))
            }
        }
        None => None,
    };

    // Optional app version.
    let app_version = match sub.app_version {
        Some(v) => {
            if contains_secret_like(&v) {
                return Err(FeedbackError::SecretDetected {
                    field: "app_version",
                });
            }
            let trimmed = v.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        None => None,
    };

    Ok(ValidatedFeedback {
        category,
        message: redact_wallet_addresses(sub.message.trim()),
        diagnostic_metadata,
        route,
        app_version,
    })
}

/// A persisted feedback record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackRecord {
    pub id: i64,
    pub category: FeedbackCategory,
    pub message: String,
    pub diagnostic_metadata: Option<String>,
    pub route: Option<String>,
    pub app_version: Option<String>,
}

/// In-memory repository for feedback records.
///
/// In a production deployment this would be backed by a database; the API surface
/// (insert/list) is kept stable so the storage layer can be swapped out.
#[derive(Debug, Default)]
pub struct FeedbackRepository {
    records: Vec<FeedbackRecord>,
    next_id: i64,
}

impl FeedbackRepository {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            next_id: 1,
        }
    }

    /// Inserts a validated feedback record and returns the stored record.
    pub fn insert(&mut self, feedback: ValidatedFeedback) -> FeedbackRecord {
        let id = self.next_id;
        self.next_id += 1;
        let record = FeedbackRecord {
            id,
            category: feedback.category,
            message: feedback.message,
            diagnostic_metadata: feedback.diagnostic_metadata,
            route: feedback.route,
            app_version: feedback.app_version,
        };
        self.records.push(record.clone());
        record
    }

    /// Lists all stored feedback records, oldest first.
    pub fn list(&self) -> &[FeedbackRecord] {
        &self.records
    }

    /// Lists stored feedback records filtered by category.
    pub fn list_by_category(&self, category: FeedbackCategory) -> Vec<&FeedbackRecord> {
        self.records
            .iter()
            .filter(|r| r.category == category)
            .collect()
    }

    /// Returns the number of stored records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_submission() -> FeedbackSubmission {
        FeedbackSubmission {
            category: Some("bug".to_string()),
            message: "The submit button does not work on mobile.".to_string(),
            consent: true,
            diagnostic_metadata: None,
            route: None,
            app_version: None,
        }
    }

    #[test]
    fn accepts_valid_submission() {
        let result = validate_submission(valid_submission()).expect("should validate");
        assert_eq!(result.category, FeedbackCategory::Bug);
        assert_eq!(result.message, "The submit button does not work on mobile.");
    }

    #[test]
    fn rejects_missing_category() {
        let mut sub = valid_submission();
        sub.category = None;
        assert_eq!(
            validate_submission(sub),
            Err(FeedbackError::MissingCategory)
        );
    }

    #[test]
    fn rejects_unknown_category() {
        let mut sub = valid_submission();
        sub.category = Some("nonsense".to_string());
        assert_eq!(
            validate_submission(sub),
            Err(FeedbackError::UnknownCategory("nonsense".to_string()))
        );
    }

    #[test]
    fn rejects_message_too_short() {
        let mut sub = valid_submission();
        sub.message = "short".to_string();
        assert_eq!(
            validate_submission(sub),
            Err(FeedbackError::MessageTooShort {
                min: MIN_MESSAGE_LEN
            })
        );
    }

    #[test]
    fn rejects_message_too_long() {
        let mut sub = valid_submission();
        sub.message = "a".repeat(MAX_MESSAGE_LEN + 1);
        assert_eq!(
            validate_submission(sub),
            Err(FeedbackError::MessageTooLong {
                max: MAX_MESSAGE_LEN
            })
        );
    }

    #[test]
    fn accepts_message_at_max_length() {
        let mut sub = valid_submission();
        sub.message = "a".repeat(MAX_MESSAGE_LEN);
        assert!(validate_submission(sub).is_ok());
    }

    #[test]
    fn rejects_metadata_too_long() {
        let mut sub = valid_submission();
        sub.diagnostic_metadata = Some("x".repeat(MAX_METADATA_LEN + 1));
        assert_eq!(
            validate_submission(sub),
            Err(FeedbackError::MetadataTooLong {
                max: MAX_METADATA_LEN
            })
        );
    }

    #[test]
    fn rejects_route_too_long() {
        let mut sub = valid_submission();
        sub.route = Some("r".repeat(MAX_ROUTE_LEN + 1));
        assert_eq!(
            validate_submission(sub),
            Err(FeedbackError::RouteTooLong {
                max: MAX_ROUTE_LEN
            })
        );
    }

    #[test]
    fn rejects_without_consent() {
        let mut sub = valid_submission();
        sub.consent = false;
        assert_eq!(
            validate_submission(sub),
            Err(FeedbackError::ConsentRequired)
        );
    }

    #[test]
    fn redacts_stellar_public_key() {
        let key = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";
        let input = format!("My wallet is {} and it failed.", key);
        let redacted = redact_wallet_addresses(&input);
        assert!(!redacted.contains(key));
        assert!(redacted.contains("[redacted]"));
    }

    #[test]
    fn redacts_multiple_addresses() {
        let key1 = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";
        let key2 = "GBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
        let input = format!("from {} to {}", key1, key2);
        let redacted = redact_wallet_addresses(&input);
        assert!(!redacted.contains(key1));
        assert!(!redacted.contains(key2));
        assert_eq!(redacted.matches("[redacted]").count(), 2);
    }

    #[test]
    fn leaves_non_strkey_text_untouched() {
        let input = "Nothing to see here, just normal text 12345.";
        assert_eq!(redact_wallet_addresses(input), input);
    }

    #[test]
    fn redaction_applied_in_validated_message() {
        let mut sub = valid_submission();
        let key = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";
        sub.message = format!("Payment to {} failed repeatedly.", key);
        let result = validate_submission(sub).expect("should validate");
        assert!(!result.message.contains(key));
        assert!(result.message.contains("[redacted]"));
    }

    #[test]
    fn rejects_secret_seed_in_message() {
        let mut sub = valid_submission();
        sub.message = "Here is my seed SAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            .to_string();
        assert_eq!(
            validate_submission(sub),
            Err(FeedbackError::SecretDetected { field: "message" })
        );
    }

    #[test]
    fn rejects_private_key_marker() {
        let mut sub = valid_submission();
        sub.message = "attaching my private key for debugging".to_string();
        assert_eq!(
            validate_submission(sub),
            Err(FeedbackError::SecretDetected { field: "message" })
        );
    }

    #[test]
    fn rejects_secret_in_metadata() {
        let mut sub = valid_submission();
        sub.diagnostic_metadata = Some("seed phrase: foo bar baz".to_string());
        assert_eq!(
            validate_submission(sub),
            Err(FeedbackError::SecretDetected {
                field: "diagnostic_metadata"
            })
        );
    }

    #[test]
    fn empty_optional_fields_become_none() {
        let mut sub = valid_submission();
        sub.diagnostic_metadata = Some("   ".to_string());
        sub.route = Some("".to_string());
        sub.app_version = Some("  ".to_string());
        let result = validate_submission(sub).expect("should validate");
        assert!(result.diagnostic_metadata.is_none());
        assert!(result.route.is_none());
        assert!(result.app_version.is_none());
    }

    #[test]
    fn repository_insert_and_list() {
        let mut repo = FeedbackRepository::new();
        assert!(repo.is_empty());

        let record = repo.insert(validate_submission(valid_submission()).unwrap());
        assert_eq!(record.id, 1);
        assert_eq!(repo.len(), 1);

        let mut sub = valid_submission();
        sub.category = Some("feature_request".to_string());
        sub.message = "Please add dark mode to the dashboard.".to_string();
        let record2 = repo.insert(validate_submission(sub).unwrap());
        assert_eq!(record2.id, 2);

        let all = repo.list();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].category, FeedbackCategory::Bug);
        assert_eq!(all[1].category, FeedbackCategory::FeatureRequest);

        let bugs = repo.list_by_category(FeedbackCategory::Bug);
        assert_eq!(bugs.len(), 1);
        assert_eq!(bugs[0].id, 1);
    }

    #[test]
    fn category_round_trips() {
        for cat in [
            FeedbackCategory::Bug,
            FeedbackCategory::FeatureRequest,
            FeedbackCategory::Usability,
            FeedbackCategory::Performance,
            FeedbackCategory::Documentation,
            FeedbackCategory::Other,
        ] {
            assert_eq!(FeedbackCategory::from_str_opt(cat.as_str()), Some(cat));
        }
        assert_eq!(FeedbackCategory::from_str_opt("bogus"), None);
    }
}