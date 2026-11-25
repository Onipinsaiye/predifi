//! Append-only, tamper-evident market metadata change history (#247).
//!
//! Every metadata mutation records the actor, role, origin, mandatory audit
//! reason, deterministic field-level diffs, and a full normalized snapshot
//! chained via domain-separated SHA-256 hashes. History is strictly
//! append-only: rollbacks and post-resolution corrections append new versions
//! referencing the superseded version rather than rewriting prior entries.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const MAX_HISTORY_PAGE_SIZE: usize = 100;
pub const MAX_CHANGE_REASON_LEN: usize = 500;

const GENESIS_PREV_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";
const HISTORY_HASH_DOMAIN: &str = "predifi-market-metadata-history-v1";

/// Authorization role of an actor recording a market metadata change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataActorRole {
    Creator,
    Translator,
    Moderator,
    Admin,
    System,
}

/// Authenticated actor responsible for a metadata revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataActor {
    pub id: String,
    pub role: MetadataActorRole,
}

impl MetadataActor {
    pub fn new(id: impl Into<String>, role: MetadataActorRole) -> Self {
        Self {
            id: id.into(),
            role,
        }
    }
}

/// Classification of how a metadata change originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeOrigin {
    Creation,
    CreatorEdit,
    ModeratorEdit,
    LocalizationUpdate,
    Correction,
    Rollback,
}

/// Point-in-time snapshot of a market's mutable and immutable metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketMetadataSnapshot {
    pub canonical_question: String,
    pub description: String,
    pub category: String,
    pub tags: Vec<String>,
    pub localized_descriptions: BTreeMap<String, String>,
    pub resolution_source: Option<String>,
    pub close_time_unix_secs: i64,
}

impl MarketMetadataSnapshot {
    /// Normalize whitespace, deduplicate/sort tags, and strip empty optional values.
    pub fn normalized(&self) -> Self {
        let tags: BTreeSet<String> = self
            .tags
            .iter()
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect();

        let localized_descriptions: BTreeMap<String, String> = self
            .localized_descriptions
            .iter()
            .filter_map(|(k, v)| {
                let key = k.trim().to_string();
                let val = v.trim().to_string();
                if key.is_empty() || val.is_empty() {
                    None
                } else {
                    Some((key, val))
                }
            })
            .collect();

        let resolution_source = self
            .resolution_source
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        Self {
            canonical_question: self.canonical_question.trim().to_string(),
            description: self.description.trim().to_string(),
            category: self.category.trim().to_ascii_lowercase(),
            tags: tags.into_iter().collect(),
            localized_descriptions,
            resolution_source,
            close_time_unix_secs: self.close_time_unix_secs,
        }
    }

    fn validate(&self) -> Result<(), MetadataHistoryError> {
        if self.canonical_question.is_empty() {
            return Err(MetadataHistoryError::Validation {
                field: "canonical_question",
                message: "must not be empty".to_string(),
            });
        }
        if self.description.is_empty() {
            return Err(MetadataHistoryError::Validation {
                field: "description",
                message: "must not be empty".to_string(),
            });
        }
        if self.category.is_empty() {
            return Err(MetadataHistoryError::Validation {
                field: "category",
                message: "must not be empty".to_string(),
            });
        }
        if self.close_time_unix_secs <= 0 {
            return Err(MetadataHistoryError::Validation {
                field: "close_time_unix_secs",
                message: "must be a positive Unix timestamp".to_string(),
            });
        }
        Ok(())
    }
}

/// Deterministic before/after record for one modified metadata field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataFieldDiff {
    pub field: String,
    pub before: Value,
    pub after: Value,
}

/// Compute deterministic, sorted field-level diffs between two snapshots.
pub fn compute_metadata_diff(
    before: &MarketMetadataSnapshot,
    after: &MarketMetadataSnapshot,
) -> Vec<MetadataFieldDiff> {
    let b = before.normalized();
    let a = after.normalized();
    let mut diffs = Vec::new();

    if b.canonical_question != a.canonical_question {
        diffs.push(MetadataFieldDiff {
            field: "canonical_question".into(),
            before: json!(b.canonical_question),
            after: json!(a.canonical_question),
        });
    }
    if b.category != a.category {
        diffs.push(MetadataFieldDiff {
            field: "category".into(),
            before: json!(b.category),
            after: json!(a.category),
        });
    }
    if b.close_time_unix_secs != a.close_time_unix_secs {
        diffs.push(MetadataFieldDiff {
            field: "close_time_unix_secs".into(),
            before: json!(b.close_time_unix_secs),
            after: json!(a.close_time_unix_secs),
        });
    }
    if b.description != a.description {
        diffs.push(MetadataFieldDiff {
            field: "description".into(),
            before: json!(b.description),
            after: json!(a.description),
        });
    }

    let all_locales: BTreeSet<&String> = b
        .localized_descriptions
        .keys()
        .chain(a.localized_descriptions.keys())
        .collect();
    for locale in all_locales {
        let prev = b.localized_descriptions.get(locale);
        let next = a.localized_descriptions.get(locale);
        if prev != next {
            diffs.push(MetadataFieldDiff {
                field: format!("localized_descriptions.{locale}"),
                before: json!(prev),
                after: json!(next),
            });
        }
    }

    if b.resolution_source != a.resolution_source {
        diffs.push(MetadataFieldDiff {
            field: "resolution_source".into(),
            before: json!(b.resolution_source),
            after: json!(a.resolution_source),
        });
    }
    if b.tags != a.tags {
        diffs.push(MetadataFieldDiff {
            field: "tags".into(),
            before: json!(b.tags),
            after: json!(a.tags),
        });
    }

    diffs
}

/// One immutable entry in a market's append-only metadata change history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataChangeEntry {
    pub version: u64,
    pub market_id: u64,
    pub timestamp_unix_ms: i64,
    pub actor: String,
    pub actor_role: MetadataActorRole,
    pub origin: ChangeOrigin,
    pub reason: String,
    pub supersedes_version: Option<u64>,
    pub diffs: Vec<MetadataFieldDiff>,
    pub snapshot: MarketMetadataSnapshot,
    pub prev_hash: String,
    pub entry_hash: String,
}

/// Compute the deterministic SHA-256 hash of a metadata history entry.
pub fn compute_entry_hash(entry: &MetadataChangeEntry) -> String {
    let mut clone = entry.clone();
    clone.entry_hash.clear();
    let canonical = serde_json::to_string(&clone).expect("history entry serializes");
    let mut hasher = Sha256::new();
    hasher.update(HISTORY_HASH_DOMAIN.as_bytes());
    hasher.update(b"\n");
    hasher.update(canonical.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Request payload for appending a metadata update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataUpdateRequest {
    pub expected_version: u64,
    pub origin: ChangeOrigin,
    pub reason: String,
    pub timestamp_unix_ms: i64,
    pub next_snapshot: MarketMetadataSnapshot,
}

/// Bounded page of metadata change entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryPage {
    pub entries: Vec<MetadataChangeEntry>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

/// Summary returned by [`MarketMetadataHistory::verify_integrity`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryIntegrityReport {
    pub ok: bool,
    pub checked_entries: usize,
    pub head_version: u64,
    pub head_hash: Option<String>,
}

/// Errors produced at the metadata history boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataHistoryError {
    Validation {
        field: &'static str,
        message: String,
    },
    NotInitialized,
    AlreadyInitialized,
    Unauthorized {
        actor: String,
        action: &'static str,
    },
    ImmutableFieldModified {
        field: String,
    },
    MarketResolved {
        market_id: u64,
    },
    NoChangesDetected,
    VersionConflict {
        expected: u64,
        actual: u64,
    },
    VersionNotFound {
        version: u64,
    },
    NonMonotonicTimestamp {
        previous_ms: i64,
        provided_ms: i64,
    },
    IntegrityFailure {
        version: u64,
        detail: String,
    },
}

impl fmt::Display for MetadataHistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetadataHistoryError::Validation { field, message } => {
                write!(f, "invalid field '{field}': {message}")
            }
            MetadataHistoryError::NotInitialized => {
                write!(f, "market metadata history has no initial creation record")
            }
            MetadataHistoryError::AlreadyInitialized => {
                write!(f, "market metadata history is already initialized")
            }
            MetadataHistoryError::Unauthorized { actor, action } => {
                write!(f, "actor '{actor}' is not authorized to {action}")
            }
            MetadataHistoryError::ImmutableFieldModified { field } => {
                write!(f, "field '{field}' is immutable after the market is published")
            }
            MetadataHistoryError::MarketResolved { market_id } => {
                write!(f, "market #{market_id} is resolved; only admin corrections are permitted")
            }
            MetadataHistoryError::NoChangesDetected => {
                write!(f, "metadata update contains no changes")
            }
            MetadataHistoryError::VersionConflict { expected, actual } => {
                write!(f, "metadata version conflict: expected {expected}, actual {actual}")
            }
            MetadataHistoryError::VersionNotFound { version } => {
                write!(f, "metadata history version {version} not found")
            }
            MetadataHistoryError::NonMonotonicTimestamp {
                previous_ms,
                provided_ms,
            } => write!(
                f,
                "timestamp {provided_ms} precedes previous entry timestamp {previous_ms}"
            ),
            MetadataHistoryError::IntegrityFailure { version, detail } => {
                write!(f, "history integrity failure at version {version}: {detail}")
            }
        }
    }
}

impl std::error::Error for MetadataHistoryError {}

fn validate_non_empty(
    field: &'static str,
    raw: &str,
    max_len: usize,
) -> Result<String, MetadataHistoryError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(MetadataHistoryError::Validation {
            field,
            message: "must not be empty".to_string(),
        });
    }
    if trimmed.chars().count() > max_len {
        return Err(MetadataHistoryError::Validation {
            field,
            message: format!("must be at most {max_len} characters"),
        });
    }
    Ok(trimmed.to_string())
}

fn is_post_publish_immutable(field: &str) -> bool {
    matches!(field, "canonical_question" | "close_time_unix_secs")
}

/// Append-only metadata change ledger for a single market.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketMetadataHistory {
    pub market_id: u64,
    pub creator_id: String,
    pub published: bool,
    pub resolved: bool,
    entries: Vec<MetadataChangeEntry>,
}

impl MarketMetadataHistory {
    pub fn new(
        market_id: u64,
        creator_id: impl Into<String>,
    ) -> Result<Self, MetadataHistoryError> {
        if market_id == 0 {
            return Err(MetadataHistoryError::Validation {
                field: "market_id",
                message: "must be positive".to_string(),
            });
        }
        let creator_id = validate_non_empty("creator_id", &creator_id.into(), 128)?;
        Ok(Self {
            market_id,
            creator_id,
            published: false,
            resolved: false,
            entries: Vec::new(),
        })
    }

    /// Record the initial creation snapshot (version 1).
    pub fn record_initial(
        &mut self,
        actor: &MetadataActor,
        snapshot: MarketMetadataSnapshot,
        timestamp_unix_ms: i64,
    ) -> Result<&MetadataChangeEntry, MetadataHistoryError> {
        if !self.entries.is_empty() {
            return Err(MetadataHistoryError::AlreadyInitialized);
        }
        let actor_id = validate_non_empty("actor.id", &actor.id, 128)?;
        let authorized = match actor.role {
            MetadataActorRole::Admin | MetadataActorRole::System => true,
            MetadataActorRole::Creator => actor_id == self.creator_id,
            MetadataActorRole::Translator | MetadataActorRole::Moderator => false,
        };
        if !authorized {
            return Err(MetadataHistoryError::Unauthorized {
                actor: actor_id,
                action: "record initial market metadata",
            });
        }

        let normalized = snapshot.normalized();
        normalized.validate()?;

        let mut entry = MetadataChangeEntry {
            version: 1,
            market_id: self.market_id,
            timestamp_unix_ms,
            actor: actor_id,
            actor_role: actor.role,
            origin: ChangeOrigin::Creation,
            reason: "initial market creation".to_string(),
            supersedes_version: None,
            diffs: Vec::new(),
            snapshot: normalized,
            prev_hash: GENESIS_PREV_HASH.to_string(),
            entry_hash: String::new(),
        };
        entry.entry_hash = compute_entry_hash(&entry);
        self.entries.push(entry);
        Ok(self.entries.last().expect("just pushed"))
    }

    /// Mark the market published, locking core contract fields against edits.
    pub fn mark_published(&mut self, actor: &MetadataActor) -> Result<(), MetadataHistoryError> {
        if self.entries.is_empty() {
            return Err(MetadataHistoryError::NotInitialized);
        }
        let authorized = match actor.role {
            MetadataActorRole::Admin | MetadataActorRole::System => true,
            MetadataActorRole::Creator => actor.id == self.creator_id,
            _ => false,
        };
        if !authorized {
            return Err(MetadataHistoryError::Unauthorized {
                actor: actor.id.clone(),
                action: "publish market metadata",
            });
        }
        self.published = true;
        Ok(())
    }

    /// Mark the market resolved, closing normal metadata edits.
    pub fn mark_resolved(&mut self, actor: &MetadataActor) -> Result<(), MetadataHistoryError> {
        if self.entries.is_empty() {
            return Err(MetadataHistoryError::NotInitialized);
        }
        if !matches!(
            actor.role,
            MetadataActorRole::Admin | MetadataActorRole::System
        ) {
            return Err(MetadataHistoryError::Unauthorized {
                actor: actor.id.clone(),
                action: "resolve market metadata",
            });
        }
        self.published = true;
        self.resolved = true;
        Ok(())
    }

    /// Append a validated metadata change with optimistic concurrency, role
    /// scoping, post-publish field immutability, and hash-chain linking.
    pub fn record_change(
        &mut self,
        actor: &MetadataActor,
        req: MetadataUpdateRequest,
    ) -> Result<&MetadataChangeEntry, MetadataHistoryError> {
        let actor_id = validate_non_empty("actor.id", &actor.id, 128)?;
        let reason = validate_non_empty("reason", &req.reason, MAX_CHANGE_REASON_LEN)?;
        if req.origin == ChangeOrigin::Creation {
            return Err(MetadataHistoryError::Validation {
                field: "origin",
                message: "use record_initial for creation".to_string(),
            });
        }

        let last = self
            .entries
            .last()
            .ok_or(MetadataHistoryError::NotInitialized)?;
        if last.version != req.expected_version {
            return Err(MetadataHistoryError::VersionConflict {
                expected: req.expected_version,
                actual: last.version,
            });
        }
        if req.timestamp_unix_ms < last.timestamp_unix_ms {
            return Err(MetadataHistoryError::NonMonotonicTimestamp {
                previous_ms: last.timestamp_unix_ms,
                provided_ms: req.timestamp_unix_ms,
            });
        }

        if self.resolved
            && !(actor.role == MetadataActorRole::Admin && req.origin == ChangeOrigin::Correction)
        {
            return Err(MetadataHistoryError::MarketResolved {
                market_id: self.market_id,
            });
        }

        let next = req.next_snapshot.normalized();
        next.validate()?;
        let diffs = compute_metadata_diff(&last.snapshot, &next);
        if diffs.is_empty() {
            return Err(MetadataHistoryError::NoChangesDetected);
        }

        self.check_edit_authorization(&actor_id, actor.role, &diffs)?;
        if self.published {
            if let Some(d) = diffs.iter().find(|d| is_post_publish_immutable(&d.field)) {
                return Err(MetadataHistoryError::ImmutableFieldModified {
                    field: d.field.clone(),
                });
            }
        }

        let prev_hash = last.entry_hash.clone();
        let next_version = last.version + 1;
        let mut entry = MetadataChangeEntry {
            version: next_version,
            market_id: self.market_id,
            timestamp_unix_ms: req.timestamp_unix_ms,
            actor: actor_id,
            actor_role: actor.role,
            origin: req.origin,
            reason,
            supersedes_version: None,
            diffs,
            snapshot: next,
            prev_hash,
            entry_hash: String::new(),
        };
        entry.entry_hash = compute_entry_hash(&entry);
        self.entries.push(entry);
        Ok(self.entries.last().expect("just pushed"))
    }

    /// Append a rollback entry restoring the snapshot from `target_version`.
    /// Prior entries are never deleted or overwritten.
    pub fn rollback_to_version(
        &mut self,
        actor: &MetadataActor,
        target_version: u64,
        expected_version: u64,
        reason: &str,
        timestamp_unix_ms: i64,
    ) -> Result<&MetadataChangeEntry, MetadataHistoryError> {
        let actor_id = validate_non_empty("actor.id", &actor.id, 128)?;
        let reason = validate_non_empty("reason", reason, MAX_CHANGE_REASON_LEN)?;

        if !matches!(
            actor.role,
            MetadataActorRole::Moderator | MetadataActorRole::Admin
        ) {
            return Err(MetadataHistoryError::Unauthorized {
                actor: actor_id,
                action: "rollback market metadata",
            });
        }
        if self.resolved {
            return Err(MetadataHistoryError::MarketResolved {
                market_id: self.market_id,
            });
        }

        let last = self
            .entries
            .last()
            .ok_or(MetadataHistoryError::NotInitialized)?;
        if last.version != expected_version {
            return Err(MetadataHistoryError::VersionConflict {
                expected: expected_version,
                actual: last.version,
            });
        }
        if timestamp_unix_ms < last.timestamp_unix_ms {
            return Err(MetadataHistoryError::NonMonotonicTimestamp {
                previous_ms: last.timestamp_unix_ms,
                provided_ms: timestamp_unix_ms,
            });
        }

        let target = self
            .at_version(target_version)
            .ok_or(MetadataHistoryError::VersionNotFound {
                version: target_version,
            })?
            .snapshot
            .clone();

        let diffs = compute_metadata_diff(&last.snapshot, &target);
        if diffs.is_empty() {
            return Err(MetadataHistoryError::NoChangesDetected);
        }
        if self.published {
            if let Some(d) = diffs.iter().find(|d| is_post_publish_immutable(&d.field)) {
                return Err(MetadataHistoryError::ImmutableFieldModified {
                    field: d.field.clone(),
                });
            }
        }

        let prev_hash = last.entry_hash.clone();
        let next_version = last.version + 1;
        let mut entry = MetadataChangeEntry {
            version: next_version,
            market_id: self.market_id,
            timestamp_unix_ms,
            actor: actor_id,
            actor_role: actor.role,
            origin: ChangeOrigin::Rollback,
            reason,
            supersedes_version: Some(target_version),
            diffs,
            snapshot: target,
            prev_hash,
            entry_hash: String::new(),
        };
        entry.entry_hash = compute_entry_hash(&entry);
        self.entries.push(entry);
        Ok(self.entries.last().expect("just pushed"))
    }

    pub fn current(&self) -> Option<&MetadataChangeEntry> {
        self.entries.last()
    }

    pub fn at_version(&self, version: u64) -> Option<&MetadataChangeEntry> {
        if version == 0 {
            return None;
        }
        self.entries.get((version - 1) as usize)
    }

    pub fn entries(&self) -> &[MetadataChangeEntry] {
        &self.entries
    }

    /// Return a bounded slice of history entries ordered oldest-to-newest.
    pub fn page(&self, offset: usize, limit: usize) -> Result<HistoryPage, MetadataHistoryError> {
        if limit == 0 || limit > MAX_HISTORY_PAGE_SIZE {
            return Err(MetadataHistoryError::Validation {
                field: "limit",
                message: format!("must be between 1 and {MAX_HISTORY_PAGE_SIZE}"),
            });
        }
        let entries = self
            .entries
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();
        Ok(HistoryPage {
            entries,
            total: self.entries.len(),
            offset,
            limit,
        })
    }

    /// Compute the net field-level diff between any two recorded versions.
    pub fn diff_between(
        &self,
        from_version: u64,
        to_version: u64,
    ) -> Result<Vec<MetadataFieldDiff>, MetadataHistoryError> {
        let from = self
            .at_version(from_version)
            .ok_or(MetadataHistoryError::VersionNotFound {
                version: from_version,
            })?;
        let to = self
            .at_version(to_version)
            .ok_or(MetadataHistoryError::VersionNotFound {
                version: to_version,
            })?;
        Ok(compute_metadata_diff(&from.snapshot, &to.snapshot))
    }

    /// Verify sequence continuity, timestamp monotonicity, hash-chain links,
    /// entry digests, and rollback/correction references.
    pub fn verify_integrity(&self) -> Result<HistoryIntegrityReport, MetadataHistoryError> {
        let mut expected_prev = GENESIS_PREV_HASH.to_string();
        let mut last_ts: Option<i64> = None;

        for (idx, entry) in self.entries.iter().enumerate() {
            let expected_version = (idx as u64) + 1;
            if entry.version != expected_version {
                return Err(MetadataHistoryError::IntegrityFailure {
                    version: entry.version,
                    detail: format!("expected version {expected_version}, found {}", entry.version),
                });
            }
            if entry.market_id != self.market_id {
                return Err(MetadataHistoryError::IntegrityFailure {
                    version: entry.version,
                    detail: format!(
                        "market_id mismatch: expected {}, found {}",
                        self.market_id, entry.market_id
                    ),
                });
            }
            if last_ts.is_some_and(|prev| entry.timestamp_unix_ms < prev) {
                return Err(MetadataHistoryError::IntegrityFailure {
                    version: entry.version,
                    detail: "non-monotonic timestamp in stored history".to_string(),
                });
            }
            if entry.prev_hash != expected_prev {
                return Err(MetadataHistoryError::IntegrityFailure {
                    version: entry.version,
                    detail: "broken prev_hash chain link".to_string(),
                });
            }
            if let Some(target) = entry.supersedes_version {
                if target == 0 || target >= entry.version {
                    return Err(MetadataHistoryError::IntegrityFailure {
                        version: entry.version,
                        detail: format!("invalid supersedes_version {target}"),
                    });
                }
            }
            let recomputed = compute_entry_hash(entry);
            if recomputed != entry.entry_hash {
                return Err(MetadataHistoryError::IntegrityFailure {
                    version: entry.version,
                    detail: "entry_hash mismatch (tampering detected)".to_string(),
                });
            }
            expected_prev = entry.entry_hash.clone();
            last_ts = Some(entry.timestamp_unix_ms);
        }

        Ok(HistoryIntegrityReport {
            ok: true,
            checked_entries: self.entries.len(),
            head_version: self.entries.last().map(|e| e.version).unwrap_or(0),
            head_hash: self.entries.last().map(|e| e.entry_hash.clone()),
        })
    }

    fn check_edit_authorization(
        &self,
        actor_id: &str,
        role: MetadataActorRole,
        diffs: &[MetadataFieldDiff],
    ) -> Result<(), MetadataHistoryError> {
        match role {
            MetadataActorRole::Admin
            | MetadataActorRole::Moderator
            | MetadataActorRole::System => Ok(()),
            MetadataActorRole::Creator => {
                if actor_id == self.creator_id {
                    Ok(())
                } else {
                    Err(MetadataHistoryError::Unauthorized {
                        actor: actor_id.to_string(),
                        action: "edit another creator's market metadata",
                    })
                }
            }
            MetadataActorRole::Translator => {
                let only_localized = diffs
                    .iter()
                    .all(|d| d.field.starts_with("localized_descriptions."));
                if only_localized {
                    Ok(())
                } else {
                    Err(MetadataHistoryError::Unauthorized {
                        actor: actor_id.to_string(),
                        action: "modify non-localized metadata fields",
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initial_snapshot() -> MarketMetadataSnapshot {
        let mut localized = BTreeMap::new();
        localized.insert("es".into(), "Descripcion inicial".into());
        MarketMetadataSnapshot {
            canonical_question: "Will ETH exceed $5,000 before June 1?".into(),
            description: "Resolves YES if ETH/USD daily close exceeds $5,000.".into(),
            category: "Crypto".into(),
            tags: vec!["ETH".into(), "Price".into(), "eth".into()],
            localized_descriptions: localized,
            resolution_source: Some("https://example.com/oracle".into()),
            close_time_unix_secs: 1_800_000_000,
        }
    }

    fn initialized_history() -> MarketMetadataHistory {
        let mut history = MarketMetadataHistory::new(77, "creator-1").unwrap();
        let creator = MetadataActor::new("creator-1", MetadataActorRole::Creator);
        history
            .record_initial(&creator, initial_snapshot(), 1_000)
            .unwrap();
        history
    }

    #[test]
    fn preserves_full_versioned_history_diffs_and_chain_integrity() {
        let mut history = initialized_history();
        let creator = MetadataActor::new("creator-1", MetadataActorRole::Creator);
        let moderator = MetadataActor::new("mod-1", MetadataActorRole::Moderator);

        // Creator refines description and tags before publishing.
        let mut v2_snap = history.current().unwrap().snapshot.clone();
        v2_snap.description = "Resolves YES if Coinbase ETH-USD daily close exceeds $5,000.".into();
        v2_snap.tags.push("defi".into());

        let v2 = history
            .record_change(
                &creator,
                MetadataUpdateRequest {
                    expected_version: 1,
                    origin: ChangeOrigin::CreatorEdit,
                    reason: "Clarified reference exchange".into(),
                    timestamp_unix_ms: 2_000,
                    next_snapshot: v2_snap,
                },
            )
            .unwrap();
        assert_eq!(v2.version, 2);
        assert_eq!(v2.diffs.len(), 2);

        history.mark_published(&creator).unwrap();

        // Moderator adds a category cleanup after publishing.
        let mut v3_snap = history.current().unwrap().snapshot.clone();
        v3_snap.category = "finance".into();
        history
            .record_change(
                &moderator,
                MetadataUpdateRequest {
                    expected_version: 2,
                    origin: ChangeOrigin::ModeratorEdit,
                    reason: "Aligned with governed taxonomy".into(),
                    timestamp_unix_ms: 3_000,
                    next_snapshot: v3_snap,
                },
            )
            .unwrap();

        let report = history.verify_integrity().unwrap();
        assert!(report.ok);
        assert_eq!(report.checked_entries, 3);
        assert_eq!(report.head_version, 3);

        // Point-in-time snapshot and cross-version diff work accurately.
        assert_eq!(history.at_version(1).unwrap().snapshot.category, "crypto");
        let net_diff = history.diff_between(1, 3).unwrap();
        assert!(net_diff.iter().any(|d| d.field == "category"));
        assert!(net_diff.iter().any(|d| d.field == "description"));

        // Bounded pagination works deterministically.
        let page = history.page(1, 2).unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.entries.len(), 2);
        assert_eq!(page.entries[0].version, 2);
        assert_eq!(page.entries[1].version, 3);

        // JSON round-trip preserves intact hash chain.
        let json = serde_json::to_string(&history).unwrap();
        let back: MarketMetadataHistory = serde_json::from_str(&json).unwrap();
        assert_eq!(back, history);
        assert!(back.verify_integrity().unwrap().ok);
    }

    #[test]
    fn enforces_post_publish_immutability_role_scope_and_resolution_freeze() {
        let mut history = initialized_history();
        let creator = MetadataActor::new("creator-1", MetadataActorRole::Creator);
        let translator = MetadataActor::new("trans-1", MetadataActorRole::Translator);
        let admin = MetadataActor::new("admin-1", MetadataActorRole::Admin);

        // Translator can update localized descriptions but cannot touch category.
        let mut bad_trans = history.current().unwrap().snapshot.clone();
        bad_trans.category = "sports".into();
        assert!(matches!(
            history.record_change(
                &translator,
                MetadataUpdateRequest {
                    expected_version: 1,
                    origin: ChangeOrigin::LocalizationUpdate,
                    reason: "Attempt category edit".into(),
                    timestamp_unix_ms: 1_500,
                    next_snapshot: bad_trans,
                },
            ),
            Err(MetadataHistoryError::Unauthorized { .. })
        ));

        let mut ok_trans = history.current().unwrap().snapshot.clone();
        ok_trans.localized_descriptions.insert("fr".into(), "Description en francais.".into());
        history
            .record_change(
                &translator,
                MetadataUpdateRequest {
                    expected_version: 1,
                    origin: ChangeOrigin::LocalizationUpdate,
                    reason: "Added French translation".into(),
                    timestamp_unix_ms: 1_600,
                    next_snapshot: ok_trans,
                },
            )
            .unwrap();

        history.mark_published(&creator).unwrap();

        // Post-publish canonical_question and close_time_unix_secs are immutable.
        let mut mutate_q = history.current().unwrap().snapshot.clone();
        mutate_q.canonical_question = "Altered question?".into();
        assert_eq!(
            history
                .record_change(
                    &admin,
                    MetadataUpdateRequest {
                        expected_version: 2,
                        origin: ChangeOrigin::ModeratorEdit,
                        reason: "Should fail".into(),
                        timestamp_unix_ms: 2_000,
                        next_snapshot: mutate_q,
                    },
                )
                .unwrap_err(),
            MetadataHistoryError::ImmutableFieldModified {
                field: "canonical_question".into(),
            }
        );

        // Once resolved, normal edits fail, while admin append-only Correction succeeds.
        history.mark_resolved(&admin).unwrap();
        let mut post_res = history.current().unwrap().snapshot.clone();
        post_res.description = "Post-resolution clarification note.".into();
        assert!(matches!(
            history.record_change(
                &creator,
                MetadataUpdateRequest {
                    expected_version: 2,
                    origin: ChangeOrigin::CreatorEdit,
                    reason: "Late edit".into(),
                    timestamp_unix_ms: 3_000,
                    next_snapshot: post_res.clone(),
                },
            ),
            Err(MetadataHistoryError::MarketResolved { market_id: 77 })
        ));

        let correction = history
            .record_change(
                &admin,
                MetadataUpdateRequest {
                    expected_version: 2,
                    origin: ChangeOrigin::Correction,
                    reason: "Added official post-resolution source note".into(),
                    timestamp_unix_ms: 3_100,
                    next_snapshot: post_res,
                },
            )
            .unwrap();
        assert_eq!(correction.version, 3);
        assert_eq!(correction.origin, ChangeOrigin::Correction);
    }

    #[test]
    fn rejects_noop_version_conflict_and_supports_append_only_rollback_and_tamper_detection() {
        let mut history = initialized_history();
        let creator = MetadataActor::new("creator-1", MetadataActorRole::Creator);
        let moderator = MetadataActor::new("mod-1", MetadataActorRole::Moderator);

        // No-op edit rejected.
        let same = history.current().unwrap().snapshot.clone();
        assert_eq!(
            history
                .record_change(
                    &creator,
                    MetadataUpdateRequest {
                        expected_version: 1,
                        origin: ChangeOrigin::CreatorEdit,
                        reason: "No changes".into(),
                        timestamp_unix_ms: 1_500,
                        next_snapshot: same,
                    },
                )
                .unwrap_err(),
            MetadataHistoryError::NoChangesDetected
        );

        // Valid v2 edit.
        let mut v2 = history.current().unwrap().snapshot.clone();
        v2.description = "Accidental bad edit".into();
        history
            .record_change(
                &creator,
                MetadataUpdateRequest {
                    expected_version: 1,
                    origin: ChangeOrigin::CreatorEdit,
                    reason: "Draft update".into(),
                    timestamp_unix_ms: 2_000,
                    next_snapshot: v2.clone(),
                },
            )
            .unwrap();

        // Stale expected_version fails cleanly.
        v2.description = "Concurrent edit".into();
        assert_eq!(
            history
                .record_change(
                    &creator,
                    MetadataUpdateRequest {
                        expected_version: 1,
                        origin: ChangeOrigin::CreatorEdit,
                        reason: "Stale write".into(),
                        timestamp_unix_ms: 2_500,
                        next_snapshot: v2,
                    },
                )
                .unwrap_err(),
            MetadataHistoryError::VersionConflict {
                expected: 1,
                actual: 2,
            }
        );

        // Append-only rollback restores v1 snapshot as v3 while keeping v1 and v2 intact.
        let rolled = history
            .rollback_to_version(&moderator, 1, 2, "Reverted accidental edit", 3_000)
            .unwrap();
        assert_eq!(rolled.version, 3);
        assert_eq!(rolled.origin, ChangeOrigin::Rollback);
        assert_eq!(rolled.supersedes_version, Some(1));
        assert_eq!(
            rolled.snapshot.description,
            history.at_version(1).unwrap().snapshot.description
        );
        assert_eq!(history.entries().len(), 3);
        assert!(history.verify_integrity().unwrap().ok);

        // Tampering with a stored entry is detected by verify_integrity.
        let mut tampered = history.clone();
        tampered.entries[1].reason = "Rewritten history".into();
        assert!(matches!(
            tampered.verify_integrity(),
            Err(MetadataHistoryError::IntegrityFailure { version: 2, .. })
        ));
    }
}
