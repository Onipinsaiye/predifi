// backend/src/content_reports.rs
//
// User-facing content report and appeal flow.
//
// Design notes:
// - Reports are keyed by (reporter, target, reason) and deduplicated within a
//   bounded time window. Duplicate submissions return the existing report.
// - Reporters can view the status of their own reports. Reviewers (moderators)
//   can list all reports and drive state transitions.
// - State machine:
//       Open -> UnderReview -> Resolved
//       Resolved -> Appealed (at most one appeal per report)
//       Appealed -> AppealUpheld | AppealRejected
// - Every reviewer action appends an audit record via `moderation_audit`.
// - This module NEVER touches on-chain funds or resolution. Content moderation
//   is strictly off-chain and cannot alter on-chain state.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::moderation_audit::{AuditAction, AuditRecord, ModerationAudit};

/// Default deduplication window (in seconds) for identical reports.
pub const DEFAULT_DEDUP_WINDOW_SECS: u64 = 24 * 60 * 60;

/// Maximum number of appeals allowed per report.
pub const MAX_APPEALS_PER_REPORT: u8 = 1;

/// Reason a user may flag content.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReportReason {
    Misleading,
    Spam,
    Harassment,
    Fraud,
    Other(String),
}

/// Lifecycle of a report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportStatus {
    Open,
    UnderReview,
    Resolved,
    Appealed,
    AppealUpheld,
    AppealRejected,
}

impl ReportStatus {
    /// Whether a reviewer may transition from `self` to `next`.
    pub fn can_transition_to(&self, next: &ReportStatus) -> bool {
        use ReportStatus::*;
        matches!(
            (self, next),
            (Open, UnderReview)
                | (UnderReview, Resolved)
                | (Resolved, Appealed)
                | (Appealed, AppealUpheld)
                | (Appealed, AppealRejected)
        )
    }

    /// Whether the report is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ReportStatus::AppealUpheld | ReportStatus::AppealRejected
        )
    }
}

/// A single content report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentReport {
    pub id: u64,
    pub reporter: String,
    pub target: String,
    pub reason: ReportReason,
    pub status: ReportStatus,
    pub created_at: u64,
    pub updated_at: u64,
    pub appeals_used: u8,
    /// Reviewer notes are visible only to reviewers, never to reporters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer_notes: Option<String>,
}

/// Public-facing view of a report, safe to return to the reporter.
/// Deliberately omits reviewer notes and any internal identifiers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReporterReportView {
    pub id: u64,
    pub target: String,
    pub reason: ReportReason,
    pub status: ReportStatus,
    pub created_at: u64,
    pub updated_at: u64,
    pub can_appeal: bool,
}

/// Errors returned by the report service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportError {
    NotFound,
    NotAuthorized,
    InvalidTransition,
    AppealLimitReached,
    Duplicate,
    InvalidInput(&'static str),
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReportError::NotFound => write!(f, "report not found"),
            ReportError::NotAuthorized => write!(f, "not authorized"),
            ReportError::InvalidTransition => write!(f, "invalid state transition"),
            ReportError::AppealLimitReached => write!(f, "appeal limit reached"),
            ReportError::Duplicate => write!(f, "duplicate report"),
            ReportError::InvalidInput(msg) => write!(f, "invalid input: {}", msg),
        }
    }
}

impl std::error::Error for ReportError {}

/// Key used for deduplication.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DedupKey {
    reporter: String,
    target: String,
    reason: ReportReason,
}

/// In-memory content report service.
///
/// In production this would be backed by a persistent store; the interface is
/// kept storage-agnostic so it can be swapped without touching callers.
pub struct ContentReportService {
    inner: Mutex<Inner>,
    dedup_window_secs: u64,
    audit: ModerationAudit,
}

struct Inner {
    next_id: u64,
    reports: HashMap<u64, ContentReport>,
    /// Maps a dedup key to the report id created within the window.
    dedup_index: HashMap<DedupKey, (u64, u64)>, // (report_id, created_at)
}

impl ContentReportService {
    pub fn new(audit: ModerationAudit) -> Self {
        Self::with_dedup_window(audit, DEFAULT_DEDUP_WINDOW_SECS)
    }

    pub fn with_dedup_window(audit: ModerationAudit, dedup_window_secs: u64) -> Self {
        Self {
            inner: Mutex::new(Inner {
                next_id: 1,
                reports: HashMap::new(),
                dedup_index: HashMap::new(),
            }),
            dedup_window_secs,
            audit,
        }
    }

    /// Submit a report. If an identical report (same reporter, target, and
    /// reason) was submitted within the dedup window, the existing report is
    /// returned instead of creating a new one.
    pub fn submit_report(
        &self,
        reporter: &str,
        target: &str,
        reason: ReportReason,
        now: u64,
    ) -> Result<ContentReport, ReportError> {
        if reporter.trim().is_empty() {
            return Err(ReportError::InvalidInput("reporter must not be empty"));
        }
        if target.trim().is_empty() {
            return Err(ReportError::InvalidInput("target must not be empty"));
        }

        let key = DedupKey {
            reporter: reporter.to_string(),
            target: target.to_string(),
            reason: reason.clone(),
        };

        let mut inner = self.inner.lock().expect("content report lock poisoned");

        if let Some((existing_id, created_at)) = inner.dedup_index.get(&key) {
            if now.saturating_sub(*created_at) < self.dedup_window_secs {
                if let Some(existing) = inner.reports.get(existing_id) {
                    return Ok(existing.clone());
                }
            }
        }

        let id = inner.next_id;
        inner.next_id += 1;

        let report = ContentReport {
            id,
            reporter: reporter.to_string(),
            target: target.to_string(),
            reason,
            status: ReportStatus::Open,
            created_at: now,
            updated_at: now,
            appeals_used: 0,
            reviewer_notes: None,
        };

        inner.reports.insert(id, report.clone());
        inner.dedup_index.insert(key, (id, now));

        Ok(report)
    }

    /// Return the reporter-facing view of a report. Only the original reporter
    /// may view it. Reviewer notes are never exposed here.
    pub fn get_reporter_view(
        &self,
        report_id: u64,
        requester: &str,
    ) -> Result<ReporterReportView, ReportError> {
        let inner = self.inner.lock().expect("content report lock poisoned");
        let report = inner.reports.get(&report_id).ok_or(ReportError::NotFound)?;

        if report.reporter != requester {
            return Err(ReportError::NotAuthorized);
        }

        Ok(ReporterReportView {
            id: report.id,
            target: report.target.clone(),
            reason: report.reason.clone(),
            status: report.status.clone(),
            created_at: report.created_at,
            updated_at: report.updated_at,
            can_appeal: report.status == ReportStatus::Resolved
                && report.appeals_used < MAX_APPEALS_PER_REPORT,
        })
    }

    /// List all reports. Reviewer-only.
    pub fn list_reports_for_reviewer(
        &self,
        reviewer: &str,
    ) -> Result<Vec<ContentReport>, ReportError> {
        self.require_reviewer(reviewer)?;
        let inner = self.inner.lock().expect("content report lock poisoned");
        let mut reports: Vec<ContentReport> = inner.reports.values().cloned().collect();
        reports.sort_by_key(|r| r.id);
        Ok(reports)
    }

    /// Reviewer transitions a report to a new status. Appends an audit record.
    pub fn reviewer_transition(
        &self,
        reviewer: &str,
        report_id: u64,
        next: ReportStatus,
        notes: Option<String>,
        now: u64,
    ) -> Result<ContentReport, ReportError> {
        self.require_reviewer(reviewer)?;

        let mut inner = self.inner.lock().expect("content report lock poisoned");
        let report = inner
            .reports
            .get_mut(&report_id)
            .ok_or(ReportError::NotFound)?;

        if !report.status.can_transition_to(&next) {
            return Err(ReportError::InvalidTransition);
        }

        let previous = report.status.clone();
        report.status = next.clone();
        report.updated_at = now;
        if notes.is_some() {
            report.reviewer_notes = notes;
        }

        let updated = report.clone();
        drop(inner);

        self.audit.append(AuditRecord {
            actor: reviewer.to_string(),
            action: AuditAction::ContentReportTransition {
                report_id,
                from: previous,
                to: next,
            },
            timestamp: now,
        });

        Ok(updated)
    }

    /// Reporter files an appeal against a resolved report. At most one appeal
    /// is allowed per report.
    pub fn file_appeal(
        &self,
        reporter: &str,
        report_id: u64,
        now: u64,
    ) -> Result<ContentReport, ReportError> {
        let mut inner = self.inner.lock().expect("content report lock poisoned");
        let report = inner
            .reports
            .get_mut(&report_id)
            .ok_or(ReportError::NotFound)?;

        if report.reporter != reporter {
            return Err(ReportError::NotAuthorized);
        }

        if report.appeals_used >= MAX_APPEALS_PER_REPORT {
            return Err(ReportError::AppealLimitReached);
        }

        if report.status != ReportStatus::Resolved {
            return Err(ReportError::InvalidTransition);
        }

        report.appeals_used += 1;
        report.status = ReportStatus::Appealed;
        report.updated_at = now;

        Ok(report.clone())
    }

    /// Reviewer resolves an appeal as upheld or rejected.
    pub fn resolve_appeal(
        &self,
        reviewer: &str,
        report_id: u64,
        upheld: bool,
        now: u64,
    ) -> Result<ContentReport, ReportError> {
        self.require_reviewer(reviewer)?;

        let next = if upheld {
            ReportStatus::AppealUpheld
        } else {
            ReportStatus::AppealRejected
        };

        self.reviewer_transition(reviewer, report_id, next, None, now)
    }

    fn require_reviewer(&self, reviewer: &str) -> Result<(), ReportError> {
        if self.audit.is_reviewer(reviewer) {
            Ok(())
        } else {
            Err(ReportError::NotAuthorized)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> ContentReportService {
        let audit = ModerationAudit::new();
        audit.register_reviewer("moderator-1");
        audit.register_reviewer("moderator-2");
        ContentReportService::with_dedup_window(audit, 1000)
    }

    #[test]
    fn deduplicates_identical_reports_within_window() {
        let svc = service();
        let r1 = svc
            .submit_report("alice", "post-1", ReportReason::Misleading, 100)
            .unwrap();
        let r2 = svc
            .submit_report("alice", "post-1", ReportReason::Misleading, 200)
            .unwrap();
        assert_eq!(r1.id, r2.id, "duplicate within window returns same report");

        // Different reason is a distinct report.
        let r3 = svc
            .submit_report("alice", "post-1", ReportReason::Spam, 200)
            .unwrap();
        assert_ne!(r1.id, r3.id);

        // Different reporter is a distinct report.
        let r4 = svc
            .submit_report("bob", "post-1", ReportReason::Misleading, 200)
            .unwrap();
        assert_ne!(r1.id, r4.id);
    }

    #[test]
    fn dedup_window_expires() {
        let svc = service();
        let r1 = svc
            .submit_report("alice", "post-1", ReportReason::Misleading, 100)
            .unwrap();
        let r2 = svc
            .submit_report("alice", "post-1", ReportReason::Misleading, 2000)
            .unwrap();
        assert_ne!(r1.id, r2.id, "outside window creates new report");
    }

    #[test]
    fn rejects_empty_inputs() {
        let svc = service();
        assert_eq!(
            svc.submit_report("", "post-1", ReportReason::Spam, 0),
            Err(ReportError::InvalidInput("reporter must not be empty"))
        );
        assert_eq!(
            svc.submit_report("alice", "  ", ReportReason::Spam, 0),
            Err(ReportError::InvalidInput("target must not be empty"))
        );
    }

    #[test]
    fn reporter_can_only_view_own_report() {
        let svc = service();
        let r = svc
            .submit_report("alice", "post-1", ReportReason::Fraud, 10)
            .unwrap();

        let view = svc.get_reporter_view(r.id, "alice").unwrap();
        assert_eq!(view.status, ReportStatus::Open);
        assert!(!view.can_appeal);

        assert_eq!(
            svc.get_reporter_view(r.id, "bob"),
            Err(ReportError::NotAuthorized)
        );
    }

    #[test]
    fn reviewer_notes_not_exposed_to_reporter() {
        let svc = service();
        let r = svc
            .submit_report("alice", "post-1", ReportReason::Fraud, 10)
            .unwrap();
        svc.reviewer_transition(
            "moderator-1",
            r.id,
            ReportStatus::UnderReview,
            Some("internal note".to_string()),
            20,
        )
        .unwrap();

        let view = svc.get_reporter_view(r.id, "alice").unwrap();
        // Serialized reporter view must not contain reviewer notes.
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("internal note"));
        assert!(!json.contains("reviewer_notes"));
    }

    #[test]
    fn non_reviewer_cannot_list_or_transition() {
        let svc = service();
        let r = svc
            .submit_report("alice", "post-1", ReportReason::Spam, 10)
            .unwrap();

        assert_eq!(
            svc.list_reports_for_reviewer("alice"),
            Err(ReportError::NotAuthorized)
        );
        assert_eq!(
            svc.reviewer_transition("alice", r.id, ReportStatus::UnderReview, None, 20),
            Err(ReportError::NotAuthorized)
        );
    }

    #[test]
    fn reviewer_can_list_all_reports() {
        let svc = service();
        svc.submit_report("alice", "post-1", ReportReason::Spam, 10)
            .unwrap();
        svc.submit_report("bob", "post-2", ReportReason::Fraud, 10)
            .unwrap();

        let list = svc.list_reports_for_reviewer("moderator-1").unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn invalid_transitions_rejected() {
        let svc = service();
        let r = svc
            .submit_report("alice", "post-1", ReportReason::Spam, 10)
            .unwrap();

        // Cannot jump straight to Resolved.
        assert_eq!(
            svc.reviewer_transition("moderator-1", r.id, ReportStatus::Resolved, None, 20),
            Err(ReportError::InvalidTransition)
        );
    }

    #[test]
    fn full_lifecycle_and_appeal() {
        let svc = service();
        let r = svc
            .submit_report("alice", "post-1", ReportReason::Misleading, 10)
            .unwrap();

        svc.reviewer_transition("moderator-1", r.id, ReportStatus::UnderReview, None, 20)
            .unwrap();
        svc.reviewer_transition("moderator-1", r.id, ReportStatus::Resolved, None, 30)
            .unwrap();

        let view = svc.get_reporter_view(r.id, "alice").unwrap();
        assert!(view.can_appeal);

        let appealed = svc.file_appeal("alice", r.id, 40).unwrap();
        assert_eq!(appealed.status, ReportStatus::Appealed);
        assert_eq!(appealed.appeals_used, 1);

        // Second appeal rejected.
        assert_eq!(
            svc.file_appeal("alice", r.id, 50),
            Err(ReportError::AppealLimitReached)
        );

        let resolved = svc.resolve_appeal("moderator-1", r.id, true, 60).unwrap();
        assert_eq!(resolved.status, ReportStatus::AppealUpheld);
        assert!(resolved.status.is_terminal());
    }

    #[test]
    fn appeal_rejected_path() {
        let svc = service();
        let r = svc
            .submit_report("alice", "post-1", ReportReason::Spam, 10)
            .unwrap();
        svc.reviewer_transition("moderator-1", r.id, ReportStatus::UnderReview, None, 20)
            .unwrap();
        svc.reviewer_transition("moderator-1", r.id, ReportStatus::Resolved, None, 30)
            .unwrap();
        svc.file_appeal("alice", r.id, 40).unwrap();
        let resolved = svc.resolve_appeal("moderator-1", r.id, false, 50).unwrap();
        assert_eq!(resolved.status, ReportStatus::AppealRejected);
    }

    #[test]
    fn only_reporter_can_appeal() {
        let svc = service();
        let r = svc
            .submit_report("alice", "post-1", ReportReason::Spam, 10)
            .unwrap();
        svc.reviewer_transition("moderator-1", r.id, ReportStatus::UnderReview, None, 20)
            .unwrap();
        svc.reviewer_transition("moderator-1", r.id, ReportStatus::Resolved, None, 30)
            .unwrap();

        assert_eq!(
            svc.file_appeal("bob", r.id, 40),
            Err(ReportError::NotAuthorized)
        );
    }

    #[test]
    fn cannot_appeal_unresolved_report() {
        let svc = service();
        let r = svc
            .submit_report("alice", "post-1", ReportReason::Spam, 10)
            .unwrap();
        assert_eq!(
            svc.file_appeal("alice", r.id, 20),
            Err(ReportError::InvalidTransition)
        );
    }

    #[test]
    fn reviewer_actions_are_audited() {
        let audit = ModerationAudit::new();
        audit.register_reviewer("moderator-1");
        let svc = ContentReportService::with_dedup_window(audit.clone(), 1000);

        let r = svc
            .submit_report("alice", "post-1", ReportReason::Spam, 10)
            .unwrap();
        svc.reviewer_transition("moderator-1", r.id, ReportStatus::UnderReview, None, 20)
            .unwrap();
        svc.reviewer_transition("moderator-1", r.id, ReportStatus::Resolved, None, 30)
            .unwrap();

        let records = audit.records();
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|rec| rec.actor == "moderator-1"));
    }
}