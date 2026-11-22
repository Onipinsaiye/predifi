/**
 * User profile API client.
 *
 * Thin, typed wrapper around `GET /api/v1/users/:address/profile` — the
 * aggregated payload behind the profile page (prediction history, win/loss
 * stats, total earnings, active positions, and claim status).
 *
 * Data ownership (see docs/data-ownership-matrix.md):
 * - `stats.*` (balances, lifecycle, analytics) are owned by the indexer and
 *   served by the API. The UI must treat them as read-only projections and
 *   never recompute them client-side.
 * - `claims[].claimed` / `claimed_amount` are owned by the chain (claim
 *   registry). The API mirrors them; the UI must not optimistically flip
 *   `claimed` without a refetch.
 * - `claims[].pool_state` / `pool_result` are owned by the chain (pool
 *   contract) and mirrored by the indexer.
 * - `performance[]` is an indexer-owned analytics rollup; it is advisory and
 *   may lag behind `stats`.
 *
 * Freshness / fallback contract:
 * - The API is the single source of truth for the profile page. There is no
 *   client-side fallback to chain reads; on failure the UI surfaces an error
 *   state rather than stale or synthesized values.
 * - Malformed payloads are rejected at the boundary (see type guards) so the
 *   UI never renders partially-valid data.
 */

import { API_BASE_URL, ApiError } from "@/lib/api/pools";

export interface ProfileStats {
  total_predictions: number;
  wins: number;
  losses: number;
  pending: number;
  /** Win rate as a percentage (0-100) of settled predictions. */
  win_rate: number;
  total_staked: number;
  total_earnings: number;
  active_positions: number;
}

export interface ClaimStatus {
  prediction_id: number;
  pool_id: number;
  pool_name: string;
  outcome: number;
  amount: number;
  pool_state: string;
  pool_result: string | null;
  is_winner: boolean | null;
  claimed: boolean;
  claimed_amount: number;
  claim_window_expires_at: string | null;
  claim_expired: boolean;
}

export interface PerformancePoint {
  day: string;
  staked: number;
  earnings: number;
  predictions: number;
}

export interface UserProfile {
  address: string;
  stats: ProfileStats;
  claims: ClaimStatus[];
  performance: PerformancePoint[];
}

/** Type guard to validate ProfileStats shape. */
function isProfileStats(obj: unknown): obj is ProfileStats {
  if (!obj || typeof obj !== "object") return false;
  const stats = obj as Record<string, unknown>;
  return (
    typeof stats.total_predictions === "number" &&
    typeof stats.wins === "number" &&
    typeof stats.losses === "number" &&
    typeof stats.pending === "number" &&
    typeof stats.win_rate === "number" &&
    typeof stats.total_staked === "number" &&
    typeof stats.total_earnings === "number" &&
    typeof stats.active_positions === "number"
  );
}

/** Type guard to validate ClaimStatus shape. */
function isClaimStatus(obj: unknown): obj is ClaimStatus {
  if (!obj || typeof obj !== "object") return false;
  const claim = obj as Record<string, unknown>;
  return (
    typeof claim.prediction_id === "number" &&
    typeof claim.pool_id === "number" &&
    typeof claim.pool_name === "string" &&
    typeof claim.outcome === "number" &&
    typeof claim.amount === "number" &&
    typeof claim.pool_state === "string" &&
    (claim.pool_result === null || typeof claim.pool_result === "string") &&
    (claim.is_winner === null || typeof claim.is_winner === "boolean") &&
    typeof claim.claimed === "boolean" &&
    typeof claim.claimed_amount === "number" &&
    (claim.claim_window_expires_at === null || typeof claim.claim_window_expires_at === "string") &&
    typeof claim.claim_expired === "boolean"
  );
}

/** Type guard to validate PerformancePoint shape. */
function isPerformancePoint(obj: unknown): obj is PerformancePoint {
  if (!obj || typeof obj !== "object") return false;
  const point = obj as Record<string, unknown>;
  return (
    typeof point.day === "string" &&
    typeof point.staked === "number" &&
    typeof point.earnings === "number" &&
    typeof point.predictions === "number"
  );
}

/** Type guard to validate UserProfile shape. */
function isUserProfile(obj: unknown): obj is UserProfile {
  if (!obj || typeof obj !== "object") return false;
  const profile = obj as Record<string, unknown>;
  return (
    typeof profile.address === "string" &&
    isProfileStats(profile.stats) &&
    Array.isArray(profile.claims) &&
    profile.claims.every(isClaimStatus) &&
    Array.isArray(profile.performance) &&
    profile.performance.every(isPerformancePoint)
  );
}

export function profileUrl(address: string): string {
  return `${API_BASE_URL}/api/v1/users/${encodeURIComponent(address)}/profile`;
}

/**
 * Versioned JSON export of a complete portfolio record.
 *
 * Unlike CSV, this format preserves nested market and event provenance as
 * well as the calculation metadata used to derive the exported figures.
 * The payload is versioned via `schema_version` so consumers can branch on
 * the shape they receive.
 */
export const PORTFOLIO_EXPORT_SCHEMA_VERSION = "portfolio.export.v1" as const;

export interface PortfolioExportSourceRef {
  /** Logical source of the datum, e.g. "indexer", "chain", "pool_contract". */
  source: string;
  /** Identifier of the record within that source (tx hash, pool id, ...). */
  reference: string;
  /** Optional URL to the canonical record. */
  url?: string | null;
}

export interface PortfolioExportCalculation {
  /** Name of the derived field, e.g. "total_earnings". */
  field: string;
  /** Formula or method used to derive the value. */
  method: string;
  /** Inputs the calculation consumed. */
  inputs?: Record<string, unknown>;
}

export interface PortfolioExportPosition {
  prediction_id: number;
  pool_id: number;
  pool_name: string;
  outcome: number;
  amount: number;
  pool_state: string;
  pool_result: string | null;
  is_winner: boolean | null;
  claimed: boolean;
  claimed_amount: number;
  sources: PortfolioExportSourceRef[];
}

export interface PortfolioExportLedgerEntry {
  id: string;
  prediction_id: number;
  pool_id: number;
  kind: string;
  amount: number;
  timestamp: string;
  sources: PortfolioExportSourceRef[];
}

export interface PortfolioExportTruncation {
  /** True when the export was bounded and some records were omitted. */
  truncated: boolean;
  /** Opaque cursor to request the next page, or null when complete. */
  continuation: string | null;
  /** Number of records included in this export. */
  returned: number;
  /** Total number of records available, when known. */
  total: number | null;
}

export interface PortfolioExport {
  schema_version: typeof PORTFOLIO_EXPORT_SCHEMA_VERSION;
  address: string;
  generated_at: string;
  positions: PortfolioExportPosition[];
  ledger: PortfolioExportLedgerEntry[];
  calculations: PortfolioExportCalculation[];
  truncation: PortfolioExportTruncation;
}

function isSourceRef(obj: unknown): obj is PortfolioExportSourceRef {
  if (!obj || typeof obj !== "object") return false;
  const ref = obj as Record<string, unknown>;
  return (
    typeof ref.source === "string" &&
    typeof ref.reference === "string" &&
    (ref.url === undefined || ref.url === null || typeof ref.url === "string")
  );
}

function isCalculation(obj: unknown): obj is PortfolioExportCalculation {
  if (!obj || typeof obj !== "object") return false;
  const calc = obj as Record<string, unknown>;
  return (
    typeof calc.field === "string" &&
    typeof calc.method === "string" &&
    (calc.inputs === undefined ||
      calc.inputs === null ||
      (typeof calc.inputs === "object" && !Array.isArray(calc.inputs)))
  );
}

function isExportPosition(obj: unknown): obj is PortfolioExportPosition {
  if (!obj || typeof obj !== "object") return false;
  const pos = obj as Record<string, unknown>;
  return (
    typeof pos.prediction_id === "number" &&
    typeof pos.pool_id === "number" &&
    typeof pos.pool_name === "string" &&
    typeof pos.outcome === "number" &&
    typeof pos.amount === "number" &&
    typeof pos.pool_state === "string" &&
    (pos.pool_result === null || typeof pos.pool_result === "string") &&
    (pos.is_winner === null || typeof pos.is_winner === "boolean") &&
    typeof pos.claimed === "boolean" &&
    typeof pos.claimed_amount === "number" &&
    Array.isArray(pos.sources) &&
    pos.sources.every(isSourceRef)
  );
}

function isExportLedgerEntry(obj: unknown): obj is PortfolioExportLedgerEntry {
  if (!obj || typeof obj !== "object") return false;
  const entry = obj as Record<string, unknown>;
  return (
    typeof entry.id === "string" &&
    typeof entry.prediction_id === "number" &&
    typeof entry.pool_id === "number" &&
    typeof entry.kind === "string" &&
    typeof entry.amount === "number" &&
    typeof entry.timestamp === "string" &&
    Array.isArray(entry.sources) &&
    entry.sources.every(isSourceRef)
  );
}

function isTruncation(obj: unknown): obj is PortfolioExportTruncation {
  if (!obj || typeof obj !== "object") return false;
  const trunc = obj as Record<string, unknown>;
  return (
    typeof trunc.truncated === "boolean" &&
    (trunc.continuation === null || typeof trunc.continuation === "string") &&
    typeof trunc.returned === "number" &&
    (trunc.total === null || typeof trunc.total === "number")
  );
}

/** Type guard validating the `portfolio.export.v1` schema. */
export function isPortfolioExport(obj: unknown): obj is PortfolioExport {
  if (!obj || typeof obj !== "object") return false;
  const exp = obj as Record<string, unknown>;
  return (
    exp.schema_version === PORTFOLIO_EXPORT_SCHEMA_VERSION &&
    typeof exp.address === "string" &&
    typeof exp.generated_at === "string" &&
    Array.isArray(exp.positions) &&
    exp.positions.every(isExportPosition) &&
    Array.isArray(exp.ledger) &&
    exp.ledger.every(isExportLedgerEntry) &&
    Array.isArray(exp.calculations) &&
    exp.calculations.every(isCalculation) &&
    isTruncation(exp.truncation)
  );
}

export interface FetchPortfolioExportOptions {
  /** Opaque continuation cursor from a previous truncated export. */
  cursor?: string | null;
  /** Maximum number of records to request; the server may bound this. */
  limit?: number;
}

export function portfolioExportUrl(address: string, options: FetchPortfolioExportOptions = {}): string {
  const params = new URLSearchParams();
  if (options.cursor) params.set("cursor", options.cursor);
  if (typeof options.limit === "number") params.set("limit", String(options.limit));
  const query = params.toString();
  return `${API_BASE_URL}/api/v1/users/${encodeURIComponent(address)}/portfolio/export${query ? `?${query}` : ""}`;
}

/**
 * Fetch the versioned JSON portfolio export.
 *
 * The returned payload always carries `truncation` metadata so callers can
 * surface "showing N of M" messaging and follow `continuation` for the next
 * page. Malformed payloads are rejected at the boundary.
 */
export async function fetchPortfolioExport(
  address: string,
  options: FetchPortfolioExportOptions = {},
): Promise<PortfolioExport> {
  const res = await fetch(portfolioExportUrl(address, options), {
    headers: { Accept: "application/json" },
  });

  if (!res.ok) {
    throw new ApiError(`Failed to export portfolio (HTTP ${res.status})`, res.status);
  }

  const body = await res.json();
  const payload =
    body && typeof body === "object" && "data" in body && body.data ? body.data : body;

  if (!isPortfolioExport(payload)) {
    throw new ApiError("Invalid portfolio export response shape", 500);
  }

  return payload;
}

/**
 * Trigger a browser download of the JSON portfolio export.
 *
 * Returns the parsed export so callers can also surface truncation metadata
 * in the UI after the download starts.
 */
export async function downloadPortfolioExport(
  address: string,
  options: FetchPortfolioExportOptions = {},
): Promise<PortfolioExport> {
  const payload = await fetchPortfolioExport(address, options);
  const blob = new Blob([JSON.stringify(payload, null, 2)], {
    type: "application/json",
  });
  const url = URL.createObjectURL(blob);
  try {
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `portfolio-${address}-${payload.generated_at}.json`;
    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
  } finally {
    URL.revokeObjectURL(url);
  }
  return payload;
}

/**
 * Portfolio exposure by canonical market and outcome (#261).
 *
 * Exposure rows aggregate *open* positions by (market, outcome). Each row
 * carries both the absolute amount and its share of the indexed portfolio so
 * the UI can surface concentration. `estimated` distinguishes indexer-derived
 * estimates from `claimable` funds that the chain has already settled.
 */
export interface PortfolioExposureRow {
  /** Canonical market identifier (pool id as string, stable across renames). */
  market_id: string;
  market_name: string;
  /** Canonical outcome index within the market. */
  outcome: number;
  outcome_label: string;
  /** Absolute open amount for this (market, outcome). */
  amount: number;
  /** Share of the indexed portfolio in the range 0-1. */
  share: number;
  /** True when the amount is an indexer estimate rather than settled funds. */
  estimated: boolean;
  /** Portion of `amount` that is claimable (settled, unclaimed). */
  claimable: number;
  /** Number of open positions aggregated into this row. */
  position_count: number;
}

export interface PortfolioExposure {
  address: string;
  generated_at: string;
  /** Total indexed portfolio value the shares are computed against. */
  total: number;
  rows: PortfolioExposureRow[];
  /** True when some markets were skipped due to missing price data. */
  incomplete: boolean;
  /** Market ids omitted from the rollup (e.g. missing price data). */
  missing_markets: string[];
}

function isExposureRow(obj: unknown): obj is PortfolioExposureRow {
  if (!obj || typeof obj !== "object") return false;
  const row = obj as Record<string, unknown>;
  return (
    typeof row.market_id === "string" &&
    typeof row.market_name === "string" &&
    typeof row.outcome === "number" &&
    typeof row.outcome_label === "string" &&
    typeof row.amount === "number" &&
    typeof row.share === "number" &&
    typeof row.estimated === "boolean" &&
    typeof row.claimable === "number" &&
    typeof row.position_count === "number"
  );
}

export function isPortfolioExposure(obj: unknown): obj is PortfolioExposure {
  if (!obj || typeof obj !== "object") return false;
  const exp = obj as Record<string, unknown>;
  return (
    typeof exp.address === "string" &&
    typeof exp.generated_at === "string" &&
    typeof exp.total === "number" &&
    Array.isArray(exp.rows) &&
    exp.rows.every(isExposureRow) &&
    typeof exp.incomplete === "boolean" &&
    Array.isArray(exp.missing_markets) &&
    exp.missing_markets.every((m) => typeof m === "string")
  );
}

export function portfolioExposureUrl(address: string): string {
  return `${API_BASE_URL}/api/v1/users/${encodeURIComponent(address)}/portfolio/exposure`;
}

export async function fetchPortfolioExposure(address: string): Promise<PortfolioExposure> {
  const res = await fetch(portfolioExposureUrl(address), {
    headers: { Accept: "application/json" },
  });

  if (!res.ok) {
    throw new ApiError(`Failed to load portfolio exposure (HTTP ${res.status})`, res.status);
  }

  const body = await res.json();
  const payload =
    body && typeof body === "object" && "data" in body && body.data ? body.data : body;

  if (!isPortfolioExposure(payload)) {
    throw new ApiError("Invalid portfolio exposure response shape", 500);
  }

  return payload;
}

/**
 * Claimable-rewards work queue (#262).
 *
 * Each entry is an eligible claim with enough context to audit and prioritise
 * the workflow. `status` is chain/indexer-owned; the UI must not flip it to
 * "claimed" without a refetch after confirmation.
 */
export type ClaimableRewardStatus =
  | "eligible"
  | "ineligible"
  | "simulating"
  | "simulation_failed"
  | "submitted"
  | "claimed"
  | "expired";

export interface ClaimableReward {
  prediction_id: number;
  pool_id: number;
  market_name: string;
  outcome: number;
  outcome_label: string;
  /** Estimated payout amount, in the portfolio's asset units. */
  estimated_amount: number;
  status: ClaimableRewardStatus;
  /** Transaction hash once submitted/claimed, else null. */
  transaction_hash: string | null;
  /** Explorer link for the claim transaction, when available. */
  transaction_url: string | null;
  /** Reason a simulation failed, when `status` is "simulation_failed". */
  failure_reason: string | null;
  claim_window_expires_at: string | null;
}

export interface ClaimableRewards {
  address: string;
  generated_at: string;
  entries: ClaimableReward[];
  /** Opaque cursor to request the next page, or null when complete. */
  continuation: string | null;
}

function isClaimableReward(obj: unknown): obj is ClaimableReward {
  if (!obj || typeof obj !== "object") return false;
  const entry = obj as Record<string, unknown>;
  const statuses: ClaimableRewardStatus[] = [
    "eligible",
    "ineligible",
    "simulating",
    "simulation_failed",
    "submitted",
    "claimed",
    "expired",
  ];
  return (
    typeof entry.prediction_id === "number" &&
    typeof entry.pool_id === "number" &&
    typeof entry.market_name === "string" &&
    typeof entry.outcome === "number" &&
    typeof entry.outcome_label === "string" &&
    typeof entry.estimated_amount === "number" &&
    typeof entry.status === "string" &&
    statuses.includes(entry.status as ClaimableRewardStatus) &&
    (entry.transaction_hash === null || typeof entry.transaction_hash === "string") &&
    (entry.transaction_url === null || typeof entry.transaction_url === "string") &&
    (entry.failure_reason === null || typeof entry.failure_reason === "string") &&
    (entry.claim_window_expires_at === null ||
      typeof entry.claim_window_expires_at === "string")
  );
}

export function isClaimableRewards(obj: unknown): obj is ClaimableRewards {
  if (!obj || typeof obj !== "object") return false;
  const rewards = obj as Record<string, unknown>;
  return (
    typeof rewards.address === "string" &&
    typeof rewards.generated_at === "string" &&
    Array.isArray(rewards.entries) &&
    rewards.entries.every(isClaimableReward) &&
    (rewards.continuation === null || typeof rewards.continuation === "string")
  );
}

export interface FetchClaimableRewardsOptions {
  cursor?: string | null;
  /** Maximum number of entries to request; the server may bound this. */
  limit?: number;
}

export function claimableRewardsUrl(
  address: string,
  options: FetchClaimableRewardsOptions = {},
): string {
  const params = new URLSearchParams();
  if (options.cursor) params.set("cursor", options.cursor);
  if (typeof options.limit === "number") params.set("limit", String(options.limit));
  const query = params.toString();
  return `${API_BASE_URL}/api/v1/users/${encodeURIComponent(address)}/claims/queue${query ? `?${query}` : ""}`;
}

export async function fetchClaimableRewards(
  address: string,
  options: FetchClaimableRewardsOptions = {},
): Promise<ClaimableRewards> {
  const res = await fetch(claimableRewardsUrl(address, options), {
    headers: { Accept: "application/json" },
  });

  if (!res.ok) {
    throw new ApiError(`Failed to load claimable rewards (HTTP ${res.status})`, res.status);
  }

  const body = await res.json();
  const payload =
    body && typeof body === "object" && "data" in body && body.data ? body.data : body;

  if (!isClaimableRewards(payload)) {
    throw new ApiError("Invalid claimable rewards response shape", 500);
  }

  return payload;
}

/**
 * Result of simulating a bounded claim batch (#262).
 *
 * A simulation failure is reported per prediction and must NOT mark the claim
 * complete; only `succeeded` entries are safe to submit.
 */
export interface ClaimBatchSimulationEntry {
  prediction_id: number;
  success: boolean;
  /** Estimated payout when the simulation succeeded. */
  estimated_amount: number | null;
  /** Failure reason when `success` is false. */
  failure_reason: string | null;
}

export interface ClaimBatchSimulation {
  address: string;
  simulated_at: string;
  entries: ClaimBatchSimulationEntry[];
  /** True when every entry simulated successfully. */
  all_succeeded: boolean;
}

function isClaimBatchSimulationEntry(obj: unknown): obj is ClaimBatchSimulationEntry {
  if (!obj || typeof obj !== "object") return false;
  const entry = obj as Record<string, unknown>;
  return (
    typeof entry.prediction_id === "number" &&
    typeof entry.success === "boolean" &&
    (entry.estimated_amount === null || typeof entry.estimated_amount === "number") &&
    (entry.failure_reason === null || typeof entry.failure_reason === "string")
  );
}

export function isClaimBatchSimulation(obj: unknown): obj is ClaimBatchSimulation {
  if (!obj || typeof obj !== "object") return false;
  const sim = obj as Record<string, unknown>;
  return (
    typeof sim.address === "string" &&
    typeof sim.simulated_at === "string" &&
    Array.isArray(sim.entries) &&
    sim.entries.every(isClaimBatchSimulationEntry) &&
    typeof sim.all_succeeded === "boolean"
  );
}

export function simulateClaimBatchUrl(address: string): string {
  return `${API_BASE_URL}/api/v1/users/${encodeURIComponent(address)}/claims/simulate`;
}

/**
 * Simulate a bounded selection of claims before submitting them.
 *
 * `predictionIds` bounds the batch; the server may further bound it. Failures
 * are returned per entry and never mark a claim complete.
 */
export async function simulateClaimBatch(
  address: string,
  predictionIds: number[],
): Promise<ClaimBatchSimulation> {
  const res = await fetch(simulateClaimBatchUrl(address), {
    method: "POST",
    headers: { Accept: "application/json", "Content-Type": "application/json" },
    body: JSON.stringify({ prediction_ids: predictionIds }),
  });

  if (!res.ok) {
    throw new ApiError(`Failed to simulate claim batch (HTTP ${res.status})`, res.status);
  }

  const body = await res.json();
  const payload =
    body && typeof body === "object" && "data" in body && body.data ? body.data : body;

  if (!isClaimBatchSimulation(payload)) {
    throw new ApiError("Invalid claim batch simulation response shape", 500);
  }

  return payload;
}

/**
 * Reconciliation of a claimed payout to its originating position (#263).
 *
 * `status` is "pending" while indexing is delayed; the UI surfaces this rather
 * than assuming the claim is complete. `source_event_id` is the identity of the
 * settlement event that produced the payout.
 */
export type ClaimReconciliationStatus = "pending" | "reconciled" | "mismatch";

export interface ClaimReconciliation {
  prediction_id: number;
  market_id: string;
  market_name: string;
  outcome: number;
  outcome_label: string;
  /** Claim transaction hash. */
  claim_transaction: string;
  /** Payout amount credited for the claim. */
  payout_amount: number;
  /** Identity of the settlement event that produced the payout. */
  source_event_id: string;
  status: ClaimReconciliationStatus;
  /** When the reconciliation was last evaluated. */
  checked_at: string;
}

export function isClaimReconciliation(obj: unknown): obj is ClaimReconciliation {
  if (!obj || typeof obj !== "object") return false;
  const rec = obj as Record<string, unknown>;
  const statuses: ClaimReconciliationStatus[] = ["pending", "reconciled", "mismatch"];
  return (
    typeof rec.prediction_id === "number" &&
    typeof rec.market_id === "string" &&
    typeof rec.market_name === "string" &&
    typeof rec.outcome === "number" &&
    typeof rec.outcome_label === "string" &&
    typeof rec.claim_transaction === "string" &&
    typeof rec.payout_amount === "number" &&
    typeof rec.source_event_id === "string" &&
    typeof rec.status === "string" &&
    statuses.includes(rec.status as ClaimReconciliationStatus) &&
    typeof rec.checked_at === "string"
  );
}

export function claimReconciliationUrl(address: string, predictionId: number): string {
  return `${API_BASE_URL}/api/v1/users/${encodeURIComponent(address)}/claims/${predictionId}/reconciliation`;
}

export async function fetchClaimReconciliation(
  address: string,
  predictionId: number,
): Promise<ClaimReconciliation> {
  const res = await fetch(claimReconciliationUrl(address, predictionId), {
    headers: { Accept: "application/json" },
  });

  if (!res.ok) {
    throw new ApiError(`Failed to load claim reconciliation (HTTP ${res.status})`, res.status);
  }

  const body = await res.json();
  const payload =
    body && typeof body === "object" && "data" in body && body.data ? body.data : body;

  if (!isClaimReconciliation(payload)) {
    throw new ApiError("Invalid claim reconciliation response shape", 500);
  }

  return payload;
}

/**
 * CSV portfolio activity export (#264).
 *
 * The CSV is generated server-side with a documented schema version, stable
 * column ordering, exact decimal preservation, and formula-injection escaping.
 * Large exports are paginated via `cursor`; the response is returned as text.
 */
export const PORTFOLIO_ACTIVITY_CSV_SCHEMA_VERSION = "portfolio.activity.csv.v1" as const;

export interface FetchPortfolioActivityCsvOptions {
  /** Opaque continuation cursor from a previous truncated export. */
  cursor?: string | null;
  /** Maximum number of rows to request; the server may bound this. */
  limit?: number;
}

export function portfolioActivityCsvUrl(
  address: string,
  options: FetchPortfolioActivityCsvOptions = {},
): string {
  const params = new URLSearchParams();
  params.set("format", "csv");
  params.set("schema_version", PORTFOLIO_ACTIVITY_CSV_SCHEMA_VERSION);
  if (options.cursor) params.set("cursor", options.cursor);
  if (typeof options.limit === "number") params.set("limit", String(options.limit));
  return `${API_BASE_URL}/api/v1/users/${encodeURIComponent(address)}/portfolio/activity?${params.toString()}`;
}

export interface PortfolioActivityCsvPage {
  /** Raw CSV text, including the header row. */
  csv: string;
  /** Continuation cursor parsed from the response, or null when complete. */
  continuation: string | null;
}

/**
 * Fetch one page of the CSV portfolio activity export.
 *
 * Returns the raw CSV text plus the continuation cursor (read from the
 * `X-Continuation` response header) so callers can paginate large exports.
 */
export async function fetchPortfolioActivityCsv(
  address: string,
  options: FetchPortfolioActivityCsvOptions = {},
): Promise<PortfolioActivityCsvPage> {
  const res = await fetch(portfolioActivityCsvUrl(address, options), {
    headers: { Accept: "text/csv" },
  });

  if (!res.ok) {
    throw new ApiError(`Failed to export portfolio activity (HTTP ${res.status})`, res.status);
  }

  const csv = await res.text();
  const continuation = res.headers.get("X-Continuation");
  return { csv, continuation: continuation || null };
}

export async function fetchProfile(url: string): Promise<UserProfile> {
  const res = await fetch(url, { headers: { Accept: "application/json" } });

  if (!res.ok) {
    throw new ApiError(`Failed to load profile (HTTP ${res.status})`, res.status);
  }

  const body = await res.json();

  // Handle both wrapped and unwrapped responses
  const profile =
    body && typeof body === "object" && "data" in body && body.data ? body.data : body;

  // Validate response shape at boundary
  if (!isUserProfile(profile)) {
    throw new ApiError("Invalid profile response shape", 500);
  }

  return profile;
}