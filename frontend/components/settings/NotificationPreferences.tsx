"use client";

/**
 * NotificationPreferences settings panel (#286)
 *
 * Renders per-event-type and per-channel toggles backed by the backend
 * notification-preferences API. Changes are saved on toggle so there is no
 * "Save" button — feedback is given inline.
 *
 * Default-deny: the backend starts all preferences off until the user
 * explicitly enables them. The unsubscribe path sets everything off at once.
 */

import { useState } from "react";
import { Button, Checkbox } from "@/components/ui";
import {
  getPersistedValue,
  setPersistedValue,
} from "@/lib/persistentStorage";
import {
  EVENT_TYPE_LABELS,
  CHANNEL_LABELS,
  NOTIFICATION_EVENT_TYPES,
  NOTIFICATION_CHANNELS,
  type NotificationEventType,
  type NotificationChannel,
} from "@/lib/api/notifications";
import { useNotificationPreferences } from "@/lib/hooks/useNotificationPreferences";
import { useWalletAddress } from "@/lib/context";

// ── Digest schedule (local-only, unchanged from before) ───────────────────────

type DigestDay =
  | "monday"
  | "tuesday"
  | "wednesday"
  | "thursday"
  | "friday"
  | "saturday"
  | "sunday";

interface DigestPrefs {
  enabled: boolean;
  day: DigestDay;
  time: string;
  timezone: string;
}

const DIGEST_DAYS: { value: DigestDay; label: string }[] = [
  { value: "monday", label: "Monday" },
  { value: "tuesday", label: "Tuesday" },
  { value: "wednesday", label: "Wednesday" },
  { value: "thursday", label: "Thursday" },
  { value: "friday", label: "Friday" },
  { value: "saturday", label: "Saturday" },
  { value: "sunday", label: "Sunday" },
];

const DIGEST_TIMEZONES: { value: string; label: string }[] = [
  { value: "UTC", label: "UTC" },
  { value: "America/New_York", label: "Eastern Time (US)" },
  { value: "America/Chicago", label: "Central Time (US)" },
  { value: "America/Denver", label: "Mountain Time (US)" },
  { value: "America/Los_Angeles", label: "Pacific Time (US)" },
  { value: "Europe/London", label: "London" },
  { value: "Europe/Berlin", label: "Berlin" },
  { value: "Asia/Tokyo", label: "Tokyo" },
  { value: "Australia/Sydney", label: "Sydney" },
];

const TIME_PATTERN = /^([01]\d|2[0-3]):[0-5]\d$/;
const DIGEST_STORAGE_KEY = "notification-digest-schedule";

const DEFAULT_DIGEST: DigestPrefs = {
  enabled: false,
  day: "monday",
  time: "09:00",
  timezone: "UTC",
};

function isDigestPrefs(value: unknown): value is DigestPrefs {
  if (!value || typeof value !== "object") return false;
  const d = value as Record<string, unknown>;
  return (
    typeof d.enabled === "boolean" &&
    DIGEST_DAYS.some(({ value: day }) => day === d.day) &&
    typeof d.time === "string" &&
    TIME_PATTERN.test(d.time) &&
    typeof d.timezone === "string" &&
    d.timezone.length > 0
  );
}

function loadDigestPrefs(): DigestPrefs {
  if (typeof window === "undefined") return DEFAULT_DIGEST;
  const raw = getPersistedValue(DIGEST_STORAGE_KEY);
  if (!raw) return DEFAULT_DIGEST;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (isDigestPrefs(parsed)) return parsed;
  } catch {
    // ignore
  }
  return DEFAULT_DIGEST;
}

// ── Event type descriptions ───────────────────────────────────────────────────

const EVENT_DESCRIPTIONS: Record<NotificationEventType, string> = {
  pool_ending_soon: "Alert before a pool you joined closes.",
  pool_resolved: "Alert when a pool you joined is settled.",
  claim_expiring: "Reminder before your claim window closes.",
  new_pool_match: "Alert when a new pool matches your interests.",
};

const CHANNEL_DESCRIPTIONS: Record<NotificationChannel, string> = {
  in_app: "Show notifications inside the app.",
  push: "Send browser push notifications (requires permission).",
  email: "Send email notifications.",
};

// ── Component ─────────────────────────────────────────────────────────────────

export function NotificationPreferences() {
  const address = useWalletAddress();
  const {
    preferences,
    isLoading,
    isError,
    isSaving,
    setEvent,
    setChannel,
    unsubscribeAll,
  } = useNotificationPreferences(address);

  const [digest, setDigest] = useState<DigestPrefs>(loadDigestPrefs);
  const [digestSaved, setDigestSaved] = useState(false);
  const [digestError, setDigestError] = useState(false);
  const [unsubscribeDone, setUnsubscribeDone] = useState(false);

  function handleDigestChange(patch: Partial<DigestPrefs>) {
    setDigest((prev) => ({ ...prev, ...patch }));
    setDigestSaved(false);
    setDigestError(false);
  }

  function handleDigestSubmit(e: React.FormEvent) {
    e.preventDefault();
    const ok = setPersistedValue(DIGEST_STORAGE_KEY, JSON.stringify(digest));
    setDigestSaved(ok);
    setDigestError(!ok);
  }

  async function handleUnsubscribeAll() {
    await unsubscribeAll();
    setUnsubscribeDone(true);
    setTimeout(() => setUnsubscribeDone(false), 3000);
  }

  if (!address) {
    return (
      <div className="space-y-1">
        <h2 className="text-lg font-semibold text-white">Notifications</h2>
        <p className="text-sm text-zinc-400">Connect a wallet to manage notification preferences.</p>
      </div>
    );
  }

  return (
    <div className="space-y-8">
      {/* Header */}
      <div className="space-y-1">
        <h2 className="text-lg font-semibold text-white">Notifications</h2>
        <p className="text-xs text-zinc-500">
          Choose which alerts you want to receive and how they are delivered.
          All notifications are off by default — enable only what you want.
        </p>
      </div>

      {isError && (
        <p className="text-sm text-red-400" role="alert">
          Failed to load your preferences. Changes will be retried on next save.
        </p>
      )}

      {/* Event types */}
      <section aria-labelledby="events-heading">
        <h3 id="events-heading" className="text-sm font-semibold text-white mb-3">
          Alert types
        </h3>
        <div className="space-y-0 divide-y divide-zinc-800 border border-zinc-800 rounded-lg overflow-hidden">
          {NOTIFICATION_EVENT_TYPES.map((event) => (
            <div
              key={event}
              className="flex items-start justify-between gap-4 px-4 py-3 bg-zinc-900 hover:bg-zinc-800/50 transition-colors"
            >
              <div>
                <p className="text-sm font-medium text-white">{EVENT_TYPE_LABELS[event]}</p>
                <p className="text-xs text-zinc-500 mt-0.5">{EVENT_DESCRIPTIONS[event]}</p>
              </div>
              <Checkbox
                checked={preferences.events[event]}
                disabled={isLoading || isSaving}
                onCheckedChange={(checked) => setEvent(event, checked === true)}
                aria-label={EVENT_TYPE_LABELS[event]}
              />
            </div>
          ))}
        </div>
      </section>

      {/* Delivery channels */}
      <section aria-labelledby="channels-heading">
        <h3 id="channels-heading" className="text-sm font-semibold text-white mb-3">
          Delivery channels
        </h3>
        <div className="space-y-0 divide-y divide-zinc-800 border border-zinc-800 rounded-lg overflow-hidden">
          {NOTIFICATION_CHANNELS.map((channel) => (
            <div
              key={channel}
              className="flex items-start justify-between gap-4 px-4 py-3 bg-zinc-900 hover:bg-zinc-800/50 transition-colors"
            >
              <div>
                <p className="text-sm font-medium text-white">{CHANNEL_LABELS[channel]}</p>
                <p className="text-xs text-zinc-500 mt-0.5">{CHANNEL_DESCRIPTIONS[channel]}</p>
              </div>
              <Checkbox
                checked={preferences.channels[channel]}
                disabled={isLoading || isSaving}
                onCheckedChange={(checked) => setChannel(channel, checked === true)}
                aria-label={CHANNEL_LABELS[channel]}
              />
            </div>
          ))}
        </div>
        {isSaving && (
          <p className="mt-2 text-xs text-zinc-500" role="status" aria-live="polite">
            Saving…
          </p>
        )}
      </section>

      {/* Weekly digest schedule (local only) */}
      <section aria-labelledby="digest-heading" className="border-t border-zinc-800 pt-6">
        <div className="space-y-1 mb-4">
          <h3 id="digest-heading" className="text-sm font-semibold text-white">
            Weekly market digest
          </h3>
          <p className="text-xs text-zinc-500">
            A single weekly summary of public status changes for markets you follow.
            Only public market status is included — no per-event alerts.
          </p>
        </div>

        <form onSubmit={handleDigestSubmit} className="space-y-4">
          <div className="flex items-start justify-between gap-4 py-3">
            <div>
              <p className="text-sm font-medium text-white">Send weekly digest</p>
              <p className="text-xs text-zinc-500 mt-0.5">Opt in to receive the digest at the schedule below.</p>
            </div>
            <Checkbox
              checked={digest.enabled}
              onCheckedChange={(checked) => handleDigestChange({ enabled: checked === true })}
              aria-label="Send weekly digest"
            />
          </div>

          <div className="grid gap-4 sm:grid-cols-3">
            <div className="space-y-1">
              <label htmlFor="digest-day" className="text-xs font-medium text-zinc-400">Day of week</label>
              <select
                id="digest-day"
                value={digest.day}
                disabled={!digest.enabled}
                onChange={(e) => handleDigestChange({ day: e.target.value as DigestDay })}
                className="w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white disabled:opacity-50"
              >
                {DIGEST_DAYS.map(({ value, label }) => (
                  <option key={value} value={value}>{label}</option>
                ))}
              </select>
            </div>

            <div className="space-y-1">
              <label htmlFor="digest-time" className="text-xs font-medium text-zinc-400">Send time (local)</label>
              <input
                id="digest-time"
                type="time"
                value={digest.time}
                disabled={!digest.enabled}
                onChange={(e) => handleDigestChange({ time: e.target.value })}
                className="w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white disabled:opacity-50"
              />
            </div>

            <div className="space-y-1">
              <label htmlFor="digest-timezone" className="text-xs font-medium text-zinc-400">Timezone</label>
              <select
                id="digest-timezone"
                value={digest.timezone}
                disabled={!digest.enabled}
                onChange={(e) => handleDigestChange({ timezone: e.target.value })}
                className="w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white disabled:opacity-50"
              >
                {DIGEST_TIMEZONES.map(({ value, label }) => (
                  <option key={value} value={value}>{label}</option>
                ))}
              </select>
            </div>
          </div>

          <div className="flex items-center gap-3">
            <Button type="submit" size="medium">Save digest schedule</Button>
            {digestSaved && <span className="text-sm text-green-400" role="status">Saved!</span>}
            {digestError && (
              <span className="text-sm text-red-400" role="alert">
                Your browser blocked preference storage.
              </span>
            )}
          </div>
        </form>
      </section>

      {/* Unsubscribe path */}
      <section className="border-t border-zinc-800 pt-6">
        <div className="space-y-1 mb-4">
          <h3 className="text-sm font-semibold text-white">Unsubscribe from all notifications</h3>
          <p className="text-xs text-zinc-500">
            Disables every event type and delivery channel at once. You can re-enable
            individual notifications above at any time.
          </p>
        </div>
        <div className="flex items-center gap-3">
          <Button
            type="button"
            variant="outline"
            size="medium"
            onClick={handleUnsubscribeAll}
            disabled={isSaving}
          >
            Unsubscribe from all
          </Button>
          {unsubscribeDone && (
            <span className="text-sm text-green-400" role="status">Done — all notifications disabled.</span>
          )}
        </div>
      </section>
    </div>
  );
}
