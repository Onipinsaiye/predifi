"use client";

/**
 * useNotificationInbox — cursor-paginated, durable notification inbox (#287).
 *
 * Fetches one page at a time using opaque read cursors from the backend.
 * Pages are accumulated into a single ordered list so the inbox can grow
 * by appending without re-fetching earlier pages.
 *
 * Key properties:
 * - Stable IDs: each notification has a backend-assigned numeric ID that is
 *   used for deduplication, so reconnect or repeated `loadMore` calls never
 *   produce duplicates.
 * - Per-wallet isolation: changing `address` resets the inbox so a previous
 *   wallet's data is never shown.
 * - `unread_count` comes from the same consistent DB snapshot as each page so
 *   badge counts are always coherent with the visible list.
 * - The read cursor advances automatically as pages are loaded; marking all
 *   read forces a refresh so the new state is reflected immediately.
 */

import { useState, useCallback, useEffect, useRef } from "react";
import {
  fetchInboxPage,
  inboxPageUrl,
  markNotificationsRead,
  type Notification,
  type NotificationInboxPage,
} from "@/lib/api/notifications";

const DEFAULT_PAGE_SIZE = 20;

export interface UseNotificationInboxResult {
  /** Accumulated, deduplicated notifications newest-first. */
  notifications: Notification[];
  /** Total unread from the most recent page snapshot. */
  unreadCount: number;
  isLoading: boolean;
  isError: boolean;
  /** True when there are more pages to load. */
  hasMore: boolean;
  /** Load the next page. */
  loadMore: () => Promise<void>;
  /** Mark specific IDs (or all) as read. */
  markRead: (ids?: number[]) => Promise<void>;
  /** Refresh from the first page (e.g. on reconnect). */
  refresh: () => Promise<void>;
}

export function useNotificationInbox(
  address: string | undefined,
  pageSize = DEFAULT_PAGE_SIZE,
): UseNotificationInboxResult {
  // Notifications are accumulated here; we deduplicate by ID.
  const [notifications, setNotifications] = useState<Notification[]>([]);
  const [unreadCount, setUnreadCount] = useState(0);
  const [cursor, setCursor] = useState<string | undefined>(undefined);
  const [hasMore, setHasMore] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [isError, setIsError] = useState(false);

  // Track the current address to reset state on wallet switch.
  const addressRef = useRef(address);

  const reset = useCallback(() => {
    setNotifications([]);
    setUnreadCount(0);
    setCursor(undefined);
    setHasMore(false);
    setIsError(false);
  }, []);

  // Reset whenever the wallet changes.
  useEffect(() => {
    if (addressRef.current !== address) {
      addressRef.current = address;
      reset();
    }
  }, [address, reset]);

  const loadPage = useCallback(
    async (pageCursor: string | undefined, append: boolean) => {
      if (!address) return;
      setIsLoading(true);
      setIsError(false);
      try {
        const url = inboxPageUrl(address, {
          cursor: pageCursor,
          limit: pageSize,
        });
        const page: NotificationInboxPage = await fetchInboxPage(url);

        setUnreadCount(page.unread_count);
        setHasMore(page.next_cursor !== null);
        setCursor(page.next_cursor ?? undefined);

        if (append) {
          // Deduplicate: keep existing entries not present in the new page.
          setNotifications((prev) => {
            const existingIds = new Set(page.notifications.map((n) => n.id));
            const merged = [
              ...prev.filter((n) => !existingIds.has(n.id)),
              ...page.notifications,
            ];
            // Sort descending by created_at then id for stable ordering.
            merged.sort(
              (a, b) =>
                new Date(b.created_at).getTime() - new Date(a.created_at).getTime() ||
                b.id - a.id,
            );
            return merged;
          });
        } else {
          setNotifications(page.notifications);
        }
      } catch {
        setIsError(true);
      } finally {
        setIsLoading(false);
      }
    },
    [address, pageSize],
  );

  // Load the first page on mount or address change.
  useEffect(() => {
    if (address) {
      void loadPage(undefined, false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [address]);

  const loadMore = useCallback(async () => {
    if (!hasMore || isLoading) return;
    await loadPage(cursor, true);
  }, [cursor, hasMore, isLoading, loadPage]);

  const markRead = useCallback(
    async (ids?: number[]) => {
      if (!address) return;
      await markNotificationsRead(address, ids);
      // Optimistically update read state in local list.
      setNotifications((prev) =>
        prev.map((n) =>
          !ids || ids.includes(n.id) ? { ...n, read: true } : n,
        ),
      );
      // Refresh unread count from server.
      await loadPage(undefined, false);
    },
    [address, loadPage],
  );

  const refresh = useCallback(async () => {
    reset();
    await loadPage(undefined, false);
  }, [reset, loadPage]);

  return {
    notifications,
    unreadCount,
    isLoading,
    isError,
    hasMore,
    loadMore,
    markRead,
    refresh,
  };
}
