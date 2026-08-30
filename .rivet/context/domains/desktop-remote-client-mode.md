---
title: Desktop Remote-Client Mode
tags: [remote, desktop, electron, hub-bus, tailscale, client-mode]
related_paths:
  - "apps/desktop/src/main/services/remoteServer.ts"
  - "apps/desktop/src/main/services/remoteServer.test.ts"
  - "apps/desktop/src/main/services/tailscaleServe.ts"
  - "apps/desktop/src/main/index.ts"
  - "apps/desktop/src/main/ipc.ts"
  - "apps/desktop/src/main/preload.ts"
  - "apps/desktop/src/main/services/hubDaemon.ts"
  - "apps/desktop/src/renderer/src/backend/install.ts"
  - "apps/desktop/src/renderer/src/backend/install.test.ts"
  - "apps/desktop/src/renderer/src/backend/remoteBackend.ts"
  - "apps/desktop/src/renderer/src/backend/webBackend.ts"
  - "apps/desktop/src/renderer/src/components/RemoteShareDialog.tsx"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# Desktop Remote-Client Mode

## Overview
Desktop remote-client mode is a startup-time actor flip: the Electron shell stops being a *host* that spawns claudemon/hub/brain locally and instead becomes a pure *client* of someone else's `workspacer serve` (typically another machine on the same Tailscale tailnet). This is the mirror image of `tailscaleServe.ts` (this desktop sharing itself OUT to a phone) — here the desktop points itself AT another server. The user sets a target URL + bearer token via the "Advanced: connect to another Workspacer server" panel inside `RemoteShareDialog.tsx`; that persists to a sidecar `remote-server.json` file, and the app relaunches. On the next boot, `apps/desktop/src/main/index.ts` reads that file *before* the daemon-spawn branch and, if set, skips claudemon/hub/brain entirely — the renderer instead boots the same web (bus) backend a browser would get at the remote server's `/app` URL, just running inside the Electron shell. This exists so a lightweight/travel machine (or one that just doesn't want two daemon stacks running) can drive agents that live entirely on another host, with the desktop acting as nothing more than a themed browser window plus a handful of host-shell affordances (window chrome, external links, relaunch).

## Key modules
- `apps/desktop/src/main/services/remoteServer.ts` — persistence + normalization: `RemoteServerSetting` (raw `{url, token}`) → `ResolvedRemoteServer` (`{httpUrl, busUrl, token}`) via `normalizeRemoteServerUrl()`; reads/writes `<config>/remote-server.json` (mode `0600`); exposes `getRemoteServer()` (cached, invalidated by `setRemoteServer`), `isRemoteClientMode()`, `setRemoteServer()` (throws on unparseable URL — fails closed, never persists garbage).
- `apps/desktop/src/main/index.ts` (~line 335) — the fork point: `const remoteServer = getRemoteServer(); if (remoteServer) { /* log + do nothing local */ } else { startClaudemon()... }`. In remote-client mode, none of claudemon, `runClaudemonInit`, the hook/statusline/conversation/event bridges, hub, `startHubClient`, or the MCP facade are started.
- `apps/desktop/src/main/ipc.ts` (`IPC.HUB_SET_REMOTE_SERVER` handler, ~line 316; `IPC.HUB_GET_REMOTE_INFO` at ~line 305; `IPC.APP_RELAUNCH` at ~line 329) — IPC surface: persist/clear the setting (returns `{ok, error?}`), read `getRemoteShareInfo()` (includes `remoteClient`), and force `app.relaunch(); app.quit()`.
- `apps/desktop/src/main/services/hubDaemon.ts` (`getRemoteShareInfo`, ~line 217–248) — bundles `remoteClient: getRemoteServer()` into the same info blob the Remote Share dialog uses for phone-sharing status, so one IPC round trip answers "am I hosting?" and "am I a client?".
- `apps/desktop/src/renderer/src/backend/install.ts` — `installBackend()` runs at renderer boot (awaited in `main.tsx`); `selectBackendMode(info)` picks `'remote' | 'bridged' | 'ipc'`. Remote wins outright whenever `info.remoteClient?.busUrl` is set — checked *before* the `WORKSPACER_DESKTOP_DIRECT` kill switch and before the local-bus-reachability check, because in this mode there is no local bus or IPC data path to fall back to.
- `apps/desktop/src/renderer/src/backend/remoteBackend.ts` — `createRemoteBackend(ipc, token, busUrl)`: builds the client backend as `createWebBackend(token, busUrl)` dialed at the REMOTE hub, then re-binds a small `REMOTE_HOST_ONLY` allowlist (`setTitleBarOverlay`, `onBeforeQuit`, `onSystemNotice`, `openExternalUrl`, `openLogsFolder`, `getRemoteInfo`, `setRemoteServer`, `appRelaunch`) back onto the real preload IPC object. Everything else — including PTY/terminal I/O — rides the bus as `pty.bytes.*` events exactly like a browser client; `api.platform` is overridden back to the real host platform (`ipc.platform`) so native-only UI chrome (e.g. Windows titlebar overlay) still gates correctly even though the rest of the API is web-backed.
- `apps/desktop/src/renderer/src/components/RemoteShareDialog.tsx` (`RemoteClientSection`, ~line 285) — the UI: URL + token inputs, `apply()` calls `setRemoteServer` then `appRelaunch()`; when `info.remoteClient` is truthy the dialog hides the local phone-sharing controls entirely (comment: "Client mode: this app IS the remote — the local sharing controls are moot, no local hub is running") and shows only Disconnect. The dialog takes an `initialSection?: 'share' | 'connect'` prop: the palette's **"Connect to Server…"** entry (distinct `onConnectServer` callback, `Server` icon) opens it as `'connect'`, which force-opens the `<details>` and focuses/scrolls to the URL input, while "Remote Control…" opens `'share'`. `App.tsx`'s `showRemote` state is therefore `false | 'share' | 'connect'`, not a boolean. The disclosure is no longer labelled "Advanced:" — the flow is a first-class entry point, not a power-user footnote.
- `apps/desktop/src/main/services/tailscaleServe.ts` — unrelated to client mode directly, but the sibling "host" mechanism: shells out to the `tailscale` CLI to front the local hub with HTTPS for phone PWA installs/push. Useful context for how the *other* end of a remote-client connection (the actual `workspacer serve` host) typically gets its reachable address.

## Failure modes
- **Unparseable URL never persists**: `setRemoteServer()` calls `normalizeRemoteServerUrl()` and throws `unrecognized server address: <input>` if it returns null; the IPC handler in `ipc.ts` catches this and returns `{ok: false, error}` so the dialog can show it inline rather than bricking the next launch with a target the app can't dial.
- **`normalizeRemoteServerUrl` accepts**: bare `host` (→ `http://host:7895`), `host:port` (explicit port kept), full `http(s)://`/`ws(s)://` URLs (scheme's own default port kept when none given, e.g. `https://node.ts.net` stays on 443), and pasted `ws://…/bus` URLs. It **rejects** empty/whitespace input, non-`http/https/ws/wss` schemes (e.g. `ftp://`), and malformed URLs like `http://`. IPv6 hosts get bracket-normalized.
- **Corrupt/missing `remote-server.json`**: `getRemoteServer()` swallows any read/parse error and returns `null` (comment: "absent or unreadable → local mode") — a broken sidecar file silently falls back to normal local-daemon boot rather than crashing startup.
- **Change requires a full relaunch**: the daemon-vs-remote decision is made once at `main` process startup (`index.ts`); there is no live-switch. Both connect and disconnect flows in `RemoteShareDialog.tsx` call `window.electronAPI.appRelaunch?.()` after a successful `setRemoteServer` call. If `appRelaunch` silently fails/is unavailable, the setting is persisted but the UI is stuck in the old mode until the user manually restarts.
- **`getRemoteInfo()` IPC failure at renderer boot** (hardened 2026-07-27): `installBackend()` now retries via `getRemoteInfoWithRetry()` (backoff `[50,150,300,600,1000]`ms) before giving up, so a transient failure — most realistically the renderer beating IPC handler registration — no longer strands a remote-client-configured user on a non-functional local IPC backend. Exhausting the backoff still leaves the IPC backend (nothing better is knowable) but logs `console.error` naming the consequence. Complementary main-side fix: `getRemoteShareInfo()` reads `remoteClient` FIRST and wraps its `probeHealth`/`fs.existsSync` work in try/catch, so the transport-deciding field can never be lost to a failure in the advisory QR/web-app fields computed around it.
- **No PTY/IPC fallback in remote mode**: `remoteBackend.ts`'s own comment is explicit — the local-terminal slice must NOT fall back to preload IPC in this mode because there is no local claudemon to serve it; if a `REMOTE_HOST_ONLY`-eligible method is missing from that allowlist, calls to it go to the web-bus backend instead, which either 404s or is simply undefined on `window.electronAPI` (`webBackend.ts`'s method set), producing silent breakage rather than a clear error.

## Gotchas
- **The sidecar file is deliberately outside config.yaml.** `remote-server.json` (mode `0600`, holds the bearer token) is read in `main` before any config plumbing is up — same pattern as the `remote-share-enabled` flag file used by `hubDaemon.ts` for the host-side sharing toggle. Do not fold this into `config.yaml`'s two-writer (TS+Go) system; it needs to be readable standalone, early, and file-permission-guarded because it's a credential.
- **Remote-client mode overrides the desktop-bus kill switch.** `selectBackendMode()` in `install.ts` checks `info.remoteClient?.busUrl` *first*, ahead of the `WORKSPACER_DESKTOP_DIRECT` env-var kill switch (`desktopBus === false`). Setting that env var will NOT force pure-IPC while a remote server is configured — the only way back to local/IPC mode is disconnecting via the dialog (which clears the setting and relaunches). This ordering is intentional (there is nothing local to fall back to) but is easy to assume is a bug when debugging the kill switch.
- **`REMOTE_HOST_ONLY` in `remoteBackend.ts` is an allowlist you must maintain.** Any new `window.electronAPI` method that is inherently host-machine-specific (native dialogs, window chrome, filesystem paths local to this Electron process) must be added to that array or it will silently be served by the remote web-bus backend instead of real IPC, likely breaking in ways that only show up when someone actually runs remote-client mode (an easy-to-miss test path).
- **`api.platform` is force-overridden back to the real host platform** even though everything else in remote-client mode is the web backend (which reports `'web'`). This is so native-only UI (e.g. Windows titlebar overlay gating) still renders correctly; if you add new platform-gated UI, remember remote-client mode still reports the true OS, not `'web'`.
- **This is the mirror of, not the same as, phone/mobile remote sharing** — see the existing `.rivet/context/domains/remote-mobile.md` doc, which covers the hub's `/remote` and `/m` PWA client surfaces (phone-facing, hub as host) plus a brief one-paragraph mention of this file. `desktop-remote-client-mode` is specifically about the **Electron desktop app acting as the client** of someone else's server — a different actor and code path (`main/index.ts` daemon-skip + `install.ts`/`remoteBackend.ts` transport selection) than the hub's outbound phone-sharing story. `RemoteShareDialog.tsx` hosts UI for both concerns in one component but they are functionally independent: `info.remoteClient` (this domain, client mode) vs. `info.enabled`/`hubAdopted` (phone-sharing / host mode, remote-mobile domain).
- **Adopted daemons vs. remote-client mode are different "not fully local" states.** `hubAdopted`/`claudemonAdopted` (also surfaced in `getRemoteShareInfo()`) mean this desktop is using daemons from a `workspacer serve` running on the *same* machine (still local, just not spawned by this Electron instance — see the `AdoptedNote` in `RemoteShareDialog.tsx`). Remote-client mode (`remoteClient`) means the daemons are on a genuinely different machine and none run locally at all. The dialog's `!info.remoteClient` guard is what keeps these two states from being conflated in the UI.
- **`selectBackendMode` and `normalizeRemoteServerUrl` are pure and unit-tested in isolation** (`install.test.ts`, `remoteServer.test.ts`) — prefer extending those tables over hand-testing the full relaunch flow when changing URL-parsing or transport-selection logic.

## Hand-authored notes (2026-08-27/28) — blocking UI and action affordances must not assume the remote is up

- **First-run welcome must not depend on remote CONFIG WRITES.** In
  desktop remote-client mode, renderer config reads and writes go through the
  remote hub. If that configured server is offline, `ConfigContext` falls back to
  `DEFAULT_CONFIG` and `config.save` returns the previous snapshot after warning —
  so `onboardingDismissed` **cannot change**. A first-run welcome gate that
  depends only on persisted config therefore keeps remounting and blocks access to
  Settings / Connect to Server: remote-client users pointed at a down Fly/ORD node
  are trapped behind the welcome modal exactly when they need local settings
  controls to recover. Keep a local, in-session dismissal path for any blocking
  UI, or route recovery settings through host-local APIs instead of remote config
  calls.
- **An action affordance can contradict a correctly rendered status — audit the
  affordance per state, not just its label.** Knowing a node is `stopping` in
  presentation state is not enough: every WAKE affordance must refuse a wake while
  the hub is draining it. The desktop/`/app` `wakeAffordance()` handled
  `available`, `waking`/`pending`, `!wakeable` and `!canWake`, then fell through
  to an **enabled Connect action for `stopping`** — and `RemoteNodesBar.tsx` calls
  that helper with no additional gating. Its sibling `sleepAffordance()` already
  had the mirrored stopping guard, which is what made the omission easy to
  overlook. The hub intentionally ACCEPTS a wake during drain
  (`services/hub/internal/nodes/wake.go` clears `stopping` and increments the
  generation), so this is UX rather than an authorization failure: **one click can
  silently cancel a user-requested shutdown and restart billing.** The
  authoritative hub state must be checked BEFORE an optimistic local `pending`
  flag. `/m` already put its `stopping` branch first; the TUI had the same gap
  (`NodeState::Stopping`, fixed on `wks/tui-handle-the-stopping-state`,
  `05cfa350`); the desktop counterpart was fixed on
  `wks/desktop-m-refuse-wake-while-stopping` (`35fa9fd4`).
  **Whenever a new node state is introduced, keep an explicit state/action parity
  matrix across `wakeAffordance`, `sleepAffordance`, `/m`'s button chain and the
  TUI equivalent.** Stale cross-client copy compounds this: claims that there is
  no stop verb, or that a failed wake necessarily leaves billing running, no
  longer match `nodes.sleep` / `stopAfterFailedWake`.
