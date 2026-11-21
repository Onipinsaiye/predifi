"use client";

import { useState } from "react";
import { Button, Checkbox } from "@/components/ui";
import {
  getPersistedValue,
  setPersistedValue,
} from "@/lib/persistentStorage";

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

interface NotificationPrefs {
  poolResults: boolean;
  newPools: boolean;
  rewards: boolean;
  marketing: boolean;
  digest: DigestPrefs;
}

const PREFS_CONFIG: { key: keyof NotificationPrefs; label: string; description: string }[] = [
  {
    key: "poolResults",
    label: "Pool results",
    description: "Get notified when a pool you joined is resolved.",
  },
  {
    key: "newPools",
    label: "New pools",
    description: "Alerts for newly created prediction pools.",
  },
  {
    key: "rewards",
    label: "Rewards & payouts",
    description: "Notifications when rewards are distributed.",
  },
  {
    key: "marketing",
    label: "Product updates",
    description: "Occasional announcements and feature releases.",
  },
];

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

const STORAGE_KEY = "notification-preferences";
const DEFAULT_DIGEST: DigestPrefs = {
  enabled: false,
  day: "monday",
  time: "09:00",
  timezone: "UTC",
};
const DEFAULT_PREFS: NotificationPrefs = {
  poolResults: true,
  newPools: true,
  rewards: true,
  marketing: false,
  digest: DEFAULT_DIGEST,
};

function isDigestPrefs(value: unknown): value is DigestPrefs {
  if (!value || typeof value !== "object") return false;

  const digest = value as Record<string, unknown>;
  return (
    typeof digest.enabled === "boolean" &&
    DIGEST_DAYS.some(({ value: day }) => day === digest.day) &&
    typeof digest.time === "string" &&
    TIME_PATTERN.test(digest.time) &&
    typeof digest.timezone === "string" &&
    digest.timezone.length > 0
  );
}

function isNotificationPrefs(value: unknown): value is NotificationPrefs {
  if (!value || typeof value !== "object") return false;

  const prefs = value as Record<string, unknown>;
  return (
    PREFS_CONFIG.every(({ key }) => typeof prefs[key] === "boolean") &&
    isDigestPrefs(prefs.digest)
  );
}

export function NotificationPreferences() {
  const [prefs, setPrefs] = useState<NotificationPrefs>(() => {
    if (typeof window === "undefined") return DEFAULT_PREFS;
    const storedPrefs = getPersistedValue(STORAGE_KEY);
    if (!storedPrefs) return DEFAULT_PREFS;
    try {
      const parsed: unknown = JSON.parse(storedPrefs);
      if (isNotificationPrefs(parsed)) return parsed;
    } catch {
      // Ignore malformed browser data and keep the safe defaults.
    }
    return DEFAULT_PREFS;
  });
  const [saved, setSaved] = useState(false);
  const [saveError, setSaveError] = useState(false);

  function handleToggle(key: keyof NotificationPrefs, checked: boolean) {
    setPrefs((prev) => ({ ...prev, [key]: checked }));
    setSaved(false);
    setSaveError(false);
  }

  function handleDigestChange(patch: Partial<DigestPrefs>) {
    setPrefs((prev) => ({ ...prev, digest: { ...prev.digest, ...patch } }));
    setSaved(false);
    setSaveError(false);
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const didSave = setPersistedValue(STORAGE_KEY, JSON.stringify(prefs));
    setSaved(didSave);
    setSaveError(!didSave);
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold text-white">Notifications</h2>
        <p className="text-xs text-zinc-500">Choose which alerts you want to receive.</p>
      </div>

      <div className="space-y-4">
        {PREFS_CONFIG.map(({ key, label, description }) => (
          <div key={key} className="flex items-start justify-between gap-4 py-3 border-b border-zinc-800 last:border-0">
            <div>
              <p className="text-sm font-medium text-white">{label}</p>
              <p className="text-xs text-zinc-500 mt-0.5">{description}</p>
            </div>
            <Checkbox
              checked={prefs[key]}
              onCheckedChange={(checked) => handleToggle(key, checked === true)}
              aria-label={label}
            />
          </div>
        ))}
      </div>

      <div className="space-y-4 border-t border-zinc-800 pt-6">
        <div className="space-y-1">
          <h3 className="text-sm font-semibold text-white">Weekly market digest</h3>
          <p className="text-xs text-zinc-500">
            A single weekly summary of public status changes for the markets you follow.
            Only public market status changes are included — no per-event notifications.
          </p>
        </div>

        <div className="flex items-start justify-between gap-4 py-3">
          <div>
            <p className="text-sm font-medium text-white">Send weekly digest</p>
            <p className="text-xs text-zinc-500 mt-0.5">
              Opt in to receive the digest at the schedule below.
            </p>
          </div>
          <Checkbox
            checked={prefs.digest.enabled}
            onCheckedChange={(checked) => handleDigestChange({ enabled: checked === true })}
            aria-label="Send weekly digest"
          />
        </div>

        <div className="grid gap-4 sm:grid-cols-3">
          <div className="space-y-1">
            <label htmlFor="digest-day" className="text-xs font-medium text-zinc-400">
              Day of week
            </label>
            <select
              id="digest-day"
              value={prefs.digest.day}
              disabled={!prefs.digest.enabled}
              onChange={(e) => handleDigestChange({ day: e.target.value as DigestDay })}
              className="w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white disabled:opacity-50"
            >
              {DIGEST_DAYS.map(({ value, label }) => (
                <option key={value} value={value}>{label}</option>
              ))}
            </select>
          </div>

          <div className="space-y-1">
            <label htmlFor="digest-time" className="text-xs font-medium text-zinc-400">
              Send time (local)
            </label>
            <input
              id="digest-time"
              type="time"
              value={prefs.digest.time}
              disabled={!prefs.digest.enabled}
              onChange={(e) => handleDigestChange({ time: e.target.value })}
              className="w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white disabled:opacity-50"
            />
          </div>

          <div className="space-y-1">
            <label htmlFor="digest-timezone" className="text-xs font-medium text-zinc-400">
              Timezone
            </label>
            <select
              id="digest-timezone"
              value={prefs.digest.timezone}
              disabled={!prefs.digest.enabled}
              onChange={(e) => handleDigestChange({ timezone: e.target.value })}
              className="w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white disabled:opacity-50"
            >
              {DIGEST_TIMEZONES.map(({ value, label }) => (
                <option key={value} value={value}>{label}</option>
              ))}
            </select>
          </div>
        </div>

        <p className="text-xs text-zinc-500">
          Times follow your selected timezone, including daylight saving changes.
          You can unsubscribe from the digest at any time using the link in the email.
        </p>
      </div>

      <div className="flex items-center gap-3">
        <Button type="submit" size="medium">Save preferences</Button>
        {saved && (
          <span className="text-sm text-green-400" role="status">Saved!</span>
        )}
        {saveError && (
          <span className="text-sm text-red-400" role="alert">
            Your browser blocked preference storage.
          </span>
        )}
      </div>
    </form>
  );
}
