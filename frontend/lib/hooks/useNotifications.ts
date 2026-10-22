"use client";

import useSWR from "swr";
import {
  fetchNotifications,
  markNotificationsRead,
  notificationsUrl,
  type NotificationsResponse,
} from "@/lib/api/notifications";

export interface UseNotificationsResult {
  notifications: NotificationsResponse["notifications"];
  unreadCount: number;
  isLoading: boolean;
  isError: boolean;
  markRead: (ids?: number[]) => Promise<void>;
  refresh: () => void;
}

/**
 * useNotifications — polls a user's notifications every 30s so the bell badge
 * stays fresh without a WebSocket round-trip.
 *
 * The SWR key is scoped by both network and address so that notifications from
 * a previous account or network are never reused after an identity change.
 */
export function useNotifications(
  address: string | undefined,
  network?: string,
): UseNotificationsResult {
  const key = address ? notificationsUrl(address, network) : null;

  const { data, error, isLoading, mutate } = useSWR<NotificationsResponse>(
    key,
    fetchNotifications,
    { refreshInterval: 30_000 },
  );

  async function markRead(ids?: number[]) {
    if (!address) return;
    await markNotificationsRead(address, ids, network);
    void mutate();
  }

  // When there is no identity (disconnected) or the key has changed, never
  // surface data from a previous account/network.
  const notifications = key ? data?.notifications ?? [] : [];
  const unreadCount = key ? data?.unread_count ?? 0 : 0;

  return {
    notifications,
    unreadCount,
    isLoading: key ? isLoading : false,
    isError: Boolean(error),
    markRead,
    refresh: () => {
      void mutate();
    },
  };
}