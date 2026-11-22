/**
 * PrediFi service worker for browser push notifications (#288).
 *
 * Receives push events from the backend and shows a browser notification.
 * Payloads contain only public market status data — no sensitive balance
 * information is ever included (backend enforces this at the envelope level).
 *
 * The notification click handler opens (or focuses) the market detail page
 * when a `pool_id` is present in the payload.
 */

/* global self, clients */

self.addEventListener("push", (event) => {
  if (!event.data) return;

  let title = "PrediFi";
  let body = "A market event occurred.";
  let url = "/";
  let tag = "predifi-market-event";

  try {
    const payload = event.data.json();
    // Payload schema: { title, body, pool_id?, tag? }
    // Sensitive fields (balances, amounts) must never appear here.
    if (typeof payload.title === "string") title = payload.title;
    if (typeof payload.body === "string") body = payload.body;
    if (typeof payload.pool_id === "number") {
      url = `/user/pool-market/${payload.pool_id}`;
      tag = `predifi-market-${payload.pool_id}`;
    }
    if (typeof payload.tag === "string") tag = payload.tag;
  } catch {
    // Malformed payload — show a generic notification.
  }

  event.waitUntil(
    self.registration.showNotification(title, {
      body,
      icon: "/logo.svg",
      badge: "/logo.svg",
      tag,
      // Merge multiple events for the same market into one notification
      // so the user is not overwhelmed.
      renotify: false,
      data: { url },
    }),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();

  const targetUrl = (event.notification.data && event.notification.data.url) || "/";

  event.waitUntil(
    clients
      .matchAll({ type: "window", includeUncontrolled: true })
      .then((windowClients) => {
        // Focus an existing window on the same path, or open a new one.
        for (const client of windowClients) {
          if (client.url.endsWith(targetUrl) && "focus" in client) {
            return client.focus();
          }
        }
        if (clients.openWindow) {
          return clients.openWindow(targetUrl);
        }
      }),
  );
});
