"use client";

/**
 * NotificationInbox (#287)
 *
 * Full durable inbox with cursor-based pagination and read-cursor tracking.
 * Notifications survive browser tab closes and WebSocket disconnects because
 * they are persisted on the backend and fetched on mount/reconnect.
 *
 * Features:
 * - Per-page loading via "Load more" (cursor advances automatically).
 * - Deduplication: reconnecting or loading the same page twice never doubles entries.
 * - Unread badge count from the same consistent snapshot as the visible list.
 * - Per-notification mark-read on click; mark-all-read button in the header.
 * - Each notification retains a link to its source market (pool_id).
 */

import { Bell, RefreshCw } from "lucide-react";
import { cn } from "@/lib/utils";
import { formatUtcDateTime } from "@/lib/date";
import { useNotificationInbox } from "@/lib/hooks/useNotificationInbox";
import { useWalletAddress } from "@/lib/context";
import type { Notification, NotificationType } from "@/lib/api/notifications";

const TYPE_LABEL: Record<NotificationType, string> = {
  pool_ending_soon: "Ending soon",
  pool_resolved: "Resolved",
  claim_expiring: "Claim expiring",
  new_pool_match: "New pool",
};

const TYPE_STYLES: Record<NotificationType, string> = {
  pool_ending_soon: "bg-yellow-500/20 text-yellow-400",
  pool_resolved: "bg-emerald-500/20 text-emerald-400",
  claim_expiring: "bg-red-500/20 text-red-400",
  new_pool_match: "bg-[#37B7C3]/20 text-[#7DE3EC]",
};

interface NotificationInboxProps {
  className?: string;
}

export function NotificationInbox({ className }: NotificationInboxProps) {
  const address = useWalletAddress();
  const {
    notifications,
    unreadCount,
    isLoading,
    isError,
    hasMore,
    loadMore,
    markRead,
    refresh,
  } = useNotificationInbox(address);

  if (!address) {
    return (
      <div className={cn("rounded-xl border border-zinc-800 bg-zinc-900 p-8 text-center", className)}>
        <Bell className="mx-auto mb-3 h-8 w-8 text-zinc-600" aria-hidden="true" />
        <p className="text-sm text-zinc-400">Connect a wallet to view your notifications.</p>
      </div>
    );
  }

  async function handleNotificationClick(notification: Notification) {
    if (!notification.read) {
      await markRead([notification.id]);
    }
  }

  return (
    <div className={cn("rounded-xl border border-zinc-800 bg-zinc-900 overflow-hidden", className)}>
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-zinc-800">
        <div className="flex items-center gap-2">
          <Bell className="h-4 w-4 text-[#37B7C3]" aria-hidden="true" />
          <h2 className="text-sm font-semibold text-white">Notifications</h2>
          {unreadCount > 0 && (
            <span className="flex h-5 min-w-5 items-center justify-center rounded-full bg-[#37B7C3] px-1.5 text-[10px] font-bold text-black">
              {unreadCount > 99 ? "99+" : unreadCount}
            </span>
          )}
        </div>
        <div className="flex items-center gap-3">
          {unreadCount > 0 && (
            <button
              type="button"
              onClick={() => markRead()}
              className="text-xs font-medium text-[#37B7C3] hover:underline"
            >
              Mark all read
            </button>
          )}
          <button
            type="button"
            onClick={refresh}
            aria-label="Refresh notifications"
            className="text-zinc-500 hover:text-zinc-300 transition-colors"
          >
            <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
          </button>
        </div>
      </div>

      {/* Content */}
      {isError ? (
        <div className="p-8 text-center space-y-3">
          <p className="text-sm text-red-400">Failed to load notifications.</p>
          <button
            type="button"
            onClick={refresh}
            className="text-xs font-medium text-[#37B7C3] hover:underline"
          >
            Try again
          </button>
        </div>
      ) : notifications.length === 0 && !isLoading ? (
        <div className="p-8 text-center">
          <Bell className="mx-auto mb-3 h-8 w-8 text-zinc-700" aria-hidden="true" />
          <p className="text-sm text-zinc-500">You&apos;re all caught up.</p>
        </div>
      ) : (
        <ul>
          {notifications.map((notification) => (
            <li key={notification.id}>
              <button
                type="button"
                onClick={() => handleNotificationClick(notification)}
                className={cn(
                  "w-full text-left px-4 py-3 border-b border-zinc-800/50 last:border-0 hover:bg-white/[0.03] transition-colors",
                  !notification.read && "bg-[#37B7C3]/[0.04]",
                )}
              >
                <div className="flex items-center gap-2">
                  <span
                    className={cn(
                      "text-[10px] font-semibold px-1.5 py-0.5 rounded-full",
                      TYPE_STYLES[notification.notif_type],
                    )}
                  >
                    {TYPE_LABEL[notification.notif_type]}
                  </span>
                  {!notification.read && (
                    <span className="h-1.5 w-1.5 rounded-full bg-[#37B7C3]" aria-label="Unread" />
                  )}
                  {notification.pool_id !== null && (
                    <span className="ml-auto text-[10px] text-zinc-600">
                      Market #{notification.pool_id}
                    </span>
                  )}
                </div>
                <p className="text-sm text-white mt-1">{notification.title}</p>
                <p className="text-xs text-zinc-500 mt-0.5">{notification.message}</p>
                <p className="text-[10px] text-zinc-600 mt-1">
                  {formatUtcDateTime(notification.created_at)}
                </p>
              </button>
            </li>
          ))}
        </ul>
      )}

      {/* Load more / loading state */}
      <div className="border-t border-zinc-800/50">
        {isLoading ? (
          <p className="p-4 text-center text-sm text-zinc-500" aria-live="polite">
            Loading…
          </p>
        ) : hasMore ? (
          <button
            type="button"
            onClick={loadMore}
            className="w-full py-3 text-sm text-[#37B7C3] hover:underline transition-colors"
          >
            Load more
          </button>
        ) : notifications.length > 0 ? (
          <p className="p-3 text-center text-xs text-zinc-600">All caught up</p>
        ) : null}
      </div>
    </div>
  );
}
