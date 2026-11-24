//! Moderator review queue for reported market content (#245).
//!
//! Aggregates user reports per `(market_id, content_kind)`, enforces role-based
//! authorization, lease-backed claiming, optimistic concurrency, step-up proof
//! verification for privileged visibility changes, and durable audit logging
//! via [`crate::moderation_audit`]. If an audit write fails, the queue item
//! transitions to an explicit `AuditSyncFailed` recovery state without
//! mutating public content visibility until the audit sync is retried.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::moderation_audit::{
    ModerationAuditLog, ModerationRecord, PrivilegedAction, StepUpError, StepUpProof,
    StepUpVerifier,
};

pub const MAX_REPORT_DETAILS_LEN: usize = 1000;
pub const MAX_REASON_LEN: usize = 500;
pub const DEFAULT_LEASE_DURATION_MS: i64 = 15 * 60 * 1000;
pub const MAX_QUEUE_PAGE_SIZE: usize = 100;

/// Specific content surface on a market that can be reported and moderated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportedContentKind {
    MarketQuestion,
    MarketDescription,
    LocalizedDescription,
    ResolutionSource,
    OutcomeLabel,
}

impl ReportedContentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReportedContentKind::MarketQuestion => "market_question",
            ReportedContentKind::MarketDescription => "market_description",
            ReportedContentKind::LocalizedDescription => "localized_description",
            ReportedContentKind::ResolutionSource => "resolution_source",
            ReportedContentKind::OutcomeLabel => "outcome_label",
        }
    }
}

/// Build the canonical audit and step-up target string for reported content.
pub fn audit_target(market_id: u64, content_kind: ReportedContentKind) -> String {
    format!("market:{market_id}:{}", content_kind.as_str())
}

/// Structured reason supplied by a user reporting market content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportReason {
    Spam,
    Misleading,
    ProhibitedContent,
    Harassment,
    DuplicateMarket,
    BrokenResolutionSource,
    Other,
}

impl ReportReason {
    pub fn base_priority(&self) -> QueuePriority {
        match self {
            ReportReason::ProhibitedContent => QueuePriority::Urgent,
            ReportReason::Harassment | ReportReason::Misleading => QueuePriority::High,
            ReportReason::BrokenResolutionSource | ReportReason::Spam => QueuePriority::Normal,
            ReportReason::DuplicateMarket | ReportReason::Other => QueuePriority::Low,
        }
    }
}

/// Triage priority; ordered from lowest to highest urgency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueuePriority {
    Low,
    Normal,
    High,
    Urgent,
}

/// Lifecycle state of a moderation queue item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueStatus {
    Pending,
    InReview,
    Escalated,
    Resolved,
    /// Decision was validated, but durable audit append failed; retryable.
    AuditSyncFailed,
}

/// Authorization role of a caller interacting with the review queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorRole {
    Reporter,
    Moderator,
    SeniorModerator,
    Admin,
}

impl ActorRole {
    pub fn can_moderate(&self) -> bool {
        matches!(self, ActorRole::Moderator | ActorRole::SeniorModerator | ActorRole::Admin)
    }

    pub fn can_handle_escalation(&self) -> bool {
        matches!(self, ActorRole::SeniorModerator | ActorRole::Admin)
    }
}

/// Authenticated actor performing a moderation queue action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueActor {
    pub id: String,
    pub role: ActorRole,
}

impl QueueActor {
    pub fn new(id: impl Into<String>, role: ActorRole) -> Self {
        Self {
            id: id.into(),
            role,
        }
    }
}

/// Moderation disposition applied to a queue item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationOutcome {
    Dismissed,
    Hidden,
    Edited,
    Restored,
}

impl ModerationOutcome {
    pub fn audit_action(&self) -> &'static str {
        match self {
            ModerationOutcome::Dismissed => "dismiss_report",
            ModerationOutcome::Hidden => PrivilegedAction::HideContent.as_str(),
            ModerationOutcome::Edited => "edit_reported_content",
            ModerationOutcome::Restored => PrivilegedAction::UnhideContent.as_str(),
        }
    }
}

/// Authoritative public visibility state for a `(market_id, content_kind)` pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentVisibility {
    Visible,
    Hidden {
        item_id: u64,
        decided_by: String,
        reason: String,
        decided_at_ms: i64,
    },
}

/// A single user-submitted content report attached to a queue item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentReport {
    pub report_id: u64,
    pub market_id: u64,
    pub content_kind: ReportedContentKind,
    pub reporter: String,
    pub reason: ReportReason,
    pub details: String,
    pub reported_at_ms: i64,
}

/// Finalized or pending-audit moderation decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModerationDecision {
    pub outcome: ModerationOutcome,
    pub decided_by: String,
    pub reason: String,
    pub decided_at_ms: i64,
    pub proof_timestamp_unix_ms: Option<i64>,
    pub audit_sequence: Option<u64>,
}

/// Explicit failure/recovery payload when audit logging fails on resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingAuditSync {
    pub decision: ModerationDecision,
    pub failure_detail: String,
    pub attempts: u32,
    pub last_attempt_ms: i64,
}

/// One aggregated moderation case in the review queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewQueueItem {
    pub item_id: u64,
    pub market_id: u64,
    pub content_kind: ReportedContentKind,
    pub status: QueueStatus,
    pub priority: QueuePriority,
    pub reports: Vec<ContentReport>,
    pub assigned_to: Option<String>,
    pub lease_expires_at_ms: Option<i64>,
    pub escalation_reason: Option<String>,
    pub decision: Option<ModerationDecision>,
    pub pending_audit: Option<PendingAuditSync>,
    pub revision: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl ReviewQueueItem {
    /// Returns true if the item is currently held under an unexpired review lease.
    pub fn has_active_lease(&self, now_ms: i64) -> bool {
        matches!(
            (self.status, &self.assigned_to, self.lease_expires_at_ms),
            (QueueStatus::InReview, Some(_), Some(expires)) if now_ms < expires
        )
    }
}

/// Input payload for reporting market content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitReportRequest {
    pub market_id: u64,
    pub content_kind: ReportedContentKind,
    pub reporter: String,
    pub reason: ReportReason,
    pub details: String,
    pub reported_at_ms: i64,
}

/// Input payload for resolving a claimed or escalated queue item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionRequest {
    pub item_id: u64,
    pub expected_revision: u64,
    pub outcome: ModerationOutcome,
    pub reason: String,
    pub decided_at_ms: i64,
}

/// Filter and pagination parameters for listing queue items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueFilter {
    pub status: Option<QueueStatus>,
    pub min_priority: Option<QueuePriority>,
    pub market_id: Option<u64>,
    pub assigned_to: Option<String>,
    pub offset: usize,
    pub limit: usize,
}

impl Default for QueueFilter {
    fn default() -> Self {
        Self {
            status: None,
            min_priority: None,
            market_id: None,
            assigned_to: None,
            offset: 0,
            limit: 25,
        }
    }
}

/// Bounded page of queue items ordered by priority and age.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuePage {
    pub items: Vec<ReviewQueueItem>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

/// Errors produced at the moderation queue boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueError {
    Validation {
        field: &'static str,
        message: String,
    },
    DuplicateReport {
        item_id: u64,
        reporter: String,
    },
    NotFound {
        item_id: u64,
    },
    Unauthorized {
        actor: String,
        required: &'static str,
    },
    EscalationPrivilegeRequired {
        item_id: u64,
        actor: String,
    },
    RevisionConflict {
        item_id: u64,
        expected: u64,
        actual: u64,
    },
    InvalidState {
        item_id: u64,
        current: QueueStatus,
        action: &'static str,
    },
    AlreadyClaimed {
        item_id: u64,
        assigned_to: String,
        lease_expires_at_ms: i64,
    },
    StepUpFailed(StepUpError),
    AuditSyncPending {
        item_id: u64,
        detail: String,
    },
}

impl fmt::Display for QueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueueError::Validation { field, message } => {
                write!(f, "invalid field '{field}': {message}")
            }
            QueueError::DuplicateReport { item_id, reporter } => {
                write!(f, "reporter '{reporter}' already reported open queue item #{item_id}")
            }
            QueueError::NotFound { item_id } => write!(f, "queue item #{item_id} not found"),
            QueueError::Unauthorized { actor, required } => {
                write!(f, "actor '{actor}' lacks required role '{required}'")
            }
            QueueError::EscalationPrivilegeRequired { item_id, actor } => {
                write!(f, "escalated item #{item_id} requires senior moderator (actor: '{actor}')")
            }
            QueueError::RevisionConflict {
                item_id,
                expected,
                actual,
            } => write!(
                f,
                "revision conflict on item #{item_id}: expected {expected}, actual {actual}"
            ),
            QueueError::InvalidState {
                item_id,
                current,
                action,
            } => write!(f, "cannot {action} item #{item_id} while in status {current:?}"),
            QueueError::AlreadyClaimed {
                item_id,
                assigned_to,
                lease_expires_at_ms,
            } => write!(
                f,
                "item #{item_id} is claimed by '{assigned_to}' until {lease_expires_at_ms}"
            ),
            QueueError::StepUpFailed(err) => write!(f, "step-up verification failed: {err}"),
            QueueError::AuditSyncPending { item_id, detail } => {
                write!(f, "decision for item #{item_id} requires audit sync retry: {detail}")
            }
        }
    }
}

impl std::error::Error for QueueError {}

impl From<StepUpError> for QueueError {
    fn from(err: StepUpError) -> Self {
        QueueError::StepUpFailed(err)
    }
}

/// Sink abstraction connecting queue resolutions to the tamper-evident audit log.
pub trait ModerationAuditSink {
    #[allow(clippy::too_many_arguments)]
    fn record_action(
        &mut self,
        actor: &str,
        action: &str,
        target: &str,
        reason: &str,
        timestamp_unix_ms: i64,
        proof_timestamp_unix_ms: Option<i64>,
        corrects_sequence: Option<u64>,
    ) -> Result<ModerationRecord, String>;
}

impl ModerationAuditSink for ModerationAuditLog {
    fn record_action(
        &mut self,
        actor: &str,
        action: &str,
        target: &str,
        reason: &str,
        timestamp_unix_ms: i64,
        proof_timestamp_unix_ms: Option<i64>,
        corrects_sequence: Option<u64>,
    ) -> Result<ModerationRecord, String> {
        let res = match corrects_sequence {
            Some(seq) => self.correct(
                seq,
                actor,
                action,
                target,
                reason,
                timestamp_unix_ms,
                proof_timestamp_unix_ms,
            ),
            None => self.append(
                actor,
                action,
                target,
                reason,
                timestamp_unix_ms,
                proof_timestamp_unix_ms,
            ),
        };
        res.map_err(|e| e.to_string())
    }
}

fn validate_non_empty(
    field: &'static str,
    raw: &str,
    max_len: usize,
) -> Result<String, QueueError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(QueueError::Validation {
            field,
            message: "must not be empty".to_string(),
        });
    }
    if trimmed.chars().count() > max_len {
        return Err(QueueError::Validation {
            field,
            message: format!("must be at most {max_len} characters"),
        });
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(QueueError::Validation {
            field,
            message: "must not contain control characters".to_string(),
        });
    }
    Ok(trimmed.to_string())
}

fn compute_priority(reports: &[ContentReport]) -> QueuePriority {
    let highest_reason = reports
        .iter()
        .map(|r| r.reason.base_priority())
        .max()
        .unwrap_or(QueuePriority::Low);

    let count_priority = match reports.len() {
        0..=1 => QueuePriority::Low,
        2 => QueuePriority::Normal,
        3..=4 => QueuePriority::High,
        _ => QueuePriority::Urgent,
    };

    highest_reason.max(count_priority)
}

fn require_moderator(actor: &QueueActor) -> Result<(), QueueError> {
    if actor.id.trim().is_empty() {
        return Err(QueueError::Validation {
            field: "actor.id",
            message: "must not be empty".to_string(),
        });
    }
    if !actor.role.can_moderate() {
        return Err(QueueError::Unauthorized {
            actor: actor.id.clone(),
            required: "moderator",
        });
    }
    Ok(())
}

/// Authoritative in-memory moderation review queue for reported market content.
#[derive(Debug, Default)]
pub struct ModerationQueue {
    next_item_id: u64,
    next_report_id: u64,
    items: HashMap<u64, ReviewQueueItem>,
    visibility: HashMap<(u64, ReportedContentKind), ContentVisibility>,
}

impl ModerationQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Submit a user report. Reports for the same `(market_id, content_kind)`
    /// with an active (non-resolved) queue item are aggregated onto that item.
    pub fn submit_report(
        &mut self,
        req: SubmitReportRequest,
    ) -> Result<ReviewQueueItem, QueueError> {
        if req.market_id == 0 {
            return Err(QueueError::Validation {
                field: "market_id",
                message: "must be positive".to_string(),
            });
        }
        let reporter = validate_non_empty("reporter", &req.reporter, 128)?;
        let details = validate_non_empty("details", &req.details, MAX_REPORT_DETAILS_LEN)?;

        let active_item_id = self
            .items
            .values()
            .find(|item| {
                item.market_id == req.market_id
                    && item.content_kind == req.content_kind
                    && item.status != QueueStatus::Resolved
            })
            .map(|item| item.item_id);

        if let Some(item_id) = active_item_id {
            let item = self.items.get_mut(&item_id).expect("item exists");
            if item.reports.iter().any(|r| r.reporter == reporter) {
                return Err(QueueError::DuplicateReport { item_id, reporter });
            }
            self.next_report_id += 1;
            item.reports.push(ContentReport {
                report_id: self.next_report_id,
                market_id: req.market_id,
                content_kind: req.content_kind,
                reporter,
                reason: req.reason,
                details,
                reported_at_ms: req.reported_at_ms,
            });
            item.priority = compute_priority(&item.reports);
            item.updated_at_ms = item.updated_at_ms.max(req.reported_at_ms);
            item.revision += 1;
            return Ok(item.clone());
        }

        self.next_item_id += 1;
        self.next_report_id += 1;
        let report = ContentReport {
            report_id: self.next_report_id,
            market_id: req.market_id,
            content_kind: req.content_kind,
            reporter,
            reason: req.reason,
            details,
            reported_at_ms: req.reported_at_ms,
        };
        let priority = compute_priority(std::slice::from_ref(&report));
        let item = ReviewQueueItem {
            item_id: self.next_item_id,
            market_id: req.market_id,
            content_kind: req.content_kind,
            status: QueueStatus::Pending,
            priority,
            reports: vec![report],
            assigned_to: None,
            lease_expires_at_ms: None,
            escalation_reason: None,
            decision: None,
            pending_audit: None,
            revision: 1,
            created_at_ms: req.reported_at_ms,
            updated_at_ms: req.reported_at_ms,
        };
        self.items.insert(item.item_id, item.clone());
        Ok(item)
    }

    /// List queue items ordered deterministically by priority (desc), age (asc),
    /// and item id (asc). Restricted to moderator roles.
    pub fn list_queue(
        &self,
        actor: &QueueActor,
        filter: &QueueFilter,
    ) -> Result<QueuePage, QueueError> {
        require_moderator(actor)?;
        if filter.limit == 0 || filter.limit > MAX_QUEUE_PAGE_SIZE {
            return Err(QueueError::Validation {
                field: "limit",
                message: format!("must be between 1 and {MAX_QUEUE_PAGE_SIZE}"),
            });
        }

        let mut matching: Vec<&ReviewQueueItem> = self
            .items
            .values()
            .filter(|item| {
                if filter.status.is_some_and(|s| item.status != s) {
                    return false;
                }
                if filter.min_priority.is_some_and(|p| item.priority < p) {
                    return false;
                }
                if filter.market_id.is_some_and(|m| item.market_id != m) {
                    return false;
                }
                if filter
                    .assigned_to
                    .as_deref()
                    .is_some_and(|a| item.assigned_to.as_deref() != Some(a))
                {
                    return false;
                }
                true
            })
            .collect();

        matching.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.created_at_ms.cmp(&b.created_at_ms))
                .then_with(|| a.item_id.cmp(&b.item_id))
        });

        let total = matching.len();
        let items = matching
            .into_iter()
            .skip(filter.offset)
            .take(filter.limit)
            .cloned()
            .collect();

        Ok(QueuePage {
            items,
            total,
            offset: filter.offset,
            limit: filter.limit,
        })
    }

    /// Fetch a single queue item. Restricted to moderator roles.
    pub fn get_item(
        &self,
        actor: &QueueActor,
        item_id: u64,
    ) -> Result<ReviewQueueItem, QueueError> {
        require_moderator(actor)?;
        self.items
            .get(&item_id)
            .cloned()
            .ok_or(QueueError::NotFound { item_id })
    }

    /// Return the authoritative visibility of a reported market content surface.
    pub fn content_visibility(
        &self,
        market_id: u64,
        content_kind: ReportedContentKind,
    ) -> ContentVisibility {
        self.visibility
            .get(&(market_id, content_kind))
            .cloned()
            .unwrap_or(ContentVisibility::Visible)
    }

    /// Claim a `Pending` item or reclaim an `InReview` item whose lease has expired.
    pub fn claim_item(
        &mut self,
        actor: &QueueActor,
        item_id: u64,
        expected_revision: u64,
        now_ms: i64,
        lease_duration_ms: Option<i64>,
    ) -> Result<ReviewQueueItem, QueueError> {
        require_moderator(actor)?;
        let lease_ms = lease_duration_ms.unwrap_or(DEFAULT_LEASE_DURATION_MS);
        if lease_ms <= 0 {
            return Err(QueueError::Validation {
                field: "lease_duration_ms",
                message: "must be positive".to_string(),
            });
        }

        let item = self
            .items
            .get_mut(&item_id)
            .ok_or(QueueError::NotFound { item_id })?;

        if item.revision != expected_revision {
            return Err(QueueError::RevisionConflict {
                item_id,
                expected: expected_revision,
                actual: item.revision,
            });
        }

        match item.status {
            QueueStatus::Pending => {}
            QueueStatus::InReview => {
                if let (Some(holder), Some(expires)) = (&item.assigned_to, item.lease_expires_at_ms)
                {
                    if now_ms < expires && holder != &actor.id {
                        return Err(QueueError::AlreadyClaimed {
                            item_id,
                            assigned_to: holder.clone(),
                            lease_expires_at_ms: expires,
                        });
                    }
                }
            }
            QueueStatus::Escalated => {
                if !actor.role.can_handle_escalation() {
                    return Err(QueueError::EscalationPrivilegeRequired {
                        item_id,
                        actor: actor.id.clone(),
                    });
                }
            }
            other => {
                return Err(QueueError::InvalidState {
                    item_id,
                    current: other,
                    action: "claim",
                });
            }
        }

        if item.status != QueueStatus::Escalated {
            item.status = QueueStatus::InReview;
        }
        item.assigned_to = Some(actor.id.clone());
        item.lease_expires_at_ms = Some(now_ms.saturating_add(lease_ms));
        item.updated_at_ms = item.updated_at_ms.max(now_ms);
        item.revision += 1;
        Ok(item.clone())
    }

    /// Voluntarily release a claim back to `Pending`. Admins may release any claim.
    pub fn release_claim(
        &mut self,
        actor: &QueueActor,
        item_id: u64,
        expected_revision: u64,
        now_ms: i64,
    ) -> Result<ReviewQueueItem, QueueError> {
        require_moderator(actor)?;
        let item = self
            .items
            .get_mut(&item_id)
            .ok_or(QueueError::NotFound { item_id })?;

        if item.revision != expected_revision {
            return Err(QueueError::RevisionConflict {
                item_id,
                expected: expected_revision,
                actual: item.revision,
            });
        }
        if item.status != QueueStatus::InReview {
            return Err(QueueError::InvalidState {
                item_id,
                current: item.status,
                action: "release",
            });
        }
        if item.assigned_to.as_deref() != Some(actor.id.as_str())
            && actor.role != ActorRole::Admin
        {
            return Err(QueueError::Unauthorized {
                actor: actor.id.clone(),
                required: "assignee_or_admin",
            });
        }

        item.status = QueueStatus::Pending;
        item.assigned_to = None;
        item.lease_expires_at_ms = None;
        item.updated_at_ms = item.updated_at_ms.max(now_ms);
        item.revision += 1;
        Ok(item.clone())
    }

    /// Escalate an item to `SeniorModerator` / `Admin` review.
    pub fn escalate_item(
        &mut self,
        actor: &QueueActor,
        item_id: u64,
        expected_revision: u64,
        reason: &str,
        now_ms: i64,
    ) -> Result<ReviewQueueItem, QueueError> {
        require_moderator(actor)?;
        let reason = validate_non_empty("reason", reason, MAX_REASON_LEN)?;

        let item = self
            .items
            .get_mut(&item_id)
            .ok_or(QueueError::NotFound { item_id })?;

        if item.revision != expected_revision {
            return Err(QueueError::RevisionConflict {
                item_id,
                expected: expected_revision,
                actual: item.revision,
            });
        }
        if !matches!(item.status, QueueStatus::Pending | QueueStatus::InReview) {
            return Err(QueueError::InvalidState {
                item_id,
                current: item.status,
                action: "escalate",
            });
        }

        item.status = QueueStatus::Escalated;
        item.priority = item.priority.max(QueuePriority::High);
        item.assigned_to = None;
        item.lease_expires_at_ms = None;
        item.escalation_reason = Some(reason);
        item.updated_at_ms = item.updated_at_ms.max(now_ms);
        item.revision += 1;
        Ok(item.clone())
    }

    /// Apply a moderation decision. Privileged outcomes (`Hidden`, `Restored`)
    /// require a valid, fresh, target-bound [`StepUpProof`]. If the audit sink
    /// fails, the item transitions to `AuditSyncFailed` and content visibility
    /// remains unchanged until [`Self::retry_audit_sync`] succeeds.
    pub fn resolve_item<F, S>(
        &mut self,
        actor: &QueueActor,
        req: DecisionRequest,
        proof: Option<&StepUpProof>,
        verifier: &mut StepUpVerifier<F>,
        audit_sink: &mut S,
    ) -> Result<ReviewQueueItem, QueueError>
    where
        F: Fn(&StepUpProof) -> bool,
        S: ModerationAuditSink,
    {
        require_moderator(actor)?;
        let reason = validate_non_empty("reason", &req.reason, MAX_REASON_LEN)?;

        let item_id = req.item_id;
        let item = self
            .items
            .get(&item_id)
            .ok_or(QueueError::NotFound { item_id })?;

        if item.revision != req.expected_revision {
            return Err(QueueError::RevisionConflict {
                item_id: req.item_id,
                expected: req.expected_revision,
                actual: item.revision,
            });
        }

        match item.status {
            QueueStatus::InReview => {
                if item.assigned_to.as_deref() != Some(actor.id.as_str()) {
                    if let (Some(holder), Some(expires)) =
                        (&item.assigned_to, item.lease_expires_at_ms)
                    {
                        if req.decided_at_ms < expires {
                            return Err(QueueError::AlreadyClaimed {
                                item_id: req.item_id,
                                assigned_to: holder.clone(),
                                lease_expires_at_ms: expires,
                            });
                        }
                    }
                    return Err(QueueError::InvalidState {
                        item_id: req.item_id,
                        current: item.status,
                        action: "resolve_unclaimed",
                    });
                }
            }
            QueueStatus::Escalated => {
                if !actor.role.can_handle_escalation() {
                    return Err(QueueError::EscalationPrivilegeRequired {
                        item_id: req.item_id,
                        actor: actor.id.clone(),
                    });
                }
            }
            other => {
                return Err(QueueError::InvalidState {
                    item_id: req.item_id,
                    current: other,
                    action: "resolve",
                });
            }
        }

        let target = audit_target(item.market_id, item.content_kind);
        let action = req.outcome.audit_action();
        verifier.verify(&actor.id, action, &target, proof, req.decided_at_ms)?;

        let proof_ts = if PrivilegedAction::is_privileged(action) {
            proof.map(|p| p.proof_timestamp_unix_ms)
        } else {
            None
        };

        let mut decision = ModerationDecision {
            outcome: req.outcome,
            decided_by: actor.id.clone(),
            reason: reason.clone(),
            decided_at_ms: req.decided_at_ms,
            proof_timestamp_unix_ms: proof_ts,
            audit_sequence: None,
        };

        let market_id = item.market_id;
        let content_kind = item.content_kind;

        match audit_sink.record_action(
            &actor.id,
            action,
            &target,
            &reason,
            req.decided_at_ms,
            proof_ts,
            None,
        ) {
            Ok(record) => {
                decision.audit_sequence = Some(record.sequence);
                let item = self.items.get_mut(&req.item_id).expect("item exists");
                item.status = QueueStatus::Resolved;
                item.assigned_to = None;
                item.lease_expires_at_ms = None;
                item.pending_audit = None;
                item.decision = Some(decision.clone());
                item.updated_at_ms = item.updated_at_ms.max(req.decided_at_ms);
                item.revision += 1;
                let updated = item.clone();
                self.apply_visibility(market_id, content_kind, req.item_id, &decision);
                Ok(updated)
            }
            Err(detail) => {
                let item = self.items.get_mut(&req.item_id).expect("item exists");
                item.status = QueueStatus::AuditSyncFailed;
                item.pending_audit = Some(PendingAuditSync {
                    decision,
                    failure_detail: detail.clone(),
                    attempts: 1,
                    last_attempt_ms: req.decided_at_ms,
                });
                item.updated_at_ms = item.updated_at_ms.max(req.decided_at_ms);
                item.revision += 1;
                Err(QueueError::AuditSyncPending {
                    item_id: req.item_id,
                    detail,
                })
            }
        }
    }

    /// Retry durable audit logging for an item stuck in `AuditSyncFailed`.
    /// Once the audit sink accepts the record, visibility is committed and the
    /// item transitions to `Resolved`.
    pub fn retry_audit_sync<S>(
        &mut self,
        actor: &QueueActor,
        item_id: u64,
        expected_revision: u64,
        now_ms: i64,
        audit_sink: &mut S,
    ) -> Result<ReviewQueueItem, QueueError>
    where
        S: ModerationAuditSink,
    {
        require_moderator(actor)?;

        let item = self
            .items
            .get(&item_id)
            .ok_or(QueueError::NotFound { item_id })?;

        if item.revision != expected_revision {
            return Err(QueueError::RevisionConflict {
                item_id,
                expected: expected_revision,
                actual: item.revision,
            });
        }
        if item.status != QueueStatus::AuditSyncFailed {
            return Err(QueueError::InvalidState {
                item_id,
                current: item.status,
                action: "retry_audit_sync",
            });
        }
        let pending = item
            .pending_audit
            .clone()
            .ok_or(QueueError::InvalidState {
                item_id,
                current: item.status,
                action: "retry_audit_sync",
            })?;

        let market_id = item.market_id;
        let content_kind = item.content_kind;
        let target = audit_target(market_id, content_kind);
        let action = pending.decision.outcome.audit_action();

        match audit_sink.record_action(
            &pending.decision.decided_by,
            action,
            &target,
            &pending.decision.reason,
            pending.decision.decided_at_ms,
            pending.decision.proof_timestamp_unix_ms,
            None,
        ) {
            Ok(record) => {
                let mut decision = pending.decision;
                decision.audit_sequence = Some(record.sequence);
                let item = self.items.get_mut(&item_id).expect("item exists");
                item.status = QueueStatus::Resolved;
                item.assigned_to = None;
                item.lease_expires_at_ms = None;
                item.pending_audit = None;
                item.decision = Some(decision.clone());
                item.updated_at_ms = item.updated_at_ms.max(now_ms);
                item.revision += 1;
                let updated = item.clone();
                self.apply_visibility(market_id, content_kind, item_id, &decision);
                Ok(updated)
            }
            Err(detail) => {
                let item = self.items.get_mut(&item_id).expect("item exists");
                if let Some(p) = item.pending_audit.as_mut() {
                    p.failure_detail = detail.clone();
                    p.attempts += 1;
                    p.last_attempt_ms = now_ms;
                }
                item.updated_at_ms = item.updated_at_ms.max(now_ms);
                item.revision += 1;
                Err(QueueError::AuditSyncPending { item_id, detail })
            }
        }
    }

    /// Reopen a resolved item (e.g. on appeal). Restricted to `SeniorModerator`
    /// or `Admin` and appends a correction record referencing the prior audit
    /// sequence when present.
    pub fn reopen_item<S>(
        &mut self,
        actor: &QueueActor,
        item_id: u64,
        expected_revision: u64,
        reason: &str,
        now_ms: i64,
        audit_sink: &mut S,
    ) -> Result<ReviewQueueItem, QueueError>
    where
        S: ModerationAuditSink,
    {
        require_moderator(actor)?;
        if !actor.role.can_handle_escalation() {
            return Err(QueueError::EscalationPrivilegeRequired {
                item_id,
                actor: actor.id.clone(),
            });
        }
        let reason = validate_non_empty("reason", reason, MAX_REASON_LEN)?;

        let item = self
            .items
            .get(&item_id)
            .ok_or(QueueError::NotFound { item_id })?;

        if item.revision != expected_revision {
            return Err(QueueError::RevisionConflict {
                item_id,
                expected: expected_revision,
                actual: item.revision,
            });
        }
        if item.status != QueueStatus::Resolved {
            return Err(QueueError::InvalidState {
                item_id,
                current: item.status,
                action: "reopen",
            });
        }

        let target = audit_target(item.market_id, item.content_kind);
        let prior_seq = item.decision.as_ref().and_then(|d| d.audit_sequence);
        audit_sink
            .record_action(
                &actor.id,
                "reopen_review_item",
                &target,
                &reason,
                now_ms,
                None,
                prior_seq,
            )
            .map_err(|detail| QueueError::AuditSyncPending { item_id, detail })?;

        let item = self.items.get_mut(&item_id).expect("item exists");
        item.status = QueueStatus::Escalated;
        item.escalation_reason = Some(reason);
        item.assigned_to = None;
        item.lease_expires_at_ms = None;
        item.updated_at_ms = item.updated_at_ms.max(now_ms);
        item.revision += 1;
        Ok(item.clone())
    }

    fn apply_visibility(
        &mut self,
        market_id: u64,
        content_kind: ReportedContentKind,
        item_id: u64,
        decision: &ModerationDecision,
    ) {
        match decision.outcome {
            ModerationOutcome::Hidden => {
                self.visibility.insert(
                    (market_id, content_kind),
                    ContentVisibility::Hidden {
                        item_id,
                        decided_by: decision.decided_by.clone(),
                        reason: decision.reason.clone(),
                        decided_at_ms: decision.decided_at_ms,
                    },
                );
            }
            ModerationOutcome::Restored => {
                self.visibility.insert((market_id, content_kind), ContentVisibility::Visible);
            }
            ModerationOutcome::Dismissed | ModerationOutcome::Edited => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_audit_log() -> (ModerationAuditLog, std::path::PathBuf) {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let file_name = format!("predifi-modqueue-audit-{pid}-{n}.jsonl");
        let path = std::env::temp_dir().join(file_name);
        let _ = fs::remove_file(&path);
        let log = ModerationAuditLog::open(path.to_string_lossy()).unwrap();
        (log, path)
    }

    fn moderator(id: &str) -> QueueActor {
        QueueActor::new(id, ActorRole::Moderator)
    }

    fn senior(id: &str) -> QueueActor {
        QueueActor::new(id, ActorRole::SeniorModerator)
    }

    fn reporter_actor(id: &str) -> QueueActor {
        QueueActor::new(id, ActorRole::Reporter)
    }

    fn report_req(
        market_id: u64,
        reporter: &str,
        reason: ReportReason,
        ts: i64,
    ) -> SubmitReportRequest {
        SubmitReportRequest {
            market_id,
            content_kind: ReportedContentKind::MarketDescription,
            reporter: reporter.into(),
            reason,
            details: "Contains misleading settlement instructions.".into(),
            reported_at_ms: ts,
        }
    }

    fn valid_proof(actor: &str, action: &str, target: &str, nonce: &str, ts: i64) -> StepUpProof {
        StepUpProof {
            actor: actor.into(),
            action: action.into(),
            target: target.into(),
            nonce: nonce.into(),
            proof_timestamp_unix_ms: ts,
            signature: "valid-wallet-sig".into(),
        }
    }

    struct FlakyAuditSink {
        fail_remaining: u32,
        inner: ModerationAuditLog,
    }

    impl ModerationAuditSink for FlakyAuditSink {
        fn record_action(
            &mut self,
            actor: &str,
            action: &str,
            target: &str,
            reason: &str,
            timestamp_unix_ms: i64,
            proof_timestamp_unix_ms: Option<i64>,
            corrects_sequence: Option<u64>,
        ) -> Result<ModerationRecord, String> {
            if self.fail_remaining > 0 {
                self.fail_remaining -= 1;
                return Err("audit storage temporarily unavailable".to_string());
            }
            self.inner.record_action(
                actor,
                action,
                target,
                reason,
                timestamp_unix_ms,
                proof_timestamp_unix_ms,
                corrects_sequence,
            )
        }
    }

    #[test]
    fn report_aggregation_claim_and_hide_with_step_up_audit() {
        let mut queue = ModerationQueue::new();
        let (mut audit_log, path) = temp_audit_log();
        let mut verifier = StepUpVerifier::new(|_| true);

        let item = queue
            .submit_report(report_req(42, "user-a", ReportReason::Spam, 1_000))
            .unwrap();
        assert_eq!(item.priority, QueuePriority::Normal);
        assert_eq!(item.reports.len(), 1);

        // Second distinct reporter aggregates onto the same queue item and escalates priority.
        let item = queue
            .submit_report(report_req(42, "user-b", ReportReason::Misleading, 2_000))
            .unwrap();
        assert_eq!(item.item_id, 1);
        assert_eq!(item.reports.len(), 2);
        assert_eq!(item.priority, QueuePriority::High);

        let mod_actor = moderator("mod-1");
        let claimed = queue
            .claim_item(&mod_actor, item.item_id, item.revision, 3_000, None)
            .unwrap();
        assert_eq!(claimed.status, QueueStatus::InReview);
        assert_eq!(claimed.assigned_to.as_deref(), Some("mod-1"));

        let target = audit_target(42, ReportedContentKind::MarketDescription);
        let proof = valid_proof("mod-1", "hide_content", &target, "nonce-1", 3_900);
        let resolved = queue
            .resolve_item(
                &mod_actor,
                DecisionRequest {
                    item_id: claimed.item_id,
                    expected_revision: claimed.revision,
                    outcome: ModerationOutcome::Hidden,
                    reason: "Confirmed misleading settlement text".into(),
                    decided_at_ms: 4_000,
                },
                Some(&proof),
                &mut verifier,
                &mut audit_log,
            )
            .unwrap();

        assert_eq!(resolved.status, QueueStatus::Resolved);
        let decision = resolved.decision.unwrap();
        assert_eq!(decision.audit_sequence, Some(1));
        assert_eq!(decision.proof_timestamp_unix_ms, Some(3_900));
        assert!(matches!(
            queue.content_visibility(42, ReportedContentKind::MarketDescription),
            ContentVisibility::Hidden { item_id: 1, .. }
        ));
        assert!(audit_log.verify().unwrap().ok);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn duplicate_report_and_validation_boundaries_rejected() {
        let mut queue = ModerationQueue::new();
        queue
            .submit_report(report_req(10, "user-a", ReportReason::Other, 100))
            .unwrap();

        let dup = queue
            .submit_report(report_req(10, "user-a", ReportReason::Spam, 200))
            .unwrap_err();
        assert_eq!(
            dup,
            QueueError::DuplicateReport {
                item_id: 1,
                reporter: "user-a".into(),
            }
        );

        let mut bad = report_req(0, "user-b", ReportReason::Spam, 200);
        assert!(matches!(
            queue.submit_report(bad.clone()),
            Err(QueueError::Validation {
                field: "market_id",
                ..
            })
        ));
        bad.market_id = 10;
        bad.details = "x".repeat(MAX_REPORT_DETAILS_LEN + 1);
        assert!(matches!(
            queue.submit_report(bad),
            Err(QueueError::Validation {
                field: "details",
                ..
            })
        ));
    }

    #[test]
    fn authorization_escalation_and_step_up_boundaries_enforced() {
        let mut queue = ModerationQueue::new();
        let (mut audit_log, path) = temp_audit_log();
        let mut verifier = StepUpVerifier::new(|_| true);

        let item = queue
            .submit_report(report_req(7, "user-1", ReportReason::ProhibitedContent, 100))
            .unwrap();
        assert_eq!(item.priority, QueuePriority::Urgent);

        // Non-moderators cannot list or claim the queue.
        let regular_user = reporter_actor("user-1");
        assert!(matches!(
            queue.list_queue(&regular_user, &QueueFilter::default()),
            Err(QueueError::Unauthorized { .. })
        ));
        assert!(matches!(
            queue.claim_item(&regular_user, item.item_id, item.revision, 200, None),
            Err(QueueError::Unauthorized { .. })
        ));

        // Escalated items require senior_moderator or admin.
        let mod1 = moderator("mod-1");
        let esc = queue
            .escalate_item(
                &mod1,
                item.item_id,
                item.revision,
                "Potential legal violation",
                300,
            )
            .unwrap();
        assert_eq!(esc.status, QueueStatus::Escalated);

        assert!(matches!(
            queue.resolve_item(
                &mod1,
                DecisionRequest {
                    item_id: esc.item_id,
                    expected_revision: esc.revision,
                    outcome: ModerationOutcome::Dismissed,
                    reason: "Attempt by junior mod".into(),
                    decided_at_ms: 400,
                },
                None,
                &mut verifier,
                &mut audit_log,
            ),
            Err(QueueError::EscalationPrivilegeRequired { .. })
        ));

        // Senior moderator trying to hide without step-up proof is rejected.
        let sr = senior("sr-1");
        assert!(matches!(
            queue.resolve_item(
                &sr,
                DecisionRequest {
                    item_id: esc.item_id,
                    expected_revision: esc.revision,
                    outcome: ModerationOutcome::Hidden,
                    reason: "Confirmed prohibited content".into(),
                    decided_at_ms: 500,
                },
                None,
                &mut verifier,
                &mut audit_log,
            ),
            Err(QueueError::StepUpFailed(StepUpError::MissingProof))
        ));

        // Visibility remains visible until a valid proof and audit succeed.
        assert_eq!(
            queue.content_visibility(7, ReportedContentKind::MarketDescription),
            ContentVisibility::Visible
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn lease_expiry_reclaim_and_optimistic_concurrency_conflict() {
        let mut queue = ModerationQueue::new();
        let item = queue
            .submit_report(report_req(9, "u1", ReportReason::Spam, 1_000))
            .unwrap();

        let mod1 = moderator("mod-1");
        let mod2 = moderator("mod-2");

        let claimed = queue
            .claim_item(&mod1, item.item_id, item.revision, 2_000, Some(5_000))
            .unwrap();
        assert!(claimed.has_active_lease(6_999));
        assert!(!claimed.has_active_lease(7_000));

        // Stale revision is rejected.
        assert!(matches!(
            queue.claim_item(&mod2, item.item_id, item.revision, 3_000, None),
            Err(QueueError::RevisionConflict {
                expected: 1,
                actual: 2,
                ..
            })
        ));

        // Active lease held by mod-1 blocks mod-2.
        assert!(matches!(
            queue.claim_item(&mod2, claimed.item_id, claimed.revision, 4_000, None),
            Err(QueueError::AlreadyClaimed { .. })
        ));

        // Once lease expires, mod-2 can reclaim safely.
        let reclaimed = queue
            .claim_item(&mod2, claimed.item_id, claimed.revision, 7_000, None)
            .unwrap();
        assert_eq!(reclaimed.assigned_to.as_deref(), Some("mod-2"));
    }

    #[test]
    fn audit_failure_enters_explicit_recovery_state_and_retry_commits() {
        let mut queue = ModerationQueue::new();
        let (log, path) = temp_audit_log();
        let mut sink = FlakyAuditSink {
            fail_remaining: 1,
            inner: log,
        };
        let mut verifier = StepUpVerifier::new(|_| true);
        let mod1 = moderator("mod-1");

        let item = queue
            .submit_report(report_req(55, "u1", ReportReason::Harassment, 100))
            .unwrap();
        let claimed = queue
            .claim_item(&mod1, item.item_id, item.revision, 200, None)
            .unwrap();

        let target = audit_target(55, ReportedContentKind::MarketDescription);
        let proof = valid_proof("mod-1", "hide_content", &target, "n-retry", 290);

        // First attempt fails audit sink -> explicit AuditSyncFailed state, visibility unchanged.
        let err = queue
            .resolve_item(
                &mod1,
                DecisionRequest {
                    item_id: claimed.item_id,
                    expected_revision: claimed.revision,
                    outcome: ModerationOutcome::Hidden,
                    reason: "Abusive text".into(),
                    decided_at_ms: 300,
                },
                Some(&proof),
                &mut verifier,
                &mut sink,
            )
            .unwrap_err();
        assert!(matches!(err, QueueError::AuditSyncPending { item_id: 1, .. }));

        let failed_item = queue.get_item(&mod1, 1).unwrap();
        assert_eq!(failed_item.status, QueueStatus::AuditSyncFailed);
        assert_eq!(failed_item.pending_audit.as_ref().unwrap().attempts, 1);
        assert_eq!(
            queue.content_visibility(55, ReportedContentKind::MarketDescription),
            ContentVisibility::Visible
        );

        // Retry succeeds without re-running step-up proof, commits visibility and resolves item.
        let recovered = queue
            .retry_audit_sync(&mod1, 1, failed_item.revision, 400, &mut sink)
            .unwrap();
        assert_eq!(recovered.status, QueueStatus::Resolved);
        assert!(recovered.pending_audit.is_none());
        assert_eq!(recovered.decision.unwrap().audit_sequence, Some(1));
        assert!(matches!(
            queue.content_visibility(55, ReportedContentKind::MarketDescription),
            ContentVisibility::Hidden { item_id: 1, .. }
        ));
        assert!(sink.inner.verify().unwrap().ok);

        // Senior moderator can reopen on appeal, appending a correction record to the audit chain.
        let sr = senior("sr-1");
        let reopened = queue
            .reopen_item(
                &sr,
                recovered.item_id,
                recovered.revision,
                "Creator submitted appeal",
                500,
                &mut sink,
            )
            .unwrap();
        assert_eq!(reopened.status, QueueStatus::Escalated);
        assert_eq!(sink.inner.records().len(), 2);
        assert_eq!(sink.inner.records()[1].corrects_sequence, Some(1));
        assert!(sink.inner.verify().unwrap().ok);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn queue_listing_orders_by_priority_then_age_with_pagination() {
        let mut queue = ModerationQueue::new();
        queue
            .submit_report(report_req(1, "u1", ReportReason::Other, 100))
            .unwrap();
        queue
            .submit_report(report_req(2, "u1", ReportReason::ProhibitedContent, 300))
            .unwrap();
        queue
            .submit_report(report_req(3, "u1", ReportReason::Misleading, 200))
            .unwrap();

        let mod1 = moderator("mod-1");
        let page = queue
            .list_queue(
                &mod1,
                &QueueFilter {
                    limit: 2,
                    ..QueueFilter::default()
                },
            )
            .unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].market_id, 2);
        assert_eq!(page.items[1].market_id, 3);
    }
}
