# RFC: Transparent Contributor Reputation Model

- **RFC number:** 297
- **Status:** Draft
- **Author:** Product / Engineering
- **Created:** 2025-01-01
- **Tracking issue:** #297

## 1. Summary

This RFC specifies a transparent contributor reputation model. The model is
intentionally **descriptive and non-financial**: it summarizes observable
contributor behavior so that it can be inspected, audited, and appealed. It
**must not** be used to gate payouts, credit, token distribution, or any other
financial outcome.

This RFC also establishes a hard gate: **no composite reputation score may be
shipped or exposed via API or UI until the underlying metrics are independently
reviewable** (see §8). Until that gate is satisfied, only raw, per-metric
counts may be displayed internally to reviewers.

## 2. Motivation

A reputation score can be gamed or misread if its inputs and limitations are
hidden. Contributors and reviewers need to understand:

- exactly which observable events feed the score,
- how older events lose influence (decay),
- where Sybil resistance is strong and where it is weak,
- how to contest an incorrect score, and
- what the score is *not* allowed to be used for.

## 3. Scope and non-goals

**In scope:** observable inputs, decay, Sybil-resistance limits, appeals, and
explicit non-financial use.

**Out of scope:** any ranking, payout, credit, or token-allocation logic. This
RFC does not define a final numeric composite; it defines the inputs and the
review gate that must precede any composite.

## 4. Observable inputs

Each input is defined by an exact data source and query semantics. All inputs
are derived from already-persisted platform events; no new tracking is
introduced by this RFC.

| ID | Input | Data source | Query semantics |
|----|-------|-------------|-----------------|
| I1 | Markets created | `markets` table | Count of rows where `creator_wallet = :wallet` and `created_at` within window. Excludes markets later removed for policy violations. |
| I2 | Resolved outcomes authored | `market_resolutions` table | Count of resolutions where `resolver_wallet = :wallet` and `finalized = true`. |
| I3 | Prediction accuracy | `predictions` joined to `market_resolutions` | For each resolved market the wallet participated in, compare the wallet's position to the final outcome. Accuracy = correct / (correct + incorrect). Ties and voided markets excluded. |
| I4 | Dispute participation | `disputes` table | Count of disputes where `participant_wallet = :wallet`, split into `upheld` and `rejected`. |
| I5 | Report accuracy | `reports` table | Count of reports by `:wallet` that were `confirmed` vs `dismissed` by moderation. |
| I6 | Wallet age / activity | `wallets` + event log | Age = `now - first_seen_at`. Activity = count of distinct days with at least one qualifying event. |

**Query semantics notes:**

- All counts are computed over a rolling window (default 365 days) unless a
  metric explicitly states otherwise.
- Every metric must be reproducible from a documented SQL query stored in the
  repository under `metrics/` and versioned alongside this RFC.
- Metrics are computed per wallet and are individually inspectable; no metric
  is hidden behind the composite.

## 5. Decay function

Older events contribute less. Each qualifying event `e` at time `t_e` is
weighted by an exponential decay relative to evaluation time `T`:

```
w(e) = 0.5 ^ ((T - t_e) / H)
```

where `H` is the half-life. Default `H = 180 days`.

A decayed metric is the sum of event weights:

```
M = Σ_e w(e)
```

**Worked formula example.** A wallet has three upheld disputes:

- one 0 days ago → `w = 0.5^(0/180) = 1.000`
- one 180 days ago → `w = 0.5^(180/180) = 0.500`
- one 360 days ago → `w = 0.5^(360/180) = 0.250`

Decayed dispute metric `M = 1.000 + 0.500 + 0.250 = 1.750`.

Per-event weighting is uniform within a metric unless a metric documents a
different weight; any such weight must be listed in the metric's SQL file.

## 6. Sybil-resistance limits

- **Per-wallet caps:** each metric is capped at a documented maximum
  contribution (e.g. I1 capped at 50 markets, I4 capped at 25 disputes) so a
  single prolific wallet cannot dominate.
- **Wallet-age threshold:** wallets younger than 30 days contribute at most
  25% weight to any metric.
- **Activity threshold:** wallets with fewer than 5 distinct active days in the
  window are flagged `low-confidence` and excluded from composite aggregation.
- **Collusion assumptions:** the model assumes independent wallets. It does
  **not** detect coordinated groups. Residual risk: a coordinated cluster of
  aged, active wallets can inflate metrics proportionally to cluster size.
- **Residual risk statement:** Sybil resistance here is *rate-limiting*, not
  *proof-of-personhood*. Scores must be read as "observed behavior of this
  wallet," never as "trustworthiness of this person."

## 7. Appeal process

- **Who can appeal:** any wallet whose score or underlying metric is displayed
  to them, or any reviewer acting on a wallet's behalf.
- **Evidence required:** the specific metric(s) contested, the expected value,
  and supporting references (event IDs, market IDs, dispute IDs).
- **Reviewer roles:** a moderation reviewer performs first-pass triage; a
  second independent reviewer must confirm any score-affecting change.
- **Timelines:** acknowledgement within 5 business days; decision within 20
  business days.
- **Effect on score:** upheld appeals correct the underlying event data, which
  flows through the normal decay computation. Appeals never directly add or
  subtract points; they only correct inputs. Rejected appeals leave the score
  unchanged and are logged.

## 8. Non-financial use and review gate

**Non-financial use (mandatory):** the reputation score **must not** gate,
weight, or influence payouts, credit, lending, token distribution, airdrops, or
any financial entitlement. It is informational only.

**Review gate:** no composite score may be shipped or exposed via API or UI
until all of the following are true:

1. Every metric in §4 has a versioned, independently runnable query in
   `metrics/`.
2. An independent reviewer (not the author of the metric) has reproduced each
   metric's output on a sample of wallets and signed off.
3. The abuse-case review in §9 has been completed and approved.
4. Worked example calculations (§10) for at least one benign and one
   adversarial profile have been reviewed and approved.

Until the gate passes, only raw per-metric counts may be shown to internal
reviewers; no composite is computed for display.

## 9. Abuse-case review

| Adversarial strategy | Description | Mitigation | Residual risk |
|----------------------|-------------|------------|---------------|
| Wash trading | Repeated self-dealing to inflate I3 accuracy | Exclude trades between wallets sharing funding source; cap I3 contribution | Sophisticated actors may evade funding-link detection |
| Sockpuppets | Many low-age wallets boosting each other | Wallet-age threshold (§6), per-wallet caps | Aged sockpuppets remain effective |
| Collusive markets | Groups creating and resolving markets to farm I1/I2 | Exclude markets with concentrated participant sets; require minimum distinct participants | Small closed groups below threshold |
| Report brigading | Mass reports to inflate I5 | Weight by moderation confirmation only; cap I5 | Coordinated accurate-looking reports |

This section must be reviewed and approved before the §8 gate can pass.

## 10. Worked example calculations

### Benign profile

Wallet active for 2 years, 10 markets created (all > 180 days ago), 20
predictions with 15 correct, 2 upheld disputes (one 90 days ago, one 400 days
ago).

- I1 decayed: 10 events, assume average age 300 days →
  `w ≈ 0.5^(300/180) ≈ 0.315` each → `M ≈ 3.15` (below cap of 50).
- I3 accuracy: `15 / 20 = 0.75`.
- I4 decayed: `0.5^(90/180) + 0.5^(400/180) ≈ 0.707 + 0.214 = 0.921`.
- Wallet passes age and activity thresholds → `high-confidence`.

### Adversarial profile

Wallet 10 days old, 40 markets created in 10 days, 30 predictions all correct
on self-created markets, 0 disputes.

- I1 decayed: 40 events at ~5 days → `w ≈ 0.5^(5/180) ≈ 0.981` each →
  `M ≈ 39.2`, but wallet-age threshold applies 25% weight → `≈ 9.8`.
- I3: self-created-market predictions flagged by wash-trading mitigation and
  excluded → accuracy undefined / excluded.
- Wallet flagged `low-confidence` (age < 30 days, < 5 active days) → excluded
  from composite aggregation.

These examples must be reviewed and approved as part of the §8 gate.

## 11. Open questions

- Exact cap values per metric (to be finalized in `metrics/` files).
- Whether `low-confidence` wallets should be shown at all or only to reviewers.
- Funding-source linkage method for wash-trading detection.