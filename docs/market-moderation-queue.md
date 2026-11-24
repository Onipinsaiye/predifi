# Market Content Moderation Review Queue

> Owning package: `backend` (`backend/src/moderation_queue.rs`, integrated with `backend/src/moderation_audit.rs`)  
> Issue: #245

This document describes the domain contract, authorization boundaries, and operational recovery procedures for the reported market content moderation review queue.

## Scope & Authoritative Boundaries

Users can report specific market content surfaces (`ReportedContentKind`):

- `market_question`
- `market_description`
- `localized_description`
- `resolution_source`
- `outcome_label`

The backend moderation queue (`ModerationQueue`) is the authoritative boundary for:

1. **Report validation and aggregation**:
   - Rejects zero `market_id`, blank reporters, control characters, and details longer than `MAX_REPORT_DETAILS_LEN` (1,000 characters).
   - Rejects duplicate reports from the same `(reporter, market_id, content_kind)` while an active (non-`Resolved`) queue item already exists (`QueueError::DuplicateReport`).
   - Aggregates reports from distinct reporters onto the same active `ReviewQueueItem` and recomputes `QueuePriority` (`Low < Normal < High < Urgent`) from both the highest-severity `ReportReason` and total unique report volume.
2. **Deterministic triage ordering**:
   - `list_queue` restricts access to moderator roles (`Moderator`, `SeniorModerator`, `Admin`) and sorts matching items by `priority DESC, created_at_ms ASC, item_id ASC` with bounded pagination (`limit` in `1..=100`).
3. **Lease-backed claiming and optimistic concurrency**:
   - Every mutation requires `expected_revision` and returns `QueueError::RevisionConflict` on stale writes.
   - `claim_item` grants a time-bounded review lease (`DEFAULT_LEASE_DURATION_MS = 900_000`, 15 minutes). Another moderator cannot steal an active lease (`QueueError::AlreadyClaimed`), but can reclaim the item once `now_ms >= lease_expires_at_ms`.
4. **Escalation & step-up authorization**:
   - Escalated items (`QueueStatus::Escalated`) can only be claimed or resolved by `SeniorModerator` or `Admin` (`QueueError::EscalationPrivilegeRequired`).
   - Privileged visibility outcomes (`ModerationOutcome::Hidden` -> `hide_content`, `ModerationOutcome::Restored` -> `unhide_content`) require a fresh, non-replayed `StepUpProof` verified by `StepUpVerifier` and bound to the canonical target `market:<market_id>:<content_kind>` (`audit_target`).
5. **Authoritative visibility & tamper-evident audit logging**:
   - Every resolution appends a record to `ModerationAuditSink` (implemented by `ModerationAuditLog`).
   - Public `ContentVisibility` (`Visible` vs `Hidden`) is mutated **only after** the audit record commits.

## Lifecycle & Explicit Failure / Recovery States

```text
Pending ──(claim)──▶ InReview ──(resolve + audit ok)──▶ Resolved
  │                    │   │                               │
  │                    │   └──(audit sink error)──▶ AuditSyncFailed ──(retry_audit_sync)──▶ Resolved
  │                    │                                   ▲
  └──(escalate)────────┴──(escalate)──▶ Escalated ─────────┴──(reopen_item, Senior/Admin)
```

### Operator Recovery Runbook

- **Stuck `AuditSyncFailed` item**:
  - Cause: The decision and step-up proof validated, but writing to `ModerationAuditLog` failed transiently.
  - Safety guarantee: Public `ContentVisibility` was **not** changed, and the validated `ModerationDecision` plus attempt count and error detail are preserved in `item.pending_audit`.
  - Recovery: Restore audit storage availability, reload the item's latest `revision`, and invoke `retry_audit_sync`. Do **not** submit a duplicate decision or delete the item.
- **Abandoned `InReview` claim**:
  - Cause: A moderator claimed an item and disconnected without releasing it.
  - Recovery: Either wait until `now_ms >= lease_expires_at_ms` and call `claim_item` with the current `revision`, or have an `Admin` call `release_claim`.
- **Appeal after resolution**:
  - Recovery: A `SeniorModerator` or `Admin` calls `reopen_item` with a mandatory reason. This appends a correction entry (`corrects_sequence`) referencing the original audit record and returns the item to `Escalated` without rewriting history.
