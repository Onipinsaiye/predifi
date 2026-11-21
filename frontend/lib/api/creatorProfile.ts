/**
 * Creator profile API client (Issue #278).
 *
 * Thin, typed wrapper around the creator-profile endpoints:
 *   GET  /api/v1/users/:address/creator-profile
 *   PUT  /api/v1/users/:address/creator-profile
 *
 * Profiles are opt-in and private by default. An absent row means the creator
 * has not opted in; the UI must treat that as "no profile" rather than an
 * error.
 *
 * Provenance links start with `verified = false` and are confirmed
 * asynchronously by the backend verification job. The UI shows a
 * "pending verification" badge until `verified` becomes true.
 */

import { API_BASE_URL, ApiError, fetchWithTimeout } from "@/lib/api/pools";

// ─── Types ────────────────────────────────────────────────────────────────────

export type ProvenancePlatform =
  | "twitter"
  | "github"
  | "farcaster"
  | "lens"
  | "url";

export interface ProvenanceLink {
  platform: ProvenancePlatform;
  /** Platform handle (e.g. "@alice") or full HTTPS URL for the "url" platform. */
  handle_or_url: string;
  /**
   * Whether the backend verification job has confirmed this link.
   * Always `false` immediately after a save; flips to `true` once verified.
   */
  verified: boolean;
}

/** Full creator profile as returned by the API. */
export interface CreatorProfile {
  address: string;
  display_name: string | null;
  avatar_url: string | null;
  bio: string | null;
  provenance_links: ProvenanceLink[];
  /** True when the profile is publicly visible. */
  public: boolean;
  opted_in_at: string;
  updated_at: string;
}

/** Payload for PUT /api/v1/users/:address/creator-profile */
export interface UpsertCreatorProfileInput {
  display_name: string | null;
  avatar_url: string | null;
  bio: string | null;
  /** Backend resets all verified flags to false on save. */
  provenance_links: Omit<ProvenanceLink, "verified">[];
  public: boolean;
}

// ─── URL helpers ──────────────────────────────────────────────────────────────

export function creatorProfileUrl(address: string): string {
  return `${API_BASE_URL}/api/v1/users/${encodeURIComponent(address)}/creator-profile`;
}

// ─── Type guards ─────────────────────────────────────────────────────────────

function isProvenanceLink(obj: unknown): obj is ProvenanceLink {
  if (!obj || typeof obj !== "object") return false;
  const link = obj as Record<string, unknown>;
  return (
    typeof link.platform === "string" &&
    typeof link.handle_or_url === "string" &&
    typeof link.verified === "boolean"
  );
}

function isCreatorProfile(obj: unknown): obj is CreatorProfile {
  if (!obj || typeof obj !== "object") return false;
  const profile = obj as Record<string, unknown>;
  return (
    typeof profile.address === "string" &&
    (profile.display_name === null || typeof profile.display_name === "string") &&
    (profile.avatar_url === null || typeof profile.avatar_url === "string") &&
    (profile.bio === null || typeof profile.bio === "string") &&
    Array.isArray(profile.provenance_links) &&
    profile.provenance_links.every(isProvenanceLink) &&
    typeof profile.public === "boolean" &&
    typeof profile.opted_in_at === "string" &&
    typeof profile.updated_at === "string"
  );
}

// ─── Fetch ────────────────────────────────────────────────────────────────────

/**
 * Fetch the creator profile for `address`.
 *
 * Returns `null` when the creator has not opted in or the profile is not
 * publicly visible (404). Throws `ApiError` for other non-2xx responses.
 *
 * Pass `bearerToken` to authenticate as the owner — the backend will then
 * return private profiles as well.
 */
export async function fetchCreatorProfile(
  address: string,
  bearerToken?: string,
): Promise<CreatorProfile | null> {
  const headers: Record<string, string> = { Accept: "application/json" };
  if (bearerToken) {
    headers["Authorization"] = `Bearer ${bearerToken}`;
  }

  const res = await fetchWithTimeout(creatorProfileUrl(address), { headers });

  if (res.status === 404) return null;

  if (!res.ok) {
    throw new ApiError(
      `Failed to fetch creator profile (HTTP ${res.status})`,
      res.status,
    );
  }

  const body = await res.json();
  const payload =
    body && typeof body === "object" && "data" in body && body.data
      ? body.data
      : body;

  if (!isCreatorProfile(payload)) {
    throw new ApiError("Invalid creator profile response shape", 500);
  }

  return payload;
}

// ─── Upsert ───────────────────────────────────────────────────────────────────

/**
 * Create or update the creator profile for `address`.
 *
 * Requires a valid bearer token for `address`. Provenance-link `verified`
 * flags are always reset to `false` by the backend; pass the links without
 * `verified` to avoid confusion.
 */
export async function upsertCreatorProfile(
  address: string,
  bearerToken: string,
  input: UpsertCreatorProfileInput,
): Promise<CreatorProfile> {
  const payload: UpsertCreatorProfileInput & {
    provenance_links: ProvenanceLink[];
  } = {
    ...input,
    // Explicit: strip any stale verified flags before sending.
    provenance_links: input.provenance_links.map((l) => ({
      ...l,
      verified: false,
    })),
  };

  const res = await fetchWithTimeout(creatorProfileUrl(address), {
    method: "PUT",
    headers: {
      Accept: "application/json",
      "Content-Type": "application/json",
      Authorization: `Bearer ${bearerToken}`,
    },
    body: JSON.stringify(payload),
  });

  if (!res.ok) {
    const errorBody = await res.json().catch(() => ({}));
    const message =
      (errorBody as Record<string, unknown>)?.error ??
      `Failed to save creator profile (HTTP ${res.status})`;
    throw new ApiError(String(message), res.status);
  }

  const body = await res.json();
  const data =
    body && typeof body === "object" && "data" in body && body.data
      ? body.data
      : body;

  if (!isCreatorProfile(data)) {
    throw new ApiError("Invalid creator profile response shape after upsert", 500);
  }

  return data;
}

// ─── Constants for validation (mirrors backend) ───────────────────────────────

export const CREATOR_PROFILE_MAX_DISPLAY_NAME = 64;
export const CREATOR_PROFILE_MAX_BIO = 280;
export const CREATOR_PROFILE_MAX_AVATAR_URL = 512;
export const CREATOR_PROFILE_MAX_PROVENANCE_LINKS = 10;
export const CREATOR_PROFILE_MAX_HANDLE_OR_URL = 256;

export const PROVENANCE_PLATFORMS: ProvenancePlatform[] = [
  "twitter",
  "github",
  "farcaster",
  "lens",
  "url",
];
