---
title: Framing a plugin pane on /app is an origin problem, not an X-Frame-Options problem
date: 2026-08-24
confidence: high
suggested_doc: webview-security-hardening
related_paths:
  - apps/desktop/src/renderer/src/lib/guestFrame.ts
  - apps/desktop/src/renderer/src/components/GuestFrame.tsx
  - apps/desktop/src/renderer/src/panes/BrowserPane.tsx
  - services/hub/internal/bus/bus.go
  - services/hub/cmd/hub/main.go
promoted: true
promoted_to: webview-security-hardening
---

# Framing a plugin pane on /app is an origin problem, not an X-Frame-Options problem

## Observation
Nothing in this repo, the hub, or the 20-plugin catalogue sets X-Frame-Options or a frame-ancestors CSP — verified by repo-wide grep and by curl against a live hub (`/plugins/ui/<id>/` and `/app/` both return neither header). All three sandbox variants of a hub-served plugin UI load in an iframe under Chromium. So framing itself needs no header change.

The real fork is the sandbox, and it splits by plugin kind:

  * A webview-only plugin (manifest `ui`, no `server`) is served by the HUB at `/plugins/ui/<id>/`, which is the SAME ORIGIN as `/app`. Framed with `allow-same-origin` its page can read `parent.document`, call `parent.window.electronAPI.*`, and lift the hub's full host token out of sessionStorage — escalating past the scoped per-pane token PluginPane mints. Confirmed in Chromium: `iframe.contentDocument` is readable both with no sandbox and with `allow-scripts allow-same-origin`.
  * A sidecar plugin (`server.port`) is served from `http://127.0.0.1:<port>`, a DIFFERENT origin from /app. `allow-scripts allow-same-origin` there is safe (`parent.document` throws) AND its bus WebSocket connects, because the frame keeps a real loopback origin. Verified live: a sidecar plugin pane on /app renders with `bus: CONNECTED`.

Withholding `allow-same-origin` gives an opaque origin, which is safe but sends `Origin: null` — and `services/hub/internal/bus/bus.go` `originAllowed` refuses that explicitly ("malformed / opaque (\"null\") Origin — fail closed"), because it is the DNS-rebinding guard. Measured: the plugin's /bus upgrade gets a 403 handshake response and the SDK reconnect-loops forever.

Catalogue split at 2026-08-24: 13 of 20 plugins are sidecar (work fully in an iframe), 7 are hub-served `ui` (render statically, no bus).

## Impact
Anyone extending the /app iframe fallback will be tempted to "just add allow-same-origin" when a hub-served plugin's bus link fails, or to make originAllowed accept null. Either one silently widens a boundary on a box that may be internet-facing over Tailscale. Both were declined deliberately.

## Recommendation
Keep `guestFramePolicy()` in apps/desktop/src/renderer/src/lib/guestFrame.ts as the single decision point and keep it origin-driven. If hub-served plugin panes need a live bus in the browser, the only change that loosens nothing is giving them a DISTINCT loopback origin from /app (e.g. serve /app on 127.0.0.1:P and frame plugins at localhost:P — different origins to the browser, both accepted by isLoopbackHost) — a user-facing decision, not a silent fix.
