use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::{self, Display};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;

/// Privileged moderation actions that require a fresh, action-bound step-up proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegedAction {
    HideContent,
    UnhideContent,
    ViewPrivateReport,
    ExportReports,
}

impl PrivilegedAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            PrivilegedAction::HideContent => "hide_content",
            PrivilegedAction::UnhideContent => "unhide_content",
            PrivilegedAction::ViewPrivateReport => "view_private_report",
            PrivilegedAction::ExportReports => "export_reports",
        }
    }

    /// Returns true if this action is privileged and requires step-up verification.
    pub fn is_privileged(action: &str) -> bool {
        matches!(
            action,
            "hide_content" | "unhide_content" | "view_private_report" | "export_reports"
        )
    }
}

/// A wallet-signed, action-bound step-up proof.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepUpProof {
    pub actor: String,
    pub action: String,
    pub target: String,
    pub nonce: String,
    pub proof_timestamp_unix_ms: i64,
    pub signature: String,
}

/// Maximum age of a step-up proof in milliseconds.
pub const PROOF_MAX_AGE_MS: i64 = 5 * 60 * 1000;

/// Errors produced by step-up verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepUpError {
    MissingProof,
    ActorMismatch,
    ActionMismatch,
    TargetMismatch,
    StaleProof { age_ms: i64 },
    FutureProof,
    ReplayedNonce,
    InvalidSignature,
}

impl Display for StepUpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StepUpError::MissingProof => write!(f, "step-up proof required"),
            StepUpError::ActorMismatch => write!(f, "proof actor does not match"),
            StepUpError::ActionMismatch => write!(f, "proof action does not match"),
            StepUpError::TargetMismatch => write!(f, "proof target does not match"),
            StepUpError::StaleProof { age_ms } => write!(f, "step-up proof stale ({age_ms} ms old)"),
            StepUpError::FutureProof => write!(f, "step-up proof timestamp in future"),
            StepUpError::ReplayedNonce => write!(f, "step-up proof nonce already used"),
            StepUpError::InvalidSignature => write!(f, "step-up proof signature invalid"),
        }
    }
}

impl std::error::Error for StepUpError {}

/// Verifies step-up proofs: freshness, action/target binding, and replay protection.
/// The signature verifier is supplied by the caller (wallet signature check).
pub struct StepUpVerifier<F>
where
    F: Fn(&StepUpProof) -> bool,
{
    verify_signature: F,
    used_nonces: std::collections::HashSet<String>,
}

impl<F> StepUpVerifier<F>
where
    F: Fn(&StepUpProof) -> bool,
{
    pub fn new(verify_signature: F) -> Self {
        Self {
            verify_signature,
            used_nonces: std::collections::HashSet::new(),
        }
    }

    /// Verify a proof for a privileged action performed by `actor` on `target`.
    /// Non-privileged actions pass without a proof. Privileged actions require
    /// a fresh, action-bound, unused proof.
    pub fn verify(
        &mut self,
        actor: &str,
        action: &str,
        target: &str,
        proof: Option<&StepUpProof>,
        now_unix_ms: i64,
    ) -> Result<(), StepUpError> {
        if !PrivilegedAction::is_privileged(action) {
            return Ok(());
        }
        let proof = proof.ok_or(StepUpError::MissingProof)?;
        if proof.actor != actor {
            return Err(StepUpError::ActorMismatch);
        }
        if proof.action != action {
            return Err(StepUpError::ActionMismatch);
        }
        if proof.target != target {
            return Err(StepUpError::TargetMismatch);
        }
        let age = now_unix_ms - proof.proof_timestamp_unix_ms;
        if age < 0 {
            return Err(StepUpError::FutureProof);
        }
        if age > PROOF_MAX_AGE_MS {
            return Err(StepUpError::StaleProof { age_ms: age });
        }
        if !(self.verify_signature)(proof) {
            return Err(StepUpError::InvalidSignature);
        }
        if self.used_nonces.contains(&proof.nonce) {
            return Err(StepUpError::ReplayedNonce);
        }
        self.used_nonces.insert(proof.nonce.clone());
        Ok(())
    }
}

/// Explicit lifecycle states for reports, reviews, actions, and appeals.
///
/// Reports and appeals move through an explicit state machine rather than
/// mutable boolean flags so every transition is auditable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationState {
    /// A report has been filed and awaits triage.
    ReportSubmitted,
    /// A reviewer has claimed the report for review.
    UnderReview,
    /// A moderation action (e.g. hide content) has been applied.
    ActionTaken,
    /// The report was dismissed without action.
    Dismissed,
    /// An appeal against an action has been filed.
    AppealSubmitted,
    /// The appeal was upheld (action reversed).
    AppealUpheld,
    /// The appeal was rejected (action stands).
    AppealRejected,
}

impl ModerationState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ModerationState::ReportSubmitted => "report_submitted",
            ModerationState::UnderReview => "under_review",
            ModerationState::ActionTaken => "action_taken",
            ModerationState::Dismissed => "dismissed",
            ModerationState::AppealSubmitted => "appeal_submitted",
            ModerationState::AppealUpheld => "appeal_upheld",
            ModerationState::AppealRejected => "appeal_rejected",
        }
    }

    /// Returns true if `from -> to` is a legal moderation transition.
    ///
    /// Legal transitions:
    /// - ReportSubmitted -> UnderReview | Dismissed
    /// - UnderReview -> ActionTaken | Dismissed
    /// - ActionTaken -> AppealSubmitted
    /// - AppealSubmitted -> AppealUpheld | AppealRejected
    /// - Dismissed, AppealUpheld, AppealRejected are terminal.
    pub fn is_legal_transition(from: ModerationState, to: ModerationState) -> bool {
        use ModerationState::*;
        matches!(
            (from, to),
            (ReportSubmitted, UnderReview)
                | (ReportSubmitted, Dismissed)
                | (UnderReview, ActionTaken)
                | (UnderReview, Dismissed)
                | (ActionTaken, AppealSubmitted)
                | (AppealSubmitted, AppealUpheld)
                | (AppealSubmitted, AppealRejected)
        )
    }

    /// Returns true if this state is terminal (no further transitions allowed).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ModerationState::Dismissed
                | ModerationState::AppealUpheld
                | ModerationState::AppealRejected
        )
    }
}

/// Reviewer roles permitted to perform moderation transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewerRole {
    /// Can triage reports and dismiss them.
    Reviewer,
    /// Can additionally take moderation actions.
    SeniorReviewer,
    /// Can additionally adjudicate appeals.
    AppealJudge,
}

impl ReviewerRole {
    /// Returns true if this role may perform the given transition.
    pub fn can_perform(&self, from: ModerationState, to: ModerationState) -> bool {
        use ModerationState::*;
        match (from, to) {
            (ReportSubmitted, UnderReview) | (ReportSubmitted, Dismissed) => true,
            (UnderReview, ActionTaken) => matches!(
                self,
                ReviewerRole::SeniorReviewer | ReviewerRole::AppealJudge
            ),
            (UnderReview, Dismissed) => true,
            (ActionTaken, AppealSubmitted) => true,
            (AppealSubmitted, AppealUpheld) | (AppealSubmitted, AppealRejected) => {
                matches!(self, ReviewerRole::AppealJudge)
            }
            _ => false,
        }
    }
}

/// Errors produced by the moderation state machine service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModerationTransitionError {
    /// The requested transition is not part of the legal state machine.
    ForbiddenTransition {
        from: ModerationState,
        to: ModerationState,
    },
    /// The actor's role is not permitted to perform this transition.
    PermissionDenied {
        role: ReviewerRole,
        from: ModerationState,
        to: ModerationState,
    },
    /// The transition was already recorded (idempotent duplicate submission).
    DuplicateTransition {
        from: ModerationState,
        to: ModerationState,
    },
    /// No report with the given id exists.
    UnknownReport { report_id: String },
    /// The audit log rejected the record.
    Audit(ModerationAuditError),
}

impl Display for ModerationTransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModerationTransitionError::ForbiddenTransition { from, to } => {
                write!(f, "forbidden transition {} -> {}", from.as_str(), to.as_str())
            }
            ModerationTransitionError::PermissionDenied { role, from, to } => write!(
                f,
                "role {:?} may not perform {} -> {}",
                role,
                from.as_str(),
                to.as_str()
            ),
            ModerationTransitionError::DuplicateTransition { from, to } => write!(
                f,
                "duplicate transition {} -> {}",
                from.as_str(),
                to.as_str()
            ),
            ModerationTransitionError::UnknownReport { report_id } => {
                write!(f, "unknown report {report_id}")
            }
            ModerationTransitionError::Audit(e) => write!(f, "audit error: {e}"),
        }
    }
}

impl std::error::Error for ModerationTransitionError {}

impl From<ModerationAuditError> for ModerationTransitionError {
    fn from(e: ModerationAuditError) -> Self {
        ModerationTransitionError::Audit(e)
    }
}

/// A single append-only moderation audit record.
///
/// Each record is hashed together with the previous record's hash, forming a
/// tamper-evident hash chain. Corrections are appended as new records that
/// reference the record being corrected rather than mutating history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModerationRecord {
    pub sequence: u64,
    pub timestamp_unix_ms: i64,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub reason: String,
    /// Report this transition belongs to, if it is a state-machine transition.
    #[serde(default)]
    pub report_id: Option<String>,
    /// State the report was in before this transition.
    #[serde(default)]
    pub from_state: Option<ModerationState>,
    /// State the report moved to as a result of this transition.
    #[serde(default)]
    pub to_state: Option<ModerationState>,
    /// Timestamp of the step-up proof that authorized this decision, if privileged.
    #[serde(default)]
    pub proof_timestamp_unix_ms: Option<i64>,
    /// Optional reference to a prior sequence number this record corrects.
    pub corrects_sequence: Option<u64>,
    /// Hash of the previous record in the chain (zero hash for the genesis record).
    pub prev_hash: String,
    /// Hex-encoded SHA-256 of this record's content including prev_hash.
    pub record_hash: String,
}

/// Result of verifying a chain of audit records.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerificationResult {
    pub ok: bool,
    pub checked_records: usize,
    pub errors: Vec<VerificationError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationError {
    pub index: usize,
    pub sequence: u64,
    pub kind: VerificationErrorKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationErrorKind {
    /// Stored hash does not match the recomputed hash of the record's content.
    HashMismatch { expected: String, actual: String },
    /// Prev-hash pointer does not match the preceding record's computed hash.
    BrokenPrevLink { expected: String, actual: String },
    /// Sequence number is not monotonically increasing (e.g. reordered or duplicated).
    UnexpectedSequence { expected: u64, actual: u64 },
    /// Chain is empty but a non-zero genesis was expected, or vice-versa.
    EmptyChain,
    /// A record references a correction target that does not exist in the chain.
    DanglingCorrection { target_sequence: u64 },
    /// A record is not serializable or has invalid fields.
    InvalidRecord { detail: String },
    /// A privileged action was recorded without a step-up proof timestamp.
    MissingStepUpProof { action: String },
}

const GENESIS_PREV_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// Domain separator so audit hashes cannot be confused with other SHA-256 uses.
const HASH_DOMAIN: &str = "bounty-moderation-audit-v1";

/// Compute the canonical hash for a record. The hash covers every field except
/// `record_hash` itself, and is prefixed with a stable domain separator.
pub fn compute_record_hash(record: &ModerationRecord) -> Result<String, VerificationErrorKind> {
    let canonical = serialize_canonical(record)?;
    let mut hasher = Sha256::new();
    hasher.update(HASH_DOMAIN.as_bytes());
    hasher.update(b"\n");
    hasher.update(canonical.as_bytes());
    Ok(hex_encode(&hasher.finalize()))
}

/// Serialize the record in a deterministic form suitable for hashing.
fn serialize_canonical(record: &ModerationRecord) -> Result<String, VerificationErrorKind> {
    let mut clone = record.clone();
    clone.record_hash.clear();
    serde_json::to_string(&clone).map_err(|e| VerificationErrorKind::InvalidRecord {
        detail: e.to_string(),
    })
}

/// Append-only moderation audit log backed by a file on disk.
pub struct ModerationAuditLog {
    path: PathBuf,
    records: Vec<ModerationRecord>,
}

impl ModerationAuditLog {
    /// Open (or create) an audit log at the given path.
    pub fn open(path: impl AsRef<str>) -> Result<Self, ModerationAuditError> {
        let path = PathBuf::from(path.as_ref());
        let records = if path.exists() {
            load_records(&path)?
        } else {
            Vec::new()
        };
        let log = Self { path, records };
        log.verify().map_err(ModerationAuditError::VerificationFailed)?;
        Ok(log)
    }

    pub fn records(&self) -> &[ModerationRecord] {
        &self.records
    }

    /// Append a new moderation action. Privileged actions must supply a proof timestamp
    /// that is present; the step-up proof itself must be verified by `StepUpVerifier`
    /// before calling this. Returns the appended record.
    pub fn append(
        &mut self,
        actor: impl Into<String>,
        action: impl Into<String>,
        target: impl Into<String>,
        reason: impl Into<String>,
        timestamp_unix_ms: i64,
        proof_timestamp_unix_ms: Option<i64>,
    ) -> Result<ModerationRecord, ModerationAuditError> {
        self.append_with_correction(
            actor,
            action,
            target,
            reason,
            timestamp_unix_ms,
            proof_timestamp_unix_ms,
            None,
        )
    }

    /// Append a correction to an existing record.
    pub fn correct(
        &mut self,
        corrects_sequence: u64,
        actor: impl Into<String>,
        action: impl Into<String>,
        target: impl Into<String>,
        reason: impl Into<String>,
        timestamp_unix_ms: i64,
        proof_timestamp_unix_ms: Option<i64>,
    ) -> Result<ModerationRecord, ModerationAuditError> {
        if !self.records.iter().any(|r| r.sequence == corrects_sequence) {
            return Err(ModerationAuditError::VerificationFailed(VerificationError {
                index: self.records.len(),
                sequence: corrects_sequence,
                kind: VerificationErrorKind::DanglingCorrection {
                    target_sequence: corrects_sequence,
                },
            }));
        }
        self.append_with_correction(
            actor,
            action,
            target,
            reason,
            timestamp_unix_ms,
            proof_timestamp_unix_ms,
            Some(corrects_sequence),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn append_with_correction(
        &mut self,
        actor: impl Into<String>,
        action: impl Into<String>,
        target: impl Into<String>,
        reason: impl Into<String>,
        timestamp_unix_ms: i64,
        proof_timestamp_unix_ms: Option<i64>,
        corrects_sequence: Option<u64>,
    ) -> Result<ModerationRecord, ModerationAuditError> {
        let action = action.into();
        // Every privileged decision must carry the timestamp of its step-up proof
        // so the audit trail shows when the actor was re-verified.
        if PrivilegedAction::is_privileged(&action) && proof_timestamp_unix_ms.is_none() {
            return Err(ModerationAuditError::VerificationFailed(VerificationError {
                index: self.records.len(),
                sequence: self.records.last().map(|r| r.sequence + 1).unwrap_or(1),
                kind: VerificationErrorKind::MissingStepUpProof {
                    action: action.clone(),
                },
            }));
        }
        let sequence = self.records.last().map(|r| r.sequence + 1).unwrap_or(1);
        let prev_hash = self
            .records
            .last()
            .map(|r| r.record_hash.clone())
            .unwrap_or_else(|| GENESIS_PREV_HASH.to_string());
        let mut record = ModerationRecord {
            sequence,
            timestamp_unix_ms,
            actor: actor.into(),
            action,
            target: target.into(),
            reason: reason.into(),
            report_id: None,
            from_state: None,
            to_state: None,
            proof_timestamp_unix_ms,
            corrects_sequence,
            prev_hash,
            record_hash: String::new(),
        };
        record.record_hash = compute_record_hash(&record).map_err(|kind| {
            ModerationAuditError::VerificationFailed(VerificationError {
                index: self.records.len(),
                sequence: record.sequence,
                kind,
            })
        })?;
        append_record_to_disk(&self.path, &record)?;
        self.records.push(record.clone());
        Ok(record)
    }

    pub fn verify(&self) -> Result<VerificationResult, VerificationError> {
        verify_records(&self.records)
    }

    pub fn verify_from_disk(&self) -> Result<VerificationResult, ModerationAuditError> {
        let records = load_records(&self.path)?;
        verify_records(&records).map_err(ModerationAuditError::VerificationFailed)
    }

    /// Append a state-machine transition record. Unlike `append`, this records
    /// the report id and the from/to states so the full lifecycle is auditable.
    #[allow(clippy::too_many_arguments)]
    fn append_transition(
        &mut self,
        report_id: &str,
        actor: &str,
        from: ModerationState,
        to: ModerationState,
        reason: &str,
        timestamp_unix_ms: i64,
        proof_timestamp_unix_ms: Option<i64>,
    ) -> Result<ModerationRecord, ModerationAuditError> {
        let action = format!("transition:{}->{}", from.as_str(), to.as_str());
        if PrivilegedAction::is_privileged(&action) && proof_timestamp_unix_ms.is_none() {
            return Err(ModerationAuditError::VerificationFailed(VerificationError {
                index: self.records.len(),
                sequence: self.records.last().map(|r| r.sequence + 1).unwrap_or(1),
                kind: VerificationErrorKind::MissingStepUpProof {
                    action: action.clone(),
                },
            }));
        }
        let sequence = self.records.last().map(|r| r.sequence + 1).unwrap_or(1);
        let prev_hash = self
            .records
            .last()
            .map(|r| r.record_hash.clone())
            .unwrap_or_else(|| GENESIS_PREV_HASH.to_string());
        let mut record = ModerationRecord {
            sequence,
            timestamp_unix_ms,
            actor: actor.to_string(),
            action,
            target: report_id.to_string(),
            reason: reason.to_string(),
            report_id: Some(report_id.to_string()),
            from_state: Some(from),
            to_state: Some(to),
            proof_timestamp_unix_ms,
            corrects_sequence: None,
            prev_hash,
            record_hash: String::new(),
        };
        record.record_hash = compute_record_hash(&record).map_err(|kind| {
            ModerationAuditError::VerificationFailed(VerificationError {
                index: self.records.len(),
                sequence: record.sequence,
                kind,
            })
        })?;
        append_record_to_disk(&self.path, &record)?;
        self.records.push(record.clone());
        Ok(record)
    }

    /// Returns the current state of a report based on the audit history, if any.
    pub fn current_state(&self, report_id: &str) -> Option<ModerationState> {
        self.records
            .iter()
            .rev()
            .find(|r| r.report_id.as_deref() == Some(report_id))
            .and_then(|r| r.to_state)
    }

    /// Returns the ordered transition history for a report.
    pub fn history(&self, report_id: &str) -> Vec<&ModerationRecord> {
        self.records
            .iter()
            .filter(|r| r.report_id.as_deref() == Some(report_id))
            .collect()
    }

    /// Apply a moderation state transition on behalf of `actor` with `role`.
    ///
    /// Enforces, in order:
    /// 1. The transition is legal in the state machine.
    /// 2. The actor's role is permitted to perform it.
    /// 3. The transition is not a duplicate of the report's current state
    ///    (idempotent: retries return the existing record without appending).
    ///
    /// On success, appends exactly one audit record capturing actor, timestamp,
    /// and from/to states, and returns it.
    #[allow(clippy::too_many_arguments)]
    pub fn transition(
        &mut self,
        report_id: &str,
        actor: &str,
        role: ReviewerRole,
        to: ModerationState,
        reason: &str,
        timestamp_unix_ms: i64,
        proof_timestamp_unix_ms: Option<i64>,
    ) -> Result<ModerationRecord, ModerationTransitionError> {
        let from = self
            .current_state(report_id)
            .ok_or_else(|| ModerationTransitionError::UnknownReport {
                report_id: report_id.to_string(),
            })?;

        // Idempotent duplicate: the report is already in the requested state.
        if from == to {
            return Err(ModerationTransitionError::DuplicateTransition { from, to });
        }

        if !ModerationState::is_legal_transition(from, to) {
            return Err(ModerationTransitionError::ForbiddenTransition { from, to });
        }

        if !role.can_perform(from, to) {
            return Err(ModerationTransitionError::PermissionDenied { role, from, to });
        }

        let record = self.append_transition(
            report_id,
            actor,
            from,
            to,
            reason,
            timestamp_unix_ms,
            proof_timestamp_unix_ms,
        )?;
        Ok(record)
    }

    /// File a new report, moving it into `ReportSubmitted`.
    pub fn submit_report(
        &mut self,
        report_id: &str,
        actor: &str,
        reason: &str,
        timestamp_unix_ms: i64,
    ) -> Result<ModerationRecord, ModerationTransitionError> {
        if self.current_state(report_id).is_some() {
            return Err(ModerationTransitionError::DuplicateTransition {
                from: ModerationState::ReportSubmitted,
                to: ModerationState::ReportSubmitted,
            });
        }
        let record = self.append_transition(
            report_id,
            actor,
            ModerationState::ReportSubmitted,
            ModerationState::ReportSubmitted,
            reason,
            timestamp_unix_ms,
            None,
        )?;
        Ok(record)
    }
}

/// Errors returned by the audit log.
#[derive(Debug)]
pub enum ModerationAuditError {
    IoError(io::Error),
    SerializationError(serde_json::Error),
    VerificationFailed(VerificationError),
    /// The on-disk chain contains a record that cannot be parsed.
    CorruptRecord { index: usize, detail: String },
}

impl Display for ModerationAuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModerationAuditError::IoError(e) => write!(f, "io error: {e}"),
            ModerationAuditError::SerializationError(e) => write!(f, "serialization error: {e}"),
            ModerationAuditError::VerificationFailed(e) => write!(f, "verification failed: {e:?}"),
            ModerationAuditError::CorruptRecord { index, detail } => {
                write!(f, "corrupt record at index {index}: {detail}")
            }
        }
    }
}

impl std::error::Error for ModerationAuditError {}

impl From<io::Error> for ModerationAuditError {
    fn from(e: io::Error) -> Self {
        ModerationAuditError::IoError(e)
    }
}

impl From<serde_json::Error> for ModerationAuditError {
    fn from(e: serde_json::Error) -> Self {
        ModerationAuditError::SerializationError(e)
    }
}

/// Verify a slice of records. Detects modification, reordering, and truncation.
pub fn verify_records(
    records: &[ModerationRecord],
) -> Result<VerificationResult, VerificationError> {
    verify_records_with_expected_length(records, None)
}

/// Verify records and optionally enforce an expected chain length.
pub fn verify_records_with_expected_length(
    records: &[ModerationRecord],
    expected_length: Option<usize>,
) -> Result<VerificationResult, VerificationError> {
    let mut errors = Vec::new();
    let mut expected_prev_hash = GENESIS_PREV_HASH.to_string();
    let mut expected_sequence: u64 = 1;

    for (index, record) in records.iter().enumerate() {
        if record.sequence != expected_sequence {
            errors.push(VerificationError {
                index,
                sequence: record.sequence,
                kind: VerificationErrorKind::UnexpectedSequence {
                    expected: expected_sequence,
                    actual: record.sequence,
                },
            });
        }
        expected_sequence = record.sequence.wrapping_add(1);

        if record.prev_hash != expected_prev_hash {
            errors.push(VerificationError {
                index,
                sequence: record.sequence,
                kind: VerificationErrorKind::BrokenPrevLink {
                    expected: expected_prev_hash.clone(),
                    actual: record.prev_hash.clone(),
                },
            });
        }

        // Privileged records must carry the step-up proof timestamp.
        if PrivilegedAction::is_privileged(&record.action) && record.proof_timestamp_unix_ms.is_none()
        {
            errors.push(VerificationError {
                index,
                sequence: record.sequence,
                kind: VerificationErrorKind::MissingStepUpProof {
                    action: record.action.clone(),
                },
            });
        }

        match compute_record_hash(record) {
            Ok(computed) => {
                if computed != record.record_hash {
                    errors.push(VerificationError {
                        index,
                        sequence: record.sequence,
                        kind: VerificationErrorKind::HashMismatch {
                            expected: computed.clone(),
                            actual: record.record_hash.clone(),
                        },
                    });
                }
                expected_prev_hash = record.record_hash.clone();
            }
            Err(kind) => {
                errors.push(VerificationError {
                    index,
                    sequence: record.sequence,
                    kind,
                });
            }
        }

        if let Some(target) = record.corrects_sequence {
            if !records[..index].iter().any(|r| r.sequence == target) {
                errors.push(VerificationError {
                    index,
                    sequence: record.sequence,
                    kind: VerificationErrorKind::DanglingCorrection {
                        target_sequence: target,
                    },
                });
            }
        }
    }

    if let Some(expected) = expected_length {
        if records.len() != expected {
            errors.push(VerificationError {
                index: records.len(),
                sequence: records.last().map(|r| r.sequence).unwrap_or(0),
                kind: VerificationErrorKind::UnexpectedSequence {
                    expected: expected as u64,
                    actual: records.len() as u64,
                },
            });
        }
    }

    let ok = errors.is_empty();
    let result = VerificationResult {
        ok,
        checked_records: records.len(),
        errors: errors.clone(),
    };
    if let Some(first) = errors.into_iter().next() {
        return Err(first);
    }
    Ok(result)
}

fn load_records(path: &PathBuf) -> Result<Vec<ModerationRecord>, ModerationAuditError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let record: ModerationRecord =
            serde_json::from_str(&line).map_err(|e| ModerationAuditError::CorruptRecord {
                index,
                detail: e.to_string(),
            })?;
        records.push(record);
    }
    Ok(records)
}

fn append_record_to_disk(path: &PathBuf, record: &ModerationRecord) -> Result<(), ModerationAuditError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let line = serde_json::to_string(record)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_data()?;
    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accept_all(_: &StepUpProof) -> bool {
        true
    }

    fn proof(actor: &str, action: &str, target: &str, nonce: &str, ts: i64) -> StepUpProof {
        StepUpProof {
            actor: actor.into(),
            action: action.into(),
            target: target.into(),
            nonce: nonce.into(),
            proof_timestamp_unix_ms: ts,
            signature: "sig".into(),
        }
    }

    #[test]
    fn privileged_action_requires_proof() {
        let mut v = StepUpVerifier::new(accept_all);
        assert_eq!(
            v.verify("alice", "hide_content", "post1", None, 1000),
            Err(StepUpError::MissingProof)
        );
    }

    #[test]
    fn non_privileged_action_needs_no_proof() {
        let mut v = StepUpVerifier::new(accept_all);
        assert!(v.verify("alice", "comment", "post1", None, 1000).is_ok());
    }

    #[test]
    fn stale_proof_rejected() {
        let mut v = StepUpVerifier::new(accept_all);
        let p = proof("alice", "hide_content", "post1", "n1", 0);
        let now = PROOF_MAX_AGE_MS + 1;
        assert!(matches!(
            v.verify("alice", "hide_content", "post1", Some(&p), now),
            Err(StepUpError::StaleProof { .. })
        ));
    }

    #[test]
    fn future_proof_rejected() {
        let mut v = StepUpVerifier::new(accept_all);
        let p = proof("alice", "hide_content", "post1", "n1", 5000);
        assert_eq!(
            v.verify("alice", "hide_content", "post1", Some(&p), 1000),
            Err(StepUpError::FutureProof)
        );
    }

    #[test]
    fn action_and_target_bound() {
        let mut v = StepUpVerifier::new(accept_all);
        let p = proof("alice", "hide_content", "post1", "n1", 1000);
        assert_eq!(
            v.verify("alice", "export_reports", "post1", Some(&p), 1000),
            Err(StepUpError::ActionMismatch)
        );
        assert_eq!(
            v.verify("alice", "hide_content", "post2", Some(&p), 1000),
            Err(StepUpError::TargetMismatch)
        );
        assert_eq!(
            v.verify("bob", "hide_content", "post1", Some(&p), 1000),
            Err(StepUpError::ActorMismatch)
        );
    }

    #[test]
    fn replayed_nonce_rejected() {
        let mut v = StepUpVerifier::new(accept_all);
        let p = proof("alice", "hide_content", "post1", "n1", 1000);
        assert!(v.verify("alice", "hide_content", "post1", Some(&p), 1000).is_ok());
        assert_eq!(
            v.verify("alice", "hide_content", "post1", Some(&p), 1000),
            Err(StepUpError::ReplayedNonce)
        );
    }

    #[test]
    fn invalid_signature_rejected() {
        let mut v = StepUpVerifier::new(|_| false);
        let p = proof("alice", "hide_content", "post1", "n1", 1000);
        assert_eq!(
            v.verify("alice", "hide_content", "post1", Some(&p), 1000),
            Err(StepUpError::InvalidSignature)
        );
    }

    #[test]
    fn privileged_record_without_proof_timestamp_rejected() {
        let path = std::env::temp_dir().join(format!("audit-noproof-{}.jsonl", std::process::id()));
        let _ = fs::remove_file(&path);
        let mut log = ModerationAuditLog::open(path.to_string_lossy()).unwrap();
        assert!(log
            .append("alice", "hide_content", "post1", "spam", 1000, None)
            .is_err());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn audit_record_captures_actor_reason_and_proof_timestamp() {
        let path = std::env::temp_dir().join(format!("audit-ok-{}.jsonl", std::process::id()));
        let _ = fs::remove_file(&path);
        let mut log = ModerationAuditLog::open(path.to_string_lossy()).unwrap();
        let rec = log
            .append("alice", "hide_content", "post1", "spam", 1000, Some(990))
            .unwrap();
        assert_eq!(rec.actor, "alice");
        assert_eq!(rec.reason, "spam");
        assert_eq!(rec.proof_timestamp_unix_ms, Some(990));
        let _ = fs::remove_file(&path);
    }

    fn temp_log(tag: &str) -> (ModerationAuditLog, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "audit-{tag}-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_file(&path);
        let log = ModerationAuditLog::open(path.to_string_lossy()).unwrap();
        (log, path)
    }

    #[test]
    fn legal_transitions_are_defined() {
        use ModerationState::*;
        assert!(ModerationState::is_legal_transition(ReportSubmitted, UnderReview));
        assert!(ModerationState::is_legal_transition(UnderReview, ActionTaken));
        assert!(ModerationState::is_legal_transition(ActionTaken, AppealSubmitted));
        assert!(ModerationState::is_legal_transition(AppealSubmitted, AppealUpheld));
        assert!(ModerationState::is_legal_transition(AppealSubmitted, AppealRejected));
        assert!(!ModerationState::is_legal_transition(ReportSubmitted, ActionTaken));
        assert!(!ModerationState::is_legal_transition(Dismissed, UnderReview));
        assert!(Dismissed.is_terminal());
    }

    #[test]
    fn forbidden_transition_rejected() {
        let (mut log, path) = temp_log("forbidden");
        log.submit_report("r1", "alice", "spam", 1000).unwrap();
        let err = log
            .transition(
                "r1",
                "bob",
                ReviewerRole::SeniorReviewer,
                ModerationState::ActionTaken,
                "skip review",
                2000,
                None,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            ModerationTransitionError::ForbiddenTransition { .. }
        ));
        // No transition record was appended for the forbidden attempt.
        assert_eq!(log.history("r1").len(), 1);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn permission_enforced_in_service_layer() {
        let (mut log, path) = temp_log("perm");
        log.submit_report("r1", "alice", "spam", 1000).unwrap();
        log.transition(
            "r1",
            "bob",
            ReviewerRole::Reviewer,
            ModerationState::UnderReview,
            "claim",
            2000,
            None,
        )
        .unwrap();
        // A plain reviewer may not take action.
        let err = log
            .transition(
                "r1",
                "bob",
                ReviewerRole::Reviewer,
                ModerationState::ActionTaken,
                "hide",
                3000,
                None,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            ModerationTransitionError::PermissionDenied { .. }
        ));
        assert_eq!(log.history("r1").len(), 2);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn duplicate_submission_is_idempotent() {
        let (mut log, path) = temp_log("dup");
        log.submit_report("r1", "alice", "spam", 1000).unwrap();
        let err = log.submit_report("r1", "alice", "spam", 1001).unwrap_err();
        assert!(matches!(
            err,
            ModerationTransitionError::DuplicateTransition { .. }
        ));
        assert_eq!(log.history("r1").len(), 1);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn audit_history_complete_after_retries() {
        let (mut log, path) = temp_log("retry");
        log.submit_report("r1", "alice", "spam", 1000).unwrap();
        // Retry the same submission; must not duplicate.
        let _ = log.submit_report("r1", "alice", "spam", 1001);
        log.transition(
            "r1",
            "bob",
            ReviewerRole::Reviewer,
            ModerationState::UnderReview,
            "claim",
            2000,
            None,
        )
        .unwrap();
        // Retry the same transition; must not duplicate.
        let _ = log.transition(
            "r1",
            "bob",
            ReviewerRole::Reviewer,
            ModerationState::UnderReview,
            "claim",
            2001,
            None,
        );
        log.transition(
            "r1",
            "carol",
            ReviewerRole::SeniorReviewer,
            ModerationState::ActionTaken,
            "hide",
            3000,
            None,
        )
        .unwrap();

        let history = log.history("r1");
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].to_state, Some(ModerationState::ReportSubmitted));
        assert_eq!(history[1].from_state, Some(ModerationState::ReportSubmitted));
        assert_eq!(history[1].to_state, Some(ModerationState::UnderReview));
        assert_eq!(history[2].to_state, Some(ModerationState::ActionTaken));
        // Actor and timestamp recorded per transition.
        assert_eq!(history[1].actor, "bob");
        assert_eq!(history[1].timestamp_unix_ms, 2000);
        assert_eq!(history[2].actor, "carol");
        assert_eq!(history[2].timestamp_unix_ms, 3000);
        assert_eq!(log.current_state("r1"), Some(ModerationState::ActionTaken));
        // Chain remains valid after retries.
        assert!(log.verify().unwrap().ok);
        let _ = fs::remove_file(&path);
    }
}
