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