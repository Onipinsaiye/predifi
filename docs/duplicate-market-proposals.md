# Duplicate Market Proposal Checks

The create-market form asks for the proposed question, event window, outcome
source, and outcome set. Before the proposal is submitted, the API compares
those fields with proposals updated in the previous 90 days.

## API

`POST /api/v1/market-drafts/duplicates` accepts:

```json
{
  "proposal_id": "stable-client-generated-uuid",
  "question": "Will the central bank raise rates in June?",
  "event_window_start": "2026-06-01T00:00:00Z",
  "event_window_end": "2026-06-30T23:59:59Z",
  "source": "https://centralbank.example/policy",
  "outcomes": ["Yes", "No"],
  "distinction_note": null
}
```

The `proposal_id` remains stable as the creator edits and resubmits the same
draft. Candidates identify existing proposals by their stable UUID and are
ranked by score descending, then ID ascending. At most ten candidates are
returned.

## Comparison policy

Candidate eligibility requires all of the following:

- normalized question-token Jaccard similarity of at least 0.60;
- same detected writing script (text is never transliterated);
- event-window overlap of at least 0.50 relative to the longer window;
- normalized source equality; and
- identical normalized outcome sets (outcome order is ignored).

The displayed score is deterministic, weighted 60% question similarity, 20%
event-window overlap, 10% source, and 10% outcome-set match. Comparisons are
bounded to the 5,000 most recently updated proposals within a 90-day window.

## Non-blocking behavior

Candidates are advisory, not a uniqueness constraint. A likely match places the
draft in `draft` status and returns `requires_distinction: true`. The creator
can resubmit the same ID with a non-empty `distinction_note`; the proposal is
then marked `submitted`. A proposal with no candidates is submitted directly.
Valid distinct proposals are never automatically rejected based on score.

## Non-goals

- This check does not prevent submission, create an on-chain market, or replace
  the later human review and resolution process.
- It does not translate or infer semantic equivalence between questions written
  in different languages.
- It does not search historical on-chain pools because they do not carry the
  source, event-window, and outcome metadata needed for a comparable match.
- It does not merge, edit, or cancel an existing proposal.

## Validation

Unit scenarios cover punctuation/whitespace and Unicode normalization,
cross-script multilingual false positives, overlapping windows, source and
outcome differences, deterministic ranking, and distinct proposals remaining
submittable. The database migration is additive and stores draft values,
candidates by stable UUID, and creator-authored distinction notes.
