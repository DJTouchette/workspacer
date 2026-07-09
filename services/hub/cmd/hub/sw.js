// Workspacer /m service worker.
//
// Two jobs:
//   1. Background Web Push — the hub sends a push when an agent needs you
//      (approval/question); we surface it as a lock-screen notification even
//      when the PWA is closed. This does NOT keep a socket alive (mobile OSes
//      forbid that) — it wakes us on demand.
//   2. Fast, resilient load — cache the app shell so re-opening the installed
//      app is instant and survives a brief network blip.
//
// The shell is versioned; bump CACHE to ship a new one.

const CACHE = 'wks-m-v1';
const SHELL = ['/m', '/icon-192.png'];

self.addEventListener('install', (event) => {
  self.skipWaiting();
  event.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL).catch(() => {})));
});

self.addEventListener('activate', (event) => {
  event.waitUntil((async () => {
    for (const key of await caches.keys()) if (key !== CACHE) await caches.delete(key);
    await self.clients.claim();
  })());
});

// Network-first for the /m navigation (so a fresh shell wins when online),
// falling back to the cached shell offline. Everything else passes through.
self.addEventListener('fetch', (event) => {
  const req = event.request;
  if (req.method !== 'GET') return;
  const url = new URL(req.url);
  if (url.origin === self.location.origin && url.pathname === '/m') {
    event.respondWith((async () => {
      try {
        const fresh = await fetch(req);
        const c = await caches.open(CACHE);
        c.put('/m', fresh.clone());
        return fresh;
      } catch {
        return (await caches.match('/m')) || Response.error();
      }
    })());
  }
});

// A push arrived from the hub: { title, body, sessionId }. Show it.
self.addEventListener('push', (event) => {
  let data = {};
  try { data = event.data ? event.data.json() : {}; } catch (_) {}
  const title = data.title || 'Workspacer';
  event.waitUntil(self.registration.showNotification(title, {
    body: data.body || 'An agent needs you',
    icon: '/icon-192.png',
    badge: '/icon-192.png',
    tag: data.sessionId || 'workspacer',   // collapse repeats for the same agent
    renotify: true,
    data: { sessionId: data.sessionId || '' },
  }));
});

// Tapping the notification focuses an open /m (deep-linking to the agent via a
// postMessage) or opens a fresh one at /m?agent=<id>.
self.addEventListener('notificationclick', (event) => {
  event.notification.close();
  const sid = (event.notification.data && event.notification.data.sessionId) || '';
  event.waitUntil((async () => {
    const wins = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
    for (const c of wins) {
      if (c.url.includes('/m')) { try { await c.focus(); } catch (_) {} c.postMessage({ type: 'open-agent', sessionId: sid }); return; }
    }
    await self.clients.openWindow(sid ? '/m?agent=' + encodeURIComponent(sid) : '/m');
  })());
});
