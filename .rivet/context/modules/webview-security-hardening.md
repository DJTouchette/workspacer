---
title: Webview attach-time hardening (BrowserPane + PluginPane)
tags: [security, electron, webview, browser-pane, plugin-pane, main-process, origin, blocked]
related_paths:
  - "apps/desktop/src/main/lib/webviewGuard.ts"
  - "apps/desktop/src/main/lib/webviewGuard.test.ts"
  - "apps/desktop/src/main/index.ts"
  - "apps/desktop/src/renderer/src/panes/BrowserPane.tsx"
  - "apps/desktop/src/renderer/src/panes/PluginPane.tsx"
  - "apps/desktop/src/main/services/chromeCookieImport.ts"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# Webview attach-time hardening (BrowserPane + PluginPane)

## Overview

Workspacer's main `BrowserWindow` enables `webviewTag: true` (`apps/desktop/src/main/index.ts`) so two very different features can embed remote pages inside a pane: `BrowserPane` (arbitrary http(s) browsing, the user-facing web browser tab) and plugin panes (loaded from the hub UI origin or a `127.0.0.1` plugin sidecar server, wrapped by `PluginPane`). A bare `<webview>` tag is a privilege-escalation vector — renderer content (including a compromised plugin webview) could otherwise attach a `<webview nodeintegration preload=…>` to reach the main process, or point `src` at `file://` to read the host filesystem. `apps/desktop/src/main/lib/webviewGuard.ts` is the pure-function policy — force-applying safe `webPreferences` and restricting allowed `src` schemes — that `apps/desktop/src/main/index.ts` wires into Electron's `will-attach-webview` / `did-attach-webview` events on every webview attach and every subsequent navigation. The guard shipped 2026-07-05; the address-bar half (`did-start-navigation`, which fires for the embedder-initiated `loadURL` that `will-navigate` never sees) landed 2026-07-30.

## Key modules

- `apps/desktop/src/main/lib/webviewGuard.ts` — the policy itself, split into two pure, unit-testable functions (no Electron `BrowserWindow` needed to test):
  - `applySafeWebviewPreferences(prefs: MutableWebPreferences): void` — mutates the `webPreferences` object handed to `will-attach-webview`: deletes `preload`/`preloadURL`, forces `nodeIntegration = false`, `nodeIntegrationInSubFrames = false`, `contextIsolation = true`. Applied unconditionally regardless of what the `<webview>` tag itself requested.
  - `isWebviewSrcAllowed(src: string | undefined): boolean` — allows `undefined`/`''`/`'about:blank'` (the empty shell a pane then drives via `loadURL()`), and `http:`/`https:` after `new URL(src).protocol` parsing; rejects everything else including `file:`, `chrome:`, `devtools:`, `data:`, and every non-blank `about:` URL (`about:config`, `about:blank#x`, etc.). Fails closed (`return false`) if `new URL(src)` throws on an unparseable string.
  - `MutableWebPreferences` — the mutable subset of Electron's `WebPreferences` interface this module touches.
- `apps/desktop/src/main/index.ts` — the only production caller, inside `createWindow()`:
  - `mainWindow.webContents.on('will-attach-webview', ...)` (line ~303) calls `applySafeWebviewPreferences` unconditionally, then `isWebviewSrcAllowed(params.src)`; if false, logs `[main] blocking <webview> attach with disallowed src: ${params.src}` and calls `event.preventDefault()` to abort the attach entirely.
  - `mainWindow.webContents.on('did-attach-webview', ...)` (line ~310) attaches `will-navigate` and `will-redirect` listeners to the newly-attached guest's `webContents`, re-running the same `isWebviewSrcAllowed` check on every later navigation/redirect (logging `[main] blocking <webview> navigation to disallowed url: ${url}` and `e.preventDefault()` on rejection) — so a `file://` URL typed into the browser bar *after* attach is blocked too, not just the initial `src`.
  - Also owns `webPreferences: { preload, contextIsolation: true, nodeIntegration: false, webviewTag: true }` on the top-level `BrowserWindow` itself (this is the host window's own prefs, separate from what the guard forces onto each `<webview>` guest).
- `apps/desktop/src/main/lib/webviewGuard.test.ts` — pins the policy: strips preload/forces safe defaults even when the tag "requested nothing"; allows `https://google.com`, the plugin sidecar (`http://127.0.0.1:7895/plugins/ui/foo/`), and hub dev origins (`http://localhost:5173`); blocks `file://`, `chrome:`, `devtools:`, `data:`, non-blank `about:` URLs, and unparseable src (`http://[::bad`).
- `apps/desktop/src/renderer/src/panes/BrowserPane.tsx` — the single, shared `<webview>` implementation for **both** arbitrary browsing and plugin/hub-UI panes. Renders `<webview ref=... src={startUrl} partition="persist:browser" allowpopups="true" />`. Owns `normalizeUrl()` (the omnibox parser — UX only, not a security boundary; see Gotchas), the theme-injection bridge (`webviewThemeCSS`/`webviewThemeJS` from `../lib/webviewTheme`, applied only when `appMode` is true), the settings-injection bridge (`webviewSettingsJS` from `../lib/webviewSettings`, applied only when `appMode && pluginId`), and a keyboard-shortcut forwarder (`before-input-event` + a `console-message`-based fallback that injects a `__WKS_KEY__`-prefixed `console.log` listener into the guest page).
- `apps/desktop/src/renderer/src/panes/PluginPane.tsx` — a thin wrapper, not a second webview implementation. For agent-scoped panes (`pluginId` + `cwd` both present) it mints an ephemeral, directory-scoped `busToken` via `window.electronAPI.pluginPaneToken(pluginId, cwd)` (hub round-trip) and swaps it into the URL's `busToken` query param before rendering; revokes the token on unmount via `revokePluginPaneToken`. Falls back to the URL's baked-in static per-plugin token if minting is unavailable (web build, hub momentarily down) or fails — the webview always loads, scoping is best-effort. Ultimately renders `<BrowserPane initialUrl={resolvedUrl} appMode={true} pluginId={pluginId} .../>`, so it goes through the exact same `will-attach-webview`/`will-navigate` guard as general browsing.
- `apps/desktop/src/main/services/chromeCookieImport.ts` — imports the user's local Chrome cookies into the `persist:browser` partition (`PARTITION = 'persist:browser'`) to work around Microsoft/Google's embedded-webview OAuth blocks; shares the same partition that `BrowserPane` and every `PluginPane` webview live in.
- `apps/desktop/src/main/lib/webviewGuard.ts` — the policy and its reasoning in the file header: the threat model (renderer content or a compromised plugin webview escalating via `<webview>`), and why forcing the scheme allowlist is behavior-preserving for the real panes (no legitimate webview uses `file://` or other local schemes).

## Failure modes

- **Attach blocked**: `isWebviewSrcAllowed(params.src)` returns `false` in `will-attach-webview` → `event.preventDefault()` aborts the attach; console warning `[main] blocking <webview> attach with disallowed src: ${params.src}`. The webview element never becomes usable (no `dom-ready`, no content).
- **Navigation blocked**: after a legitimate attach, a subsequent `will-navigate` or `will-redirect` to a disallowed scheme is prevented the same way; console warning `[main] blocking <webview> navigation to disallowed url: ${url}`. The guest stays on its last allowed page.
- **Unparseable `src`**: `isWebviewSrcAllowed` fails closed — `new URL(src)` throwing (malformed string, e.g. `http://[::bad`) returns `false`, not `true`. Any parse ambiguity denies rather than allows.
- **`about:` URLs beyond `about:blank`**: rejected outright (`about:version`, `about:config`, `about:srcdoc`, and even `about:blank#x`/`about:blank?y` — only the exact literal string `'about:blank'` matches the allow-list, not the `about:blank` origin family).
- **PluginPane token mint failure**: if `window.electronAPI.pluginPaneToken` is unavailable (web/browser build) or the hub round-trip fails/rejects, `PluginPane` silently falls back to the URL's static baked-in token rather than blocking — this is a scoping *degradation*, not a webview-attach security failure; the `will-attach-webview` guard still applies to whatever URL is ultimately used.
- **Theme/settings injection failure**: `applyWebviewTheme`/`applyWebviewSettings` in `BrowserPane.tsx` swallow errors from `wv.insertCSS`/`wv.executeJavaScript` (e.g. webview mid-navigation or destroyed) with an empty catch and a comment that the next `dom-ready` will re-apply — not a security issue, just a UX no-op on transient failure.

## Gotchas

- **BrowserPane is the ONE shared `<webview>` implementation for both browsing and plugins.** `PluginPane.tsx` does not render its own `<webview>` — it renders `<BrowserPane appMode={true} .../>`. Any change to `BrowserPane.tsx`'s webview element, event listeners, or the hardcoded `partition="persist:browser"` affects the plugin/hub-UI surface too, not just general web browsing.
- **The renderer's `normalizeUrl()` in `BrowserPane.tsx` is NOT the security boundary — it is UX only.** Its regex `/^(https?|about|file):/i` explicitly lets a `file:` URL typed into the omnibox pass through unchanged into `wv.loadURL()`. The actual block happens downstream, at the Electron main-process level, via the `will-navigate`/`will-redirect` listeners in `apps/desktop/src/main/index.ts` calling `isWebviewSrcAllowed`. Do not "fix" `normalizeUrl` to strip `file:` and assume that closes anything — the enforcement point is `webviewGuard.ts` + `index.ts`, and editing one without the other risks a false sense of security either way.
- **All browsing and plugin webviews share one persistent Electron session partition, `persist:browser`.** This is hardcoded as a JSX prop in `BrowserPane.tsx` (`partition="persist:browser"`) — it is not passed down from `PluginPane` or configurable per-pane. `chromeCookieImport.ts` also imports Chrome cookies into this same partition. Cookies/localStorage/session state persist across restarts and are shared by every browser tab and every plugin pane simultaneously (same-origin policy still applies within the partition, but there is no per-plugin or per-pane session isolation).
- **`applySafeWebviewPreferences` runs unconditionally on every attach, with no allowlist exception** — even a "trusted" hub-UI or sidecar plugin webview gets `preload`/`preloadURL` stripped and `nodeIntegration`/`contextIsolation` forced. There is no mechanism today for a plugin to legitimately request a preload script through this path; if that need arises, it requires a deliberate policy change here, not a one-off bypass.
- **Empty `src` / `about:blank` is allowed at attach time by design**, because panes attach an empty shell and then drive it via `wv.loadURL()`/`navigate()` afterward — that follow-up load is caught by the separate `will-navigate` check, so this is not a bypass, but a future editor must not "simplify" by rejecting empty `src` at attach without also confirming panes don't rely on the shell-then-load pattern.
- **Two-listener design is required, not redundant**: `will-attach-webview` alone would only catch the *initial* `src`; `did-attach-webview` + `will-navigate`/`will-redirect` on the guest's own `webContents` is what stops a later in-page navigation (e.g. a link click, JS redirect, or a user retyping the omnibox) from reaching `file://` after a legitimate attach. Removing either listener reopens part of the original gap.
- **Fan-in is deliberately tiny** (`recon.context` reports `fan_in: 2`, `hotspot_score` ~0.005) — `webviewGuard.ts` has exactly one production caller (`apps/desktop/src/main/index.ts`) plus its test file. Low fan-in here is a *feature*, not a coverage gap: the policy is intentionally centralized to one attach point rather than duplicated per pane type.

## Hand-authored notes (2026-08-24) — framing plugin UI in a BROWSER is an origin problem

Everything above is about the Electron `<webview>`. The `/app` browser mirror has
a different problem with the same panes, and the two are easy to conflate.

- **Nothing sets X-Frame-Options or a `frame-ancestors` CSP** — verified by
  repo-wide grep and by curl against a live hub (`/plugins/ui/<id>/` and `/app/`
  both return neither header), and all three sandbox variants of a hub-served
  plugin UI load in an iframe under Chromium. **So framing itself needs no header
  change.** The real fork is the SANDBOX, and it splits by plugin kind:
  - A **webview-only** plugin (manifest `ui`, no `server`) is served by the HUB at
    `/plugins/ui/<id>/`, which is the **SAME ORIGIN** as `/app`. Framed with
    `allow-same-origin` its page can read `parent.document`, call
    `parent.window.electronAPI.*`, and **lift the hub's full host token out of
    sessionStorage** — escalating past the scoped per-pane token `PluginPane`
    mints. Mutation-verified in Chromium: the frame reported
    `hostToken:<host token>`, and `iframe.contentDocument` was readable both with
    no sandbox and with `allow-scripts allow-same-origin`.
  - A **sidecar** plugin (`server.port`) is served from `http://127.0.0.1:<port>`,
    a DIFFERENT origin from `/app`. `allow-scripts allow-same-origin` there is
    safe (`parent.document` throws) AND its bus WebSocket connects, because the
    frame keeps a real loopback origin. Verified live: a sidecar plugin pane on
    `/app` renders with `bus: CONNECTED`.
- **Withholding `allow-same-origin` is safe but kills the bus.** An opaque origin
  sends `Origin: null`, which `services/hub/internal/bus/bus.go`'s `originAllowed`
  refuses explicitly ("malformed / opaque (\"null\") Origin — fail closed") because
  it is the DNS-rebinding guard. Measured: the plugin's `/bus` upgrade gets a 403
  handshake response and the SDK reconnect-loops forever.
- **So there is exactly one degree of freedom: a SECOND origin.** A distinct
  origin routed to the same hub (`hub --plugin-origin`, advertised at the public
  `/plugins/origin` route, or the free `127.0.0.1` ⇄ `localhost` sibling on
  loopback) is the only way a browser-framed plugin keeps its bus link, and it
  **loosens nothing** — the browser's own same-origin policy becomes the wall.
  Verified: at a distinct origin the frame connects to `/bus`, keeps storage, and
  still reports parentDoc/hostToken/electronAPI BLOCKED, while every capability
  outside its manifest is refused by the pane token.
  **Anyone extending the `/app` iframe fallback will be tempted to "just add
  allow-same-origin" when a hub-served plugin's bus link fails, or to make
  `originAllowed` accept null. Both were declined deliberately** — either silently
  widens a boundary on a box that may be internet-facing over Tailscale. Keep
  `guestFramePolicy()` (`apps/desktop/src/renderer/src/lib/guestFrame.ts`) as the
  single decision point and keep it ORIGIN-driven.
- **SIDECAR plugin UIs cannot be remoted with a hub path-prefix proxy.** Their
  pages fetch ROOT-relative paths (`jira/ui/index.html`: `/state`,
  `/issue/<k>/detail`, `/briefs`, `/projects`; `shiplight/ui/widget.html`:
  `STATE_URL = '/state'`) which escape any `/plugins/proxy/<id>/` prefix — they
  need an origin ROOT, i.e. **one origin per plugin**. On top of that, a proxy
  would publish a loopback-confined sidecar's API to the network with no
  subresource-safe credential (relative fetches carry no token; cookies are
  unavailable in an opaque frame and third-party-blocked in a cross-origin one).
  If remote sidecar panes are ever wanted, budget for per-plugin origins plus a
  credential scheme that survives root-relative subresources.
- **The discriminator for "can I reach a sidecar's loopback port" must be *is the
  hub endpoint I talk to on my own loopback*, never `platform === 'web'`** —
  desktop remote-client mode is a web backend with a real host platform, and a
  browser on the hub machine is `'web'` but local.
- *(Catalogue split at 2026-08-24: 13 of 20 plugins are sidecar and work fully in
  an iframe; 7 are hub-served `ui` and render statically with no bus.)*

## Hand-authored notes (2026-08-24) — a derived-path guard whose ROOT is the caller's own cwd

`library.list`/`save`/`remove` guard the caller's `cwd` and then guard the
DERIVED item path (`<cwd>/.workspacer/library/<slug>.md`,
`<cwd>/.claude/skills/<id>/SKILL.md`) against
`libraryItemRoots(canonicalCwd)` = `[<configDir>/library, canonicalCwd]`. **That
second guard is only as narrow as the cwd the caller named** — and `library.list`
checks its cwd against the BROWSE roots (workspace + the whole home tree),
because the New Agent dialog lists a directory no agent runs in yet.

So a caller may name `$HOME`, the item roots become the whole home tree, and a
`$HOME/.workspacer/library/a.md -> $HOME/.ssh/id_rsa` symlink canonicalizes
inside the root, passes, and comes back as an item BODY — while `fs.read` of the
identical path is refused for the same caller. The brain
(`services/hub/cmd/brain/library.go`) had closed this with a second, **LEXICAL**
requirement (`libraryItemDirs` + `containsPath`) that the RESOLVED file sit in a
directory a library item actually lives in; `hubCapabilities.ts` shipped without
it, so the two providers disagreed about the same call — and the wide one is what
`DELEGATE_CATALOG_TO_BRAIN=false` puts back on the bus. Fixed 2026-08-24
(3158c6f2), pinned by a `libraryItemDirs` block in
`contracts/path-containment-cases.json` with two loaders. **The second gate must
stay LEXICAL:** canonicalizing the two cwd-derived directories resolves the very
link it exists to see.

**The general lesson beyond `library.*`: when a derived-path guard's ROOT is
itself derived from the caller's directory, the guard is self-fulfilling for any
method whose cwd may be a wide root. Ask what the WIDEST cwd that method accepts
is, not what a typical one is.** Any new capability that composes a
provider-chosen path under a caller-supplied directory needs BOTH halves —
containment against roots, and a requirement about where the resolved file may
live — with cases in `contracts/path-containment-cases.json`
(`methods.derivedRootSet` for the first, the `libraryItemDirs` pattern for the
second) rather than one-off tests.
