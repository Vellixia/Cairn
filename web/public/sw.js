// Cairn Dashboard service worker (v0.5.0 Sprint 20a — PWA).
//
// Cache-first for static assets (the dashboard's own JS / CSS / HTML shell),
// network-first for the `/api/*` surface so the dashboard always shows fresh
// data when online. Falls back to a cached shell + a friendly "offline"
// notice when the network is unreachable.
//
// Also handles Web Push: when the dashboard subscribes to `/api/push/subscribe`,
// the server can `postMessage` a `CAIRN_PUSH` event into this SW, which
// displays a `notification` and forwards the click to `/dashboard/reliability/drift`.

const CACHE_VERSION = "cairn-v1";
const STATIC_ASSETS = ["/", "/dashboard", "/manifest.json"];

self.addEventListener("install", (event) => {
  event.waitUntil(
    (async () => {
      const cache = await caches.open(CACHE_VERSION);
      await cache.addAll(STATIC_ASSETS).catch(() => {
        // Best-effort precache — server may be down at install time.
      });
      self.skipWaiting();
    })(),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const keys = await caches.keys();
      await Promise.all(
        keys.map((k) => (k !== CACHE_VERSION ? caches.delete(k) : null)),
      );
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);

  // API: network-first, cache fallback. Never serve stale API data forever.
  if (url.pathname.startsWith("/api/")) {
    event.respondWith(
      (async () => {
        try {
          const fresh = await fetch(req);
          const cache = await caches.open(CACHE_VERSION);
          cache.put(req, fresh.clone()).catch(() => {});
          return fresh;
        } catch (_e) {
          const cached = await caches.match(req);
          if (cached) return cached;
          return new Response(
            JSON.stringify({ error: "offline", detail: "no cached response" }),
            { status: 503, headers: { "Content-Type": "application/json" } },
          );
        }
      })(),
    );
    return;
  }

  // Static: cache-first, network fallback.
  event.respondWith(
    (async () => {
      const cached = await caches.match(req);
      if (cached) return cached;
      try {
        const fresh = await fetch(req);
        const cache = await caches.open(CACHE_VERSION);
        cache.put(req, fresh.clone()).catch(() => {});
        return fresh;
      } catch (_e) {
        // Last resort: serve the offline shell.
        const shell = await caches.match("/");
        if (shell) return shell;
        return new Response("Offline", { status: 503 });
      }
    })(),
  );
});

self.addEventListener("push", (event) => {
  let payload = { title: "Cairn", body: "Something changed." };
  try {
    if (event.data) payload = event.data.json();
  } catch (_e) {
    // Fall back to the default payload above.
  }
  event.waitUntil(
    self.registration.showNotification(payload.title, {
      body: payload.body,
      icon: "/assets/icon-192.svg",
      badge: "/assets/icon-192.svg",
      data: payload,
      tag: payload.tag || "cairn-default",
    }),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const url = (event.notification.data && event.notification.data.url) || "/dashboard";
  event.waitUntil(
    (async () => {
      const all = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
      for (const c of all) {
        if (c.url.endsWith(url)) {
          c.focus();
          return;
        }
      }
      self.clients.openWindow(url);
    })(),
  );
});
