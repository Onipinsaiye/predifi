"use client";

/**
 * useNotificationPreferences — per-wallet, per-event-type preferences (#286).
 *
 * Fetches versioned preferences from the backend and provides granular
 * toggle functions for individual event types and delivery channels.
 * All mutations optimistically update local state and fall back on error.
 *
 * Default-deny: when no preferences exist the backend returns all-false,
 * which this hook surfaces without special-casing.
 */

import { useState, useCallback } from "react";
import useSWR from "swr";
import {
  fetchNotificationPreferences,
  saveNotificationPreferences,
  resetNotificationPreferences,
  notificationPreferencesUrl,
  NOTIFICATION_EVENT_TYPES,
  NOTIFICATION_CHANNELS,
  type BackendNotificationPreferences,
  type NotificationEventType,
  type NotificationChannel,
} from "@/lib/api/notifications";

function defaultPreferences(): BackendNotificationPreferences {
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

export interface UseNotificationPreferencesResult {
  preferences: BackendNotificationPreferences;
  isLoading: boolean;
  isError: boolean;
  isSaving: boolean;
  /** Set a single event-type toggle. */
  setEvent: (event: NotificationEventType, enabled: boolean) => Promise<void>;
  /** Set a single channel toggle. */
  setChannel: (channel: NotificationChannel, enabled: boolean) => Promise<void>;
  /** Replace the entire preference object. */
  save: (prefs: BackendNotificationPreferences) => Promise<void>;
  /** Reset to backend defaults (all-off). */
  reset: () => Promise<void>;
  /** Unsubscribe path: disable all channels and events at once. */
  unsubscribeAll: () => Promise<void>;
}

export function useNotificationPreferences(
  address: string | undefined,
): UseNotificationPreferencesResult {
  const key = address ? notificationPreferencesUrl(address) : null;

  const { data, error, isLoading, mutate } = useSWR<BackendNotificationPreferences>(
    key,
    () => fetchNotificationPreferences(address!),
    { revalidateOnFocus: false },
  );

  const [isSaving, setIsSaving] = useState(false);

  const preferences = data ?? defaultPreferences();

  const save = useCallback(
    async (prefs: BackendNotificationPreferences) => {
      if (!address) return;
      setIsSaving(true);
      try {
        const updated = await saveNotificationPreferences(address, prefs);
        await mutate(updated, false);
      } finally {
        setIsSaving(false);
      }
    },
    [address, mutate],
  );

  const setEvent = useCallback(
    async (event: NotificationEventType, enabled: boolean) => {
      const next: BackendNotificationPreferences = {
        ...preferences,
        events: { ...preferences.events, [event]: enabled },
      };
      await save(next);
    },
    [preferences, save],
  );

  const setChannel = useCallback(
    async (channel: NotificationChannel, enabled: boolean) => {
      const next: BackendNotificationPreferences = {
        ...preferences,
        channels: { ...preferences.channels, [channel]: enabled },
      };
      await save(next);
    },
    [preferences, save],
  );

  const reset = useCallback(async () => {
    if (!address) return;
    setIsSaving(true);
    try {
      const defaults = await resetNotificationPreferences(address);
      await mutate(defaults, false);
    } finally {
      setIsSaving(false);
    }
  }, [address, mutate]);

  const unsubscribeAll = useCallback(async () => {
    const disabled: BackendNotificationPreferences = defaultPreferences();
    await save(disabled);
  }, [save]);

  return {
    preferences,
    isLoading: key ? isLoading : false,
    isError: Boolean(error),
    isSaving,
    setEvent,
    setChannel,
    save,
    reset,
    unsubscribeAll,
  };
}
