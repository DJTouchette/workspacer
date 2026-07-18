---
name: make-workspacer-plugin
description: Scaffold and author a workspacer plugin — a webview or a sidecar — that talks the hub bus (calls capabilities, publishes/consumes events) inside a scoped, fail-closed capability sandbox.
---

# Make a workspacer plugin

A workspacer plugin is a folder with a `plugin.json` manifest that contributes
panes, hotkeys, and settings, and connects to the **hub bus** to call
capabilities and exchange events. There are exactly two kinds:

- **sidecar** — a long-running process (any language; the template is zero-dep
  Node). Marked by a `server` block in the manifest. Runs on the host, gets a
  `HUB_TOKEN` env var, and is ideal for filesystem watching, running commands,
  and background reactions. Example ships a `/health` HTTP endpoint + status pane.
- **webview** — static HTML/JS the hub serves at `/plugins/ui/<id>/`. Marked by a
  `ui` subdir in the manifest (no process). The host injects its scoped token into
  the pane URL as `?busToken=…`. Ideal for dashboards / HUDs / live UI.

A plugin can be both (a `server` that also serves a pane), but start with one.

## 0. Decide the kind

- Need to watch files, run tests/commands, or react in the background? → **sidecar**.
- Purely a visual pane driven by bus events + capability reads? → **webview**.

## 1. Scaffold

Copy the matching template folder into a new plugin dir (one plugin per repo, by
convention):

- sidecar → `templates/sidecar/` (`plugin.json`, `server.js`, `README.md`, `LICENSE`, `.gitignore`)
- webview → `templates/webview/` (`plugin.json`, `ui/index.html`, `README.md`, `LICENSE`, `.gitignore`)

Then edit `plugin.json`. **Do NOT hand-write or commit these loader-owned files**
(they are already in the template `.gitignore`): `.bus-token`, `.settings.json`,
`.install-source`, `.disabled`, plus `node_modules`.

## 2. Fill in the manifest (`plugin.json`)

Schema + validation source of truth: `services/hub/internal/plugin/manifest.go`
(struct `Manifest`, func `Validate`). Rules the loader enforces — get these wrong
and the plugin fails to load:

- `id` **required**, convention `owner.name` (e.g. `djtouchette.cost-hud`). Drives
  the install dir via `sanitizeName`. Make it unique.
- `apiVersion` **required and must be exactly `"1"`** — any other value is a fatal
  load error (`const APIVersion = "1"`).
- `name` — human label.
- `server` = sidecar. `{ command (required if present), args, port, health }`
  (e.g. `"health": "/health"`).
- `ui` = webview. A subdir string (e.g. `"ui"`); served at `/plugins/ui/<id>/`.
  Only that subdir is exposed — the manifest and `.bus-token` stay private.
- `panes[]` — `{ type (required, unique), title, icon, path, scope }`.
  `scope ∈ global | agent | both` (default `both`). A pane needs a `server` or a
  `ui` to be served from, or the manifest is rejected.
- `hotkeys[]` — `{ id, default (e.g. "ctrl+shift+u"), command }` where command is
  `open-pane:<paneType>` or `emit:<eventType>`.
- `settings[]` — `{ key (required, unique), label, type, default, options, help }`.
  `type ∈ boolean | number | string | select`. **`select` must declare a non-empty
  `options[]`** or the manifest is rejected.
- `capabilities[]` — bus methods this plugin may **call**.
- `emits[]` — event types it may **publish**.
- `consumes[]` — event types it may **receive** (subscribe topics; support `ns.*`
  and `*`).
- `provides[]` — capabilities it answers on the bus.
- `install[]` — a one-time argv run in the dir after a GitHub install, e.g.
  `["npm","install"]` or `["go","build","-o","bin"]`. Empty = self-contained.

## 3. The capability sandbox (3 layers, all fail-closed)

Everything is **deny-by-default** and mostly **silent** when denied:

- Calling a capability not in `capabilities[]` → **error**.
- Publishing an event type not in `emits[]` → **dropped silently**.
- Receiving a consumed event not in `consumes[]` → **never delivered**.

**Filesystem-scoped capabilities need the object form.** The methods
`fs.read`, `fs.write`, `fs.listEntries`, `fs.listDir`, `fs.watch`, `fs.unwatch`,
`search.project` (source: `services/hub/internal/capspec/capspec.go`) **must** be
declared as `{ "method": "fs.read", "paths": ["${pluginDir}"] }` — the bare-string
form is rejected, so a plugin can never get unrestricted host FS access.

Path tokens:
- `${pluginDir}` — always bound (the plugin's own install dir).
- `${agentCwd}` — bound **only for per-pane webview tokens**. A sidecar's static
  token can't resolve it (it registers with no roots → "granted with no roots").
  **So sidecars watch the filesystem locally with Node `fs.watch`, not `fs.watch`
  the capability** — see `../workspacer-plugins/test-on-save/server.js`.

Exact params for each capability: grep the method name in
`apps/desktop/src/main/services/hubCapabilities.ts`. Common set: `agents.list`,
`agents.sendMessage`, `notifications.post`. Full set spans `agents.*`, `claude.*`,
`sessions.*`, `git.*`, `analytics.*`, `fs.watch/unwatch`, `search.project`,
`providers.*`, `replay.*`, `terminals.create`.

## 4. The bus protocol

Connect: `ws://127.0.0.1:7895/bus?token=<t>` (the WS query param is `?token=`).

Frames:
- subscribe: `{ op:'subscribe', topics:[...] }` (topics support `ns.*` and `*`)
- call: `{ op:'call', id, method, params }` → `{ op:'result', id, result }` or
  `{ op:'error', id, error }`
- publish: `{ op:'publish', event:{ type, source, data } }`
- inbound event: `{ op:'event', event }`

**Token delivery by context:**
- **sidecar**: token in the `HUB_TOKEN` env var (the template also falls back to a
  `.bus-token` file / `WKS_BUS_TOKEN`). Settings arrive as JSON in `WKS_SETTINGS`.
- **webview**: token injected in the pane URL as `?busToken=…`; read it from
  `location.search`. Build the WS URL from `location.host` (so remote/non-default
  ports work), falling back to `ws://127.0.0.1:7895/bus`.

Both templates already implement: connect, a **reconnect loop**, subscribe to
`consumes`, and `call` / `publish` / `log` helpers.

## 5. Key consumable events

Declare in `consumes[]`, then handle in `onEvent`:

- `agent.state_changed` — `{ sessionId, hookEvent, mode, cwd }`,
  `mode ∈ unknown | input | responding | approval | question | stopped`.
- `agent.snapshot` / `agent.statusline` — per-session snapshot / status line.
- `workflow.completed` / `workflow.failed`.
- `ui.pane.*` / `ui.tab.focused` / `ui.workspace.focused`.
- `fs.changed` — only after a prior `fs.watch` (webview panes; sidecars use local
  `fs.watch`).
- `plugin.loaded` / `plugin.unloaded`.

## 6. Settings

- **Sidecars** read `WKS_SETTINGS` **only at spawn** — a settings change **restarts**
  the sidecar (the host re-Adds it). `SetSettings` is all-or-nothing.
- **Webviews** get settings **live** via `window.__WKS_SETTINGS__` plus a settings
  bus event — no restart.

## 7. Verify before shipping

- **sidecar**: `node --check server.js` must pass, and the process must **start and
  retry** (reconnect loop) when the hub is down — it must not crash. Hit
  `/health` and the status pane.
- **webview**: the HTML parses and there are no obvious JS errors; it shows
  "disconnected" and reconnects when the bus is unreachable.

Use `witness.select` on changed files to get the right test command for repo-side
changes.

## References (go deeper)

- Manifest schema + validation: `services/hub/internal/plugin/manifest.go`
- Path-scoped capability set: `services/hub/internal/capspec/capspec.go`
- Capability params: `apps/desktop/src/main/services/hubCapabilities.ts`
- Sidecar env wiring (`HUB_TOKEN` / `WKS_SETTINGS`): `services/hub/internal/plugin/manager.go`
- Authoring guide: `../workspacer-plugins/IMPLEMENTING.md`
- Live sidecar example: `../workspacer-plugins/test-on-save/server.js`
- Live webview example: `../workspacer-plugins/cost-hud/ui/index.html`
- Hub overview + bus: `services/hub/README.md`
- Rules-engine plugin pattern: `services/hub/docs/rules-engine-plugin.md`
- New landing guide (sibling task): `landing/build-plugin.html`
