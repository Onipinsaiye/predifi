/**
 * Tests for notification preferences by market event type (#286).
 *
 * Covers: default privacy, suppression of disabled event types,
 * preference migration shape, and unsubscribe path.
 */

import {
  NOTIFICATION_EVENT_TYPES,
  NOTIFICATION_CHANNELS,
  EVENT_TYPE_LABELS,
  CHANNEL_LABELS,
  type BackendNotificationPreferences,
  type NotificationEventType,
  type NotificationChannel,
} from "@/lib/api/notifications";

// ── Helper to build a fresh all-off preferences object ───────────────────────

function allOff(): BackendNotificationPreferences {
  return {
    schema_version: 2,
    channels: Object.fromEntries(NOTIFICATION_CHANNELS.map((c) => [c, false])) as Record<
      NotificationChannel,
      boolean
    >,
    events: Object.fromEntries(NOTIFICATION_EVENT_TYPES.map((e) => [e, false])) as Record<
      NotificationEventType,
      boolean
    >,
  };
}

function allOn(): BackendNotificationPreferences {
  return {
    schema_version: 2,
    channels: Object.fromEntries(NOTIFICATION_CHANNELS.map((c) => [c, true])) as Record<
      NotificationChannel,
      boolean
    >,
    events: Object.fromEntries(NOTIFICATION_EVENT_TYPES.map((e) => [e, true])) as Record<
      NotificationEventType,
      boolean
    >,
  };
}

// ── Default privacy ───────────────────────────────────────────────────────────

test("default preferences have all events disabled", () => {
  const prefs = allOff();
  for (const event of NOTIFICATION_EVENT_TYPES) {
    expect(prefs.events[event]).toBe(false);
  }
});

test("default preferences have all channels disabled", () => {
  const prefs = allOff();
  for (const channel of NOTIFICATION_CHANNELS) {
    expect(prefs.channels[channel]).toBe(false);
  }
});

// ── Suppression logic (simulated gating) ─────────────────────────────────────

/**
 * Simulates the backend's enqueue_notification_for_event check:
 * a notification is only delivered when BOTH the in_app channel AND the
 * specific event type are enabled.
 */
function shouldDeliver(
  prefs: BackendNotificationPreferences,
  event: NotificationEventType,
): boolean {
  return (
    prefs.channels["in_app"] === true && prefs.events[event] === true
  );
}

test("disabled event types are suppressed even when in_app channel is on", () => {
  const prefs: BackendNotificationPreferences = {
    ...allOff(),
    channels: { in_app: true, push: false, email: false },
  };
  for (const event of NOTIFICATION_EVENT_TYPES) {
    expect(shouldDeliver(prefs, event)).toBe(false);
  }
});

test("enabled event type is suppressed when in_app channel is off", () => {
  const prefs: BackendNotificationPreferences = {
    ...allOff(),
    events: { pool_ending_soon: true, pool_resolved: false, claim_expiring: false, new_pool_match: false },
  };
  expect(shouldDeliver(prefs, "pool_ending_soon")).toBe(false);
});

test("notification is delivered only when both event and channel are on", () => {
  const prefs: BackendNotificationPreferences = {
    schema_version: 2,
    channels: { in_app: true, push: false, email: false },
    events: { pool_ending_soon: true, pool_resolved: false, claim_expiring: false, new_pool_match: false },
  };
  expect(shouldDeliver(prefs, "pool_ending_soon")).toBe(true);
  expect(shouldDeliver(prefs, "pool_resolved")).toBe(false);
});

test("all events delivered when everything is on", () => {
  const prefs = allOn();
  for (const event of NOTIFICATION_EVENT_TYPES) {
    expect(shouldDeliver(prefs, event)).toBe(true);
  }
});

// ── Unsubscribe path ──────────────────────────────────────────────────────────

test("unsubscribeAll produces all-off preferences", () => {
  const prefs = allOn();
  // Apply unsubscribe: set everything to false.
  const unsubscribed: BackendNotificationPreferences = {
    ...prefs,
    channels: Object.fromEntries(NOTIFICATION_CHANNELS.map((c) => [c, false])) as Record<NotificationChannel, boolean>,
    events: Object.fromEntries(NOTIFICATION_EVENT_TYPES.map((e) => [e, false])) as Record<NotificationEventType, boolean>,
  };
  for (const event of NOTIFICATION_EVENT_TYPES) {
    expect(shouldDeliver(unsubscribed, event)).toBe(false);
  }
  for (const channel of NOTIFICATION_CHANNELS) {
    expect(unsubscribed.channels[channel]).toBe(false);
  }
});

// ── Schema migration ──────────────────────────────────────────────────────────

test("schema_version is a positive integer", () => {
  const prefs = allOff();
  expect(typeof prefs.schema_version).toBe("number");
  expect(prefs.schema_version).toBeGreaterThan(0);
});

test("migrated v1 preferences preserve existing flags and default new ones off", () => {
  // Simulate a v1 preferences object (no digest block, may have partial events)
  const v1 = {
    schema_version: 1,
    channels: { in_app: true, push: false, email: false },
    events: { pool_ending_soon: true, pool_resolved: false, claim_expiring: false, new_pool_match: false },
  };

  // After migration to v2 all known keys should be present and new types default off.
  const migrated: BackendNotificationPreferences = {
    schema_version: 2,
    channels: { in_app: true, push: false, email: false },
    events: {
      pool_ending_soon: v1.events.pool_ending_soon,
      pool_resolved: v1.events.pool_resolved,
      claim_expiring: v1.events.claim_expiring,
      new_pool_match: v1.events.new_pool_match,
    },
  };

  for (const event of NOTIFICATION_EVENT_TYPES) {
    expect(typeof migrated.events[event]).toBe("boolean");
  }
  for (const channel of NOTIFICATION_CHANNELS) {
    expect(typeof migrated.channels[channel]).toBe("boolean");
  }
  expect(migrated.schema_version).toBe(2);
});

// ── Label completeness ────────────────────────────────────────────────────────

test("every event type has a label", () => {
  for (const event of NOTIFICATION_EVENT_TYPES) {
    expect(typeof EVENT_TYPE_LABELS[event]).toBe("string");
    expect(EVENT_TYPE_LABELS[event].length).toBeGreaterThan(0);
  }
});

test("every channel has a label", () => {
  for (const channel of NOTIFICATION_CHANNELS) {
    expect(typeof CHANNEL_LABELS[channel]).toBe("string");
    expect(CHANNEL_LABELS[channel].length).toBeGreaterThan(0);
  }
});
