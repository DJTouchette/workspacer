---
title: Plugin UI in a browser needs a SECOND hub origin; sidecar UIs cannot be path-proxied
date: 2026-08-24
confidence: high
suggested_doc: webview-security-hardening
related_paths:
  - apps/desktop/src/renderer/src/lib/pluginOrigin.ts
  - apps/desktop/src/renderer/src/lib/guestFrame.ts
  - apps/desktop/src/renderer/src/types/plugin.ts
  - services/hub/cmd/hub/main.go
  - services/hub/internal/bus/bus.go
promoted: true
promoted_to: webview-security-hardening
---

# Plugin UI in a browser needs a SECOND hub origin; sidecar UIs cannot be path-proxied

## Observation
Framing plugin UI inside /app has exactly one degree of freedom: the origin it is served from. A hub-served `ui` plugin is same-origin with /app, so it must be framed opaque (allow-same-origin lets it read parent.document and lift the host token from sessionStorage — mutation-verified in Chromium: the frame reported `hostToken:<host token>`), and an opaque document sends `Origin: null`, which bus.go's originAllowed refuses by design. There is no third option: a second origin routed to the same hub (`hub --plugin-origin`, advertised at the public /plugins/origin route, or the free `127.0.0.1` ⇄ `localhost` sibling on loopback) is the only way a browser-framed plugin keeps its bus link, and it loosens nothing — the browser's own same-origin policy becomes the wall. Verified: at a distinct origin the frame connects to /bus, keeps storage, and still reports parentDoc/hostToken/electronAPI BLOCKED, while every capability outside its manifest is refused by the pane token.

Separately: SIDECAR plugin UIs cannot be remoted with a hub path-prefix proxy. Their pages fetch ROOT-relative paths (jira/ui/index.html: `/state`, `/issue/<k>/detail`, `/briefs`, `/projects`; shiplight/ui/widget.html: `STATE_URL = '/state'`), which escape any `/plugins/proxy/<id>/` prefix — they need an origin ROOT, i.e. one origin per plugin. On top of that, a proxy would publish a loopback-confined sidecar's API to the network with no subresource-safe credential (relative fetches carry no token; cookies are unavailable in an opaque frame and third-party-blocked in a cross-origin one).</observation>
<parameter name="impact">Decides how any future remote-plugin work must be shaped, and rules out the obvious-looking hub proxy route. Also: the discriminator for "can I reach a sidecar's loopback port" must be `is the hub endpoint I talk to on my own loopback`, never `platform === 'web'` — desktop remote-client mode is a web backend with a real host platform, and a browser on the hub machine is 'web' but local.

## Recommendation
Reach for a distinct origin (operator-declared or loopback sibling), never for allow-same-origin on a same-origin guest and never for teaching originAllowed to accept `null`. If remote sidecar panes are ever wanted, budget for per-plugin origins plus a credential scheme that survives root-relative subresources.
