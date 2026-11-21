/**
 * Notifications API client.
 *
 * Wraps the `/api/v1/notifications/:address` and `/api/v1/users/:address/interests`
 * endpoints — in-app alerts for pools ending soon, resolutions, expiring claim
 * windows, and new pools matching a user's followed categories/tags.
 */

import { API_BASE_URL, ApiError } from "@/lib/api/pools";

export type NotificationType =
  | "pool_ending_soon"
  | "pool_resolved"
  | "claim_expiring"
  | "new_pool_match";

export interface Notification {
  id: number;
  user_address: string;
  notif_type: NotificationType;
  title: string;
  message: string;
  pool_id: number | null;
  read: boolean;
  created_at: string;
}

export interface NotificationsResponse {
  address: string;
  notifications: Notification[];
  unread_count: number;
  limit: number;
  offset: number;
}

/** Type guard to validate Notification shape. */
function isNotification(obj: unknown): obj is Notification {
  if (!obj || typeof obj !== "object") return false;
  const notif = obj as Record<string, unknown>;
  return (
    typeof notif.id === "number" &&
    typeof notif.user_address === "string" &&
    typeof notif.notif_type === "string" &&
    (["pool_ending_soon", "pool_resolved", "claim_expiring", "new_pool_match"].includes(
      notif.notif_type as string,
    )) &&
    typeof notif.title === "string" &&
    typeof notif.message === "string" &&
    (notif.pool_id === null || typeof notif.pool_id === "number") &&
    typeof notif.read === "boolean" &&
    typeof notif.created_at === "string"
  );
}

/** Type guard to validate NotificationsResponse shape. */
function isNotificationsResponse(obj: unknown): obj is NotificationsResponse {
  if (!obj || typeof obj !== "object") return false;
  const response = obj as Record<string, unknown>;
  return (
    typeof response.address === "string" &&
    Array.isArray(response.notifications) &&
    response.notifications.every(isNotification) &&
    typeof response.unread_count === "number" &&
    typeof response.limit === "number" &&
    typeof response.offset === "number"
  );
}

export function notificationsUrl(address: string, unreadOnly = false): string {
  const params = new URLSearchParams();
  if (unreadOnly) params.set("unread_only", "true");
  const qs = params.toString();
  return `${API_BASE_URL}/api/v1/notifications/${encodeURIComponent(address)}${qs ? `?${qs}` : ""}`;
}

export async function fetchNotifications(url: string): Promise<NotificationsResponse> {
  const res = await fetch(url, { headers: { Accept: "application/json" } });

  if (!res.ok) {
    throw new ApiError(`Failed to load notifications (HTTP ${res.status})`, res.status);
  }

  const body = await res.json();

  // Handle both wrapped and unwrapped responses
  const response =
    body && typeof body === "object" && "data" in body && body.data
      ? body.data
      : body;

  // Validate response shape at boundary
  if (!isNotificationsResponse(response)) {
    throw new ApiError("Invalid notifications response shape", 500);
  }

  return response;
}

export async function markNotificationsRead(
  address: string,
  ids?: number[],
): Promise<void> {
  const res = await fetch(
    `${API_BASE_URL}/api/v1/notifications/${encodeURIComponent(address)}/read`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ ids: ids ?? [] }),
    },
  );

  if (!res.ok) {
    throw new ApiError(`Failed to mark notifications read (HTTP ${res.status})`, res.status);
  }
}

export function interestsUrl(address: string): string {
  return `${API_BASE_URL}/api/v1/users/${encodeURIComponent(address)}/interests`;
}

/** Type guard to validate interests response shape. */
function isInterestsData(obj: unknown): obj is { address: string; interests: string[] } {
  if (!obj || typeof obj !== "object") return false;
  const data = obj as Record<string, unknown>;
  return (
    typeof data.address === "string" &&
    Array.isArray(data.interests) &&
    data.interests.every((i: unknown) => typeof i === "string")
  );
}

export async function fetchInterests(url: string): Promise<string[]> {
  const res = await fetch(url, { headers: { Accept: "application/json" } });

  if (!res.ok) {
    throw new ApiError(`Failed to load interests (HTTP ${res.status})`, res.status);
  }

  const body = await res.json();

  // Handle both wrapped and unwrapped responses
  const data =
    body && typeof body === "object" && "data" in body && body.data ? body.data : body;

  // Validate response shape at boundary
  if (!isInterestsData(data)) {
    throw new ApiError("Invalid interests response shape", 500);
  }

  return data.interests;
}

export async function setInterests(address: string, interests: string[]): Promise<void> {
  const res = await fetch(interestsUrl(address), {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ interests }),
  });

  if (!res.ok) {
    throw new ApiError(`Failed to save interests (HTTP ${res.status})`, res.status);
  }
}

/** Supported weekly digest cadences. */
export type DigestSchedule = "weekly" | "biweekly" | "off";

/** Digest delivery preferences for a user. */
export interface DigestPreferences {
  address: string;
  enabled: boolean;
  schedule: DigestSchedule;
  /** IANA timezone name, e.g. "America/New_York". */
  timezone: string;
  /** ISO timestamp of the next scheduled send, or null when disabled. */
  next_send_at: string | null;
}

/** Payload accepted when updating digest preferences. */
export interface DigestPreferencesUpdate {
  enabled: boolean;
  schedule: DigestSchedule;
  timezone: string;
}

const DIGEST_SCHEDULES: readonly DigestSchedule[] = ["weekly", "biweekly", "off"];

/** Type guard to validate DigestPreferences shape. */
function isDigestPreferences(obj: unknown): obj is DigestPreferences {
  if (!obj || typeof obj !== "object") return false;
  const prefs = obj as Record<string, unknown>;
  return (
    typeof prefs.address === "string" &&
    typeof prefs.enabled === "boolean" &&
    typeof prefs.schedule === "string" &&
    DIGEST_SCHEDULES.includes(prefs.schedule as DigestSchedule) &&
    typeof prefs.timezone === "string" &&
    prefs.timezone.length > 0 &&
    (prefs.next_send_at === null || typeof prefs.next_send_at === "string")
  );
}

export function digestPreferencesUrl(address: string): string {
  return `${API_BASE_URL}/api/v1/users/${encodeURIComponent(address)}/digest`;
}

export async function fetchDigestPreferences(
  address: string,
): Promise<DigestPreferences> {
  const res = await fetch(digestPreferencesUrl(address), {
    headers: { Accept: "application/json" },
  });

  if (!res.ok) {
    throw new ApiError(
      `Failed to load digest preferences (HTTP ${res.status})`,
      res.status,
    );
  }

  const body = await res.json();

  // Handle both wrapped and unwrapped responses
  const data =
    body && typeof body === "object" && "data" in body && body.data ? body.data : body;

  // Validate response shape at boundary
  if (!isDigestPreferences(data)) {
    throw new ApiError("Invalid digest preferences response shape", 500);
  }

  return data;
}

export async function updateDigestPreferences(
  address: string,
  update: DigestPreferencesUpdate,
): Promise<DigestPreferences> {
  const res = await fetch(digestPreferencesUrl(address), {
    method: "PUT",
    headers: { "Content-Type": "application/json", Accept: "application/json" },
    body: JSON.stringify(update),
  });

  if (!res.ok) {
    throw new ApiError(
      `Failed to save digest preferences (HTTP ${res.status})`,
      res.status,
    );
  }

  const body = await res.json();

  // Handle both wrapped and unwrapped responses
  const data =
    body && typeof body === "object" && "data" in body && body.data ? body.data : body;

  // Validate response shape at boundary
  if (!isDigestPreferences(data)) {
    throw new ApiError("Invalid digest preferences response shape", 500);
  }

  return data;
}

/**
 * Build the one-click unsubscribe URL embedded in digest emails.
 *
 * The token is issued by the backend and is single-purpose (digest
 * unsubscribe only); it may expire, in which case the backend responds with
 * an error and the user must unsubscribe from the settings UI instead.
 */
export function digestUnsubscribeUrl(token: string): string {
  return `${API_BASE_URL}/api/v1/digest/unsubscribe?token=${encodeURIComponent(token)}`;
}
