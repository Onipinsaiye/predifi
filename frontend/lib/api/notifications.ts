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

// ── Cursor-paginated inbox (#287) ─────────────────────────────────────────────

/**
 * A single page of inbox notifications with a read-cursor for pagination and
 * an unread count taken from the same consistent snapshot.
 */
export interface NotificationInboxPage {
  notifications: Notification[];
  unread_count: number;
  /** Opaque cursor to pass as `?cursor=` on the next request, or null when exhausted. */
  next_cursor: string | null;
}

function isNotificationInboxPage(obj: unknown): obj is NotificationInboxPage {
  if (!obj || typeof obj !== "object") return false;
  const p = obj as Record<string, unknown>;
  return (
    Array.isArray(p.notifications) &&
    p.notifications.every(isNotification) &&
    typeof p.unread_count === "number" &&
    (p.next_cursor === null || typeof p.next_cursor === "string")
  );
}

/**
 * Build the URL for a cursor-paginated inbox page.
 * The cursor is the opaque value returned in `next_cursor` from the previous page.
 */
export function inboxPageUrl(
  address: string,
  options?: { cursor?: string; limit?: number; unreadOnly?: boolean },
): string {
  const params = new URLSearchParams();
  if (options?.cursor) params.set("cursor", options.cursor);
  if (options?.limit) params.set("limit", String(options.limit));
  if (options?.unreadOnly) params.set("unread_only", "true");
  const qs = params.toString();
  return `${API_BASE_URL}/api/v1/notifications/${encodeURIComponent(address)}/inbox${qs ? `?${qs}` : ""}`;
}

/** Fetch one page of the inbox. Used as an SWR fetcher. */
export async function fetchInboxPage(url: string): Promise<NotificationInboxPage> {
  const res = await fetch(url, { headers: { Accept: "application/json" } });

  if (!res.ok) {
    throw new ApiError(`Failed to load inbox (HTTP ${res.status})`, res.status);
  }

  const body = await res.json();
  const data =
    body && typeof body === "object" && "data" in body && body.data ? body.data : body;

  if (!isNotificationInboxPage(data)) {
    throw new ApiError("Invalid inbox page response shape", 500);
  }

  return data;
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

// ── Browser push subscriptions (#288) ────────────────────────────────────────

/** Request body to register a new push subscription. */
export interface PushSubscriptionRequest {
  endpoint: string;
  p256dh: string;
  auth: string;
  expires_at?: string;
  /** Previous endpoint to revoke atomically when the browser rotates keys. */
  previous_endpoint?: string;
}

/** Minimal summary returned after a successful subscription save. */
export interface PushSubscriptionSummary {
  id: number;
  expires_at: string;
}

function isPushSubscriptionSummary(obj: unknown): obj is PushSubscriptionSummary {
  if (!obj || typeof obj !== "object") return false;
  const s = obj as Record<string, unknown>;
  return typeof s.id === "number" && typeof s.expires_at === "string";
}

export function pushSubscriptionUrl(address: string): string {
  return `${API_BASE_URL}/api/v1/users/${encodeURIComponent(address)}/push-subscriptions`;
}

/**
 * Register a push subscription for `address`.
 * Binds the subscription to the wallet so revocation is wallet-scoped.
 * Sends `previous_endpoint` when the browser rotated keys so the old
 * subscription is atomically replaced.
 */
export async function savePushSubscription(
  address: string,
  request: PushSubscriptionRequest,
): Promise<PushSubscriptionSummary> {
  const res = await fetch(pushSubscriptionUrl(address), {
    method: "POST",
    headers: { "Content-Type": "application/json", Accept: "application/json" },
    body: JSON.stringify(request),
  });

  if (!res.ok) {
    throw new ApiError(
      `Failed to save push subscription (HTTP ${res.status})`,
      res.status,
    );
  }

  const body = await res.json();
  const data =
    body && typeof body === "object" && "data" in body && body.data ? body.data : body;

  if (!isPushSubscriptionSummary(data)) {
    throw new ApiError("Invalid push subscription response shape", 500);
  }

  return data;
}

/**
 * Revoke the push subscription for `address`.
 * Pass `endpoint` to revoke a specific subscription, or omit to revoke all.
 */
export async function revokePushSubscription(
  address: string,
  endpoint?: string,
): Promise<void> {
  const url = endpoint
    ? `${pushSubscriptionUrl(address)}?endpoint=${encodeURIComponent(endpoint)}`
    : pushSubscriptionUrl(address);

  const res = await fetch(url, {
    method: "DELETE",
    headers: { Accept: "application/json" },
  });

  if (!res.ok) {
    throw new ApiError(
      `Failed to revoke push subscription (HTTP ${res.status})`,
      res.status,
    );
  }
}

/**
/**
 * Convert a browser `PushSubscription` object into the API request shape.
 * Keeps sensitive keys out of logs by never logging the returned object.
 */
export function pushSubscriptionToRequest(
  sub: PushSubscription,
  previousEndpoint?: string,
): PushSubscriptionRequest {
  const json = sub.toJSON();
  const keys = json.keys ?? {};
  return {
    endpoint: sub.endpoint,
    p256dh: keys.p256dh ?? "",
    auth: keys.auth ?? "",
    expires_at: sub.expirationTime
      ? new Date(sub.expirationTime).toISOString()
      : undefined,
    previous_endpoint: previousEndpoint,
  };
}

// ── Per-wallet notification preferences by event type and channel (#286) ──────

/** Event types the backend supports. Maps 1-to-1 with the backend enum. */
export type NotificationEventType =
  | "pool_ending_soon"
  | "pool_resolved"
  | "claim_expiring"
  | "new_pool_match";

/** Delivery channels the backend supports. */
export type NotificationChannel = "in_app" | "push" | "email";

export const NOTIFICATION_EVENT_TYPES: NotificationEventType[] = [
  "pool_ending_soon",
  "pool_resolved",
  "claim_expiring",
  "new_pool_match",
];

export const NOTIFICATION_CHANNELS: NotificationChannel[] = [
  "in_app",
  "push",
  "email",
];

/** Human-readable labels for each event type. */
export const EVENT_TYPE_LABELS: Record<NotificationEventType, string> = {
  pool_ending_soon: "Pool ending soon",
  pool_resolved: "Pool resolved",
  claim_expiring: "Claim window expiring",
  new_pool_match: "New pool matching interests",
};

/** Human-readable labels for each channel. */
export const CHANNEL_LABELS: Record<NotificationChannel, string> = {
  in_app: "In-app",
  push: "Browser push",
  email: "Email",
};

/**
 * Versioned per-wallet preferences as returned by the backend.
 * Default is all-off (default-deny).
 */
export interface BackendNotificationPreferences {
  schema_version: number;
  channels: Record<NotificationChannel, boolean>;
  events: Record<NotificationEventType, boolean>;
}

function isBackendNotificationPreferences(
  obj: unknown,
): obj is BackendNotificationPreferences {
  if (!obj || typeof obj !== "object") return false;
  const p = obj as Record<string, unknown>;
  if (typeof p.schema_version !== "number") return false;
  if (!p.channels || typeof p.channels !== "object") return false;
  if (!p.events || typeof p.events !== "object") return false;
  const channels = p.channels as Record<string, unknown>;
  const events = p.events as Record<string, unknown>;
  return (
    NOTIFICATION_CHANNELS.every((c) => typeof channels[c] === "boolean") &&
    NOTIFICATION_EVENT_TYPES.every((e) => typeof events[e] === "boolean")
  );
}

export function notificationPreferencesUrl(address: string): string {
  return `${API_BASE_URL}/api/v1/users/${encodeURIComponent(address)}/notification-preferences`;
}

/** Fetch the current per-event-type and per-channel preferences for `address`. */
export async function fetchNotificationPreferences(
  address: string,
): Promise<BackendNotificationPreferences> {
  const res = await fetch(notificationPreferencesUrl(address), {
    headers: { Accept: "application/json" },
  });

  if (!res.ok) {
    throw new ApiError(
      `Failed to load notification preferences (HTTP ${res.status})`,
      res.status,
    );
  }

  const body = await res.json();
  const data =
    body && typeof body === "object" && "data" in body && body.data ? body.data : body;

  if (!isBackendNotificationPreferences(data)) {
    throw new ApiError("Invalid notification preferences response shape", 500);
  }

  return data;
}

/**
 * Save updated preferences for `address`.
 * Returns the normalised preferences as confirmed by the backend.
 */
export async function saveNotificationPreferences(
  address: string,
  prefs: BackendNotificationPreferences,
): Promise<BackendNotificationPreferences> {
  const res = await fetch(notificationPreferencesUrl(address), {
    method: "PUT",
    headers: { "Content-Type": "application/json", Accept: "application/json" },
    body: JSON.stringify(prefs),
  });

  if (!res.ok) {
    throw new ApiError(
      `Failed to save notification preferences (HTTP ${res.status})`,
      res.status,
    );
  }

  const body = await res.json();
  const data =
    body && typeof body === "object" && "data" in body && body.data ? body.data : body;

  if (!isBackendNotificationPreferences(data)) {
    throw new ApiError("Invalid notification preferences response shape", 500);
  }

  return data;
}

/**
 * Reset preferences for `address` to the backend default (all-off).
 * Returns the default preferences.
 */
export async function resetNotificationPreferences(
  address: string,
): Promise<BackendNotificationPreferences> {
  const res = await fetch(notificationPreferencesUrl(address), {
    method: "DELETE",
    headers: { Accept: "application/json" },
  });

  if (!res.ok) {
    throw new ApiError(
      `Failed to reset notification preferences (HTTP ${res.status})`,
      res.status,
    );
  }

  const body = await res.json();
  const data =
    body && typeof body === "object" && "data" in body && body.data ? body.data : body;

  if (!isBackendNotificationPreferences(data)) {
    throw new ApiError("Invalid notification preferences response shape", 500);
  }

  return data;
}
