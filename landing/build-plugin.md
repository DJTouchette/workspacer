# How to build a work{spacer} plugin

> HTML version: build-plugin.html

## The two kinds

A plugin extends work{spacer} with automations, capabilities, tools for agents, and panes. It's a folder with a `plugin.json` manifest at its root that declares what it contributes. The hub reads `<plugins-dir>/<name>/plugin.json`, validates it, starts whatever it needs, and announces the contribution on the bus (`plugin.loaded`). This page is the hands-on how-to; the shorter tour lives on the [build & internals](build.html#plugins) page.

### where a plugin shows up

Read this before you choose a shape. The hub is the same Rust code whether it runs inside the native app, under `workspacer serve`, or beside the Electron app, so everything a plugin does *on the bus* works in all three. What differs is who can draw your UI.

- **Everywhere:** a sidecar process (`server`), the bus methods you answer (`provides`), the events you publish and consume, your stored `settings`, and the MCP `tools` you hand to agents. Every agent the hub starts gets your tools while the plugin is enabled, whichever window started it.
- **Only where the web renderer runs:** panes (`ui`, `panes`), `widgets`, `hotkeys`, and the screens that manage plugins and edit their settings. That renderer is the Electron app, and the same code served to a browser at `/app` by a `workspacer serve` that has the web bundle beside it (the release server bundle does).
- **Not in the native app:** it has no plugin renderer and no plugin manager. Its hub still loads your plugin, runs its sidecar and gives its tools to agents, but a pane has nowhere to open; a request to open one is answered with a notice that the native client cannot render it.

So for the native app, build a sidecar that contributes `tools` or automation. Build a pane when your users run the web app; the Electron app also renders panes, but it is deprecated and v0.170.0 was its final release.

Every plugin is one of two shapes, and it's the first decision you make. The split is exactly the `server` vs `ui` field in the manifest.

### webview plugin (`ui`)

Set `ui` to a subdirectory of static assets and **omit `server`**. There is no process to run: the hub serves your files at `/plugins/ui/<id>/`, and your page opens as an origin-isolated pane in the Electron app or the web app. The webview uses a **per-plugin identity token** (`?busToken=…`). Enabling the plugin is the trust decision: manifest method/path declarations do not restrict its bus or filesystem access. Browser origin/webview isolation still prevents direct access to the host app document.

- Reach for it when your plugin *is* a UI: a dashboard, a panel, an editor, a rule editor.
- Any language that compiles to static HTML/JS/CSS. No build step needed if you ship plain files.
- Bundled examples (in `plugins/examples/`): the `editor` and the `transcript-timeline` replay pane.

### sidecar plugin (`server`)

Set `server.command` to a long-lived process the hub spawns and supervises (spawn → health-poll → restart-on-crash, SIGTERM then SIGKILL on stop). It's a polyglot sidecar: **any language**. It connects to the bus itself, so it can run *always-on with no pane at all* (an automation), `provide` capabilities other clients call, and/or serve its own webview panes from its port (the host loads `http://127.0.0.1:<port><path>` directly in the pane's webview).

- Reach for it when your plugin *does* something in the background: reacts to events, calls out to another service, answers capabilities for the rest of the fleet, or gives agents a tool. This is the shape that works under the native app.
- Give it a `port` + `health` path so the supervisor can health-check it and surface `sidecar.*` status colors.
- Examples: the bundled `clock-plugin` (the minimal sidecar demo) and the catalog's automations: `policy-approver`, `fleet-guardian`, `test-on-save`, `slack-bridge`.

A sidecar can serve panes too, so the two are not exclusive in spirit, but a single manifest sets *either* `server` *or* `ui` as the way its panes are served. If it declares panes with neither, the loader rejects it (the webview would have no URL to load).

## Your first plugin

The fastest path is a **webview** plugin that lists the running agents. Three files, no build step.

### 1. the folder

A plugin is a folder with a `plugin.json` at its root and, for a webview, a subdir of static assets:

```
my-hello/
  plugin.json
  ui/
    index.html
```

### 2. the manifest

A minimal webview manifest. `apiVersion` MUST be `"1"` (the loader rejects anything else), `ui: "ui"` makes it webview-only, and `capabilities` documents the methods you expect to use:

```json
{
  "id": "example.hello",
  "name": "Hello",
  "apiVersion": "1",
  "version": "1.0.0",
  "ui": "ui",
  "panes": [
    { "type": "example.hello", "title": "Hello", "icon": "👋", "scope": "both" }
  ],
  "hotkeys": [
    { "id": "open-hello", "default": "ctrl+shift+h", "command": "open-pane:example.hello" }
  ],
  "capabilities": ["agents.list"]
}
```

`panes` contributes one pane type; `hotkeys` binds a key to open it; `capabilities` documents the verb this plugin expects to use. Keep declarations accurate for discovery and review, but do not treat them as an enforcement boundary.

### 3. the page

The host **auto-injects the Plugin SDK** into your served HTML, so `window.workspacer` is just there, with no bus WebSocket to hand-roll. Await `ready`, then `call` a capability, subscribe with `on`, `publish`, and read live `settings`. The whole client is a few lines:

```js
await window.workspacer.ready;                     // resolves when connected

// receive events (declare expected types in "consumes" for discovery)
window.workspacer.on('agent.state_changed', (data) => {
  console.log('state', data.sessionId, data.mode);
});

// call a host capability
const agents = await window.workspacer.call('agents.list');
document.body.textContent = `${agents.length} agent(s) running`;

// publish an ordinary event (host-owned topics remain protected)
window.workspacer.publish('command.focus_agent', { sessionId: agents[0]?.sessionId });

// typed settings, delivered live
let settings = window.workspacer.settings;
window.workspacer.onSettings((next) => { settings = next; });
```

> **Plugin SDK.** The hub serves `/plugins/sdk.js` and injects it (plus `window.__WKS_PLUGIN_ID__` and `window.__WKS_SETTINGS__`) into every webview's HTML, so `window.workspacer` is present with no setup. The SDK provides bus calls, subscriptions, `provide` for answering methods, connection state (a live `.connected` boolean and `onStatus(connected => …)`, which fires on every drop and reconnect), and the plugin's authenticated identity (`.token` and `.url`, if you need the raw connection). Manifest declarations aid discovery and routing but do not sandbox enabled plugin code; sidecars run as the Workspacer user. Webview origin/CSP isolation still protects the app document.

Style the page with the injected `--wks-*` theme tokens (with fallbacks, e.g. `background: var(--wks-bg-base, #1a1a1a)`) so it matches the app. The host re-injects them on every live theme switch, and also exposes `window.__WKS_THEME__` + a `wks-theme` event for canvas UIs.

### 4. load it

Drop `my-hello/` into the hub's plugins directory: `<configDir>/plugins` for `workspacer serve` and the Electron app, or whatever you passed to `--plugins-dir`. The hub loads every folder that has a `plugin.json` when it starts, validates the manifest, and emits `plugin.loaded`. This one is a pane, so you need a client that draws panes: in the Electron app or the web app the renderer registers your pane and hotkey, and you open it with `Ctrl+Shift+H` or from the command palette.

The native app keeps its own plugins directory, `config/plugins` inside its data folder (`Workspacer Native` in your platform's local-data directory; builds before the rename called it `Workspacer Native Rust Preview`, and the app renames it on first launch). It has no plugin manager, so installing there is by hand: copy the plugin folder in, already built, and restart the app. An empty file named `.disabled` in the folder switches a plugin off. This hello pane would load there and have nowhere to open, which is why the rest of this section turns it into a sidecar.

### make it a sidecar instead

To make this a **sidecar**, drop `ui`, add a `server` block pointing at your process (and an `install` build step if it needs compiling), and connect to the bus from that process. A sidecar has no injected SDK; it speaks the bus frames directly. The hub starts it with three environment variables: `HUB_TOKEN` (its token), `HUB_URL` (the bus address, e.g. `ws://127.0.0.1:7895/bus`) and `WKS_SETTINGS` (its merged settings, as JSON). Read `HUB_URL` rather than assuming a port: `workspacer serve` and the Electron app default to `7895`, but the native app's hub listens on a port of its own. A tiny Node sidecar that watches for agents that need you and posts a notification, raw frames and all:

```js
// server.js — a headless sidecar (no pane at all)
const WebSocket = require('ws');
const token = process.env.HUB_TOKEN || '';
const url   = process.env.HUB_URL || 'ws://127.0.0.1:7895/bus';
const ws = new WebSocket(`${url}?token=${encodeURIComponent(token)}`);

ws.on('open', () => {
  // declared in "consumes"
  ws.send(JSON.stringify({ op: 'subscribe', topics: ['agent.snapshot'] }));
});
ws.on('message', (buf) => {
  const f = JSON.parse(buf.toString());
  if (f.op !== 'event') return;
  const a = f.event.data || {};
  if (a.pendingApproval) {
    // declared in "capabilities"
    ws.send(JSON.stringify({ op: 'call', id: 'n' + Date.now(), method: 'notifications.post',
      params: { title: 'Agent needs you', body: a.sessionId } }));
  }
});
```

And the matching sidecar manifest:

```json
{
  "id": "example.watcher",
  "name": "Needs-You Watcher",
  "apiVersion": "1",
  "server": { "command": "node", "args": ["server.js"], "port": 8140, "health": "/health" },
  "consumes": ["agent.snapshot"],
  "capabilities": ["notifications.post"],
  "install": ["npm", "install"]
}
```

A sidecar with no `panes` and no `ui` is a pure automation: it never shows a window, it just reacts on the bus. If you give it a `port` + `health`, the supervisor health-checks it, publishes its state as `sidecar.*` events, and the Electron app's Plugins Manager colors it accordingly.

### which Node your sidecar runs on

A `"command": "node"` sidecar runs on whichever Node its host gives it, and the hosts differ.

- **The native app and `workspacer serve`** ship no runtime. Bare `node` is the user's own Node from their `PATH`, and a machine without one cannot start your sidecar. `workspacer serve --sidecar-node <path>` (or `WORKSPACER_SIDECAR_NODE`) pins a specific binary.
- **The Electron app** re-points bare `node` at its own bundled runtime (Electron running as Node), so there your sidecar gets a known-modern Node even on a machine with none installed.

Only a bare `node` / `node.exe` command is ever redirected. A prebuilt per-platform binary (`./bin/${os}-${arch}/server${exe}`) is always launched exactly as written, and `"/usr/bin/node"` is treated as a path to a specific binary rather than as "give me Node". So don't assume a version you were not given: the `wks.js` twin below needs Node 22 or newer for the global `WebSocket`, and your README should say so.

Your **`install` step has it worse.** No host ships a package manager, so an `"install": ["npm", "install"]` runs the user's own npm off their `PATH` and fails outright on a machine that hasn't got one. Two things follow:

- **Prefer zero dependencies.** A sidecar that vendors `wks.js` and sticks to built-in `WebSocket` / `fs` / `http` needs no `install` step at all, and so installs cleanly for every user. The `require('ws')` in the example above is what forces that `npm install`; drop it and the `install` line goes away with it.
- **If you must build, depend on nothing the user has to have.** Something like `["go", "build", "-o", "server", "."]`, or just commit a prebuilt binary, which also removes the question of which Node. Under the Electron app, note that an `npm install` builds against the *system* Node while your sidecar runs on the *bundled* one, so a native module can end up compiled for a different ABI than the Node that later loads it; the failure surfaces at `require` time, a long way from the cause. An `install` step whose command is plain `node` (e.g. `["node", "build.js"]`) gets the same runtime as the sidecar and doesn't have this problem.

### the Node twin (`wks.js`)

Sidecars have no injected SDK, but you can vendor one file that gives you the same surface as `window.workspacer`. Drop this zero-dependency `wks.js` next to your `server.js` (it uses only Node >=22 built-ins: the global `WebSocket`, `fs`, `path`), then `require('./wks.js')`. It reads the token from `HUB_TOKEN` (falling back to `WKS_BUS_TOKEN` then a `.bus-token` file), connects to `HUB_URL` (falling back to `ws://127.0.0.1:7895/bus`), and reconnects on drop:

```js
// wks.js — a zero-dependency hub-bus client for a sidecar (Node >=22, built-in WebSocket).
// Vendor this file next to server.js and require it: const { connect } = require('./wks.js');
const fs = require('fs');
const path = require('path');

function readToken() {
  if (process.env.HUB_TOKEN) return process.env.HUB_TOKEN;
  if (process.env.WKS_BUS_TOKEN) return process.env.WKS_BUS_TOKEN;
  try {
    return fs.readFileSync(path.join(__dirname, '.bus-token'), 'utf8').trim();
  } catch {
    return '';
  }
}

function readSettings() {
  try {
    return JSON.parse(fs.readFileSync(path.join(__dirname, '.settings.json'), 'utf8'));
  } catch {
    return {};
  }
}

// connect() -> { ready, connected, call, publish, provide, on, onStatus, settings } — mirrors window.workspacer.
function connect(opts = {}) {
  const url = opts.url || process.env.HUB_URL || 'ws://127.0.0.1:7895/bus';
  const source = opts.source || 'sidecar';
  const listeners = new Map(); // type -> Set(cb)
  const pending = new Map(); // id -> { resolve, reject }
  const statusListeners = new Set(); // cb(connected)
  const providers = new Map(); // method -> handler(params)
  let ws = null;
  let seq = 1;
  let connected = false;
  let settings = readSettings();
  let markReady;
  const ready = new Promise((r) => {
    markReady = r;
  });

  const deliver = (type, data, event) => {
    for (const key of [type, '*']) {
      const set = listeners.get(key);
      if (set) for (const cb of set) try { cb(data, event); } catch {}
    }
  };

  const fireStatus = (c) => {
    for (const cb of statusListeners) try { cb(c); } catch {}
  };

  const open = () => {
    ws = new WebSocket(`${url}?token=${encodeURIComponent(readToken())}`);
    ws.addEventListener('open', () => {
      connected = true;
      ws.send(JSON.stringify({ op: 'subscribe', topics: ['*'] }));
      // re-announce the methods we answer on every (re)connect
      if (providers.size) ws.send(JSON.stringify({ op: 'register', methods: [...providers.keys()] }));
      markReady();
      fireStatus(true);
    });
    ws.addEventListener('message', (ev) => {
      let f;
      try {
        f = JSON.parse(typeof ev.data === 'string' ? ev.data : ev.data.toString());
      } catch {
        return;
      }
      if (f.op === 'event' && f.event) {
        if (f.event.type === 'plugin.settings.changed' && f.event.data) settings = f.event.data;
        deliver(f.event.type, f.event.data, f.event);
      } else if (f.op === 'result' && pending.has(f.id)) {
        pending.get(f.id).resolve(f.result);
        pending.delete(f.id);
      } else if (f.op === 'error' && pending.has(f.id)) {
        pending.get(f.id).reject(new Error(f.error || 'call failed'));
        pending.delete(f.id);
      } else if (f.op === 'call' && providers.has(f.method)) {
        // someone is calling a method we provide: answer with the same id
        Promise.resolve()
          .then(() => providers.get(f.method)(f.params))
          .then(
            (result) => ws.send(JSON.stringify({ op: 'result', id: f.id, result: result === undefined ? null : result })),
            (err) => ws.send(JSON.stringify({ op: 'error', id: f.id, error: String((err && err.message) || err) })),
          );
      }
    });
    ws.addEventListener('close', () => {
      connected = false;
      fireStatus(false);
      setTimeout(open, 1000); // reconnect loop
    });
    ws.addEventListener('error', () => {
      try {
        ws.close();
      } catch {}
    });
  };
  open();

  return {
    ready,
    get connected() {
      return connected;
    },
    get settings() {
      return settings;
    },
    call(method, params = {}) {
      return new Promise((resolve, reject) => {
        const id = 'c' + seq++;
        pending.set(id, { resolve, reject });
        ws.send(JSON.stringify({ op: 'call', id, method, params }));
      });
    },
    publish(type, data = {}) {
      ws.send(JSON.stringify({ op: 'publish', event: { type, source, data } }));
    },
    // answer a bus method declared in your manifest "provides"
    provide(method, handler) {
      providers.set(method, handler);
      if (connected) ws.send(JSON.stringify({ op: 'register', methods: [method] }));
    },
    on(type, cb) {
      if (!listeners.has(type)) listeners.set(type, new Set());
      listeners.get(type).add(cb);
      return () => listeners.get(type)?.delete(cb);
    },
    onStatus(cb) {
      statusListeners.add(cb);
      return () => statusListeners.delete(cb);
    },
  };
}

module.exports = { connect };
```

Your `server.js` then reads like the webview client, with the same `ready` / `on` / `call` / `publish` / `provide` / `settings` surface, and it mirrors live connection status. Enabling the plugin is the trust decision; manifest declarations support routing and discovery rather than sandboxing the process.

```js
// server.js
const { connect } = require('./wks.js');
const wks = connect();

(async () => {
  await wks.ready;
  wks.on('agent.snapshot', async (a) => {
    if (a.pendingApproval) {
      await wks.call('notifications.post', { title: 'Agent needs you', body: a.sessionId });
    }
  });
})();
```

## Widgets

Widgets are drawn by the same renderer as panes, so they exist in the Electron app and the web app and not in the native app, which has no widget board.

A **widget** is a small, glanceable view pinned to a *project directory*. It lives in the inspector rail's Project board, keyed by the agent's cwd, so several agents working in one repo all see the same board.

A widget is a **sibling of a pane, not a shrunken one**. This is the same distinction iPhone widgets make, and for the same reason: your pane may own a big bundle, a scroll region, or an editing surface, and none of that reads at 150px. Declare `widgets` alongside `panes`; they're served from the same origin, so one sidecar backs both.

```json
{
  "id": "acme.ship",
  "name": "Ship",
  "apiVersion": "1",
  "server": { "command": "node", "args": ["server.js"], "port": 9211, "health": "/health" },
  "panes":   [{ "type": "acme.ship", "title": "Ship", "path": "/" }],
  "widgets": [
    { "id": "lamp", "title": "Ship status", "icon": "🚢", "path": "/widget/lamp", "sizes": ["small", "large"] }
  ]
}
```

**Sizes are a closed set of three**, iPhone's own, in a two-column board:

- `small`: 1×1, a square (~148px)
- `medium`: 2×1, full width, one row
- `large`: 2×2, a full-width square

Declare only the sizes your widget reads well at; the user picks among those and nothing else. Omitting `sizes` means `["small"]`. There is no free resizing: three classes force the design decision at authoring time and keep every board aligned. Your widget is told nothing about its size directly; lay out against the box you're given, and use a CSS container/media query if you need to reflow between classes.

Your widget page gets the same treatment a pane does:

- **The SDK**, auto-injected: `window.workspacer` with the same `call` / `publish` / `on` surface.
- **Theme tokens**: the app's `--wks-*` variables, reapplied when the user switches theme. Style against them and your widget matches the app.
- **Your settings**: `window.__WKS_SETTINGS__` plus a `wks-settings` event, same as a pane.
- **Query params**: `?cwd=<project>&surface=widget` (plus your `busToken`). `surface=widget` lets one HTML file serve both a pane and a widget and branch on it.

Two rules the host enforces on your behalf, worth designing for:

- **A widget can be unmounted at any moment.** Closing the rail or switching to the Session tab tears the webview down; reopening loads it fresh. Keep no state in the widget that isn't recoverable; put it in your sidecar.
- **Keep the sidecar the expensive part.** One sidecar can back a pane and several widgets while polling once. Don't give each widget its own poll loop, and avoid continuous animation, because a widget on a visible board is not background-throttled.

## The manifest

Schema version is `"apiVersion": "1"` (the loader rejects anything else). The loader and everything it validates are in `services/hub-rs/src/plugins/manifest.rs`; the real fields:

- `id` (required), `name`. `id` is the install dir name and the token key; namespace it (`owner.thing`).
- `apiVersion`, which MUST be the string `"1"`.
- `version`, your plugin's release version (semver-ish, e.g. `"1.4.0"`; an optional leading `v` is fine). Optional, but it's what powers update detection: the Plugins Manager re-fetches your repo's manifest and offers **Update** only when the published `version` is higher than the installed one. Omit it and the plugin can still be reinstalled, but never reports an update. Bump it on every release.
- `server`, the sidecar process: `command` (required when `server` is set), `args`, `port`, `health` (a path, e.g. `/health`). The pane's webview loads the plugin's panes from `http://127.0.0.1:<port><path>` directly. Which Node a `command` of exactly `node` runs on depends on the host; see "which Node your sidecar runs on".
- `ui`, instead of `server`, a subdir of static assets the hub serves itself at `/plugins/ui/<id>/`. This is a webview plugin with no sidecar process. Only the named subdir is exposed (not `plugin.json` or `.bus-token`).
- `panes`, pane types injected into the UI of the Electron app and the web app. Each: `type` (unique id), `title`, `icon`, `path` (the URL path served for the pane), and `scope` (`global` = Overview only, `agent` = inside an agent workspace and gets its sessionId/cwd, `both` = wherever you are, the default).
- `widgets`, glanceable views for a project's widget board (see widgets). Each: `id` (unique within your plugin), `title`, `icon`, `path`, and `sizes` (any of `small` / `medium` / `large`; omitted = `["small"]`).
- `hotkeys`, each with `id`, `default` (e.g. `ctrl+shift+a`), and `command`, which is either `open-pane:<paneType>` or `emit:<eventType>`.
- `settings`, typed settings the hub stores and the Electron app renders in Settings. Each: `key`, `label`, `type` (`boolean`/`number`/`string`/`select`), `default`, `options` (for `select`), `help`, and `secret` (masked write-only PAT/API-key handling, covered under Settings below). Delivered into the webview as `window.__WKS_SETTINGS__` (secrets redacted) + a `wks-settings` event.
- `capabilities`, advisory bus methods the plugin expects to **call**. The legacy object form `{ "method": "fs.read", "paths": ["${pluginDir}"] }` remains parse-compatible, but `paths` is not enforced.
- `provides`, capabilities the plugin **answers** on the bus (it becomes a provider other clients can call).
- `tools`, MCP tools the plugin contributes to agents through the workspacer facade (see "agent tools" below). Each: `name`, `description`, optional `inputSchema` (a JSON Schema object), and `method` (a bus method covered by your `provides`).
- `emits` / `consumes`, event types it publishes / subscribes to.
- `install`, a one-time setup argv run in the plugin dir after a GitHub install (e.g. `["go","build","-o","server","."]`). Requires the user's consent, which names the exact argv. A `node` command here gets the same runtime as a sidecar's; `npm`/`npx`/`yarn`/`pnpm` are always the user's own, and fail if the user has no Node toolchain, so prefer no `install` step at all (see "which Node your sidecar runs on"). It is not run for a folder you copy into the plugins directory by hand.

## Optional launch integrations

A sidecar may contribute `launchIntegration: { version: 1, agents: ["claude", "codex"], prepareMethod: "your.plugin.prepareLaunch" }`. Declare that exact method in `provides` and register it on the existing scoped bus. The loader requires a sidecar, version 1, and a method within the plugin's namespace.

Today the integration is chosen in the Electron app's Spawn agent dialog; the native app's New Agent has no picker for one. When a user selects it, the host calls it before launch with `{ version: 1, agent, cwd, model?, resume, provider? }`. Codex's `provider` contains only the resolved ID and optional upstream base URL. The request omits raw config, credentials and inherited environment.

Return optional `env` (string map) and `args` (string array): the host overlays the child environment and appends arguments after profile/host arguments. It rejects extra fields, nulls, NUL characters and oversized patches. Executable and cwd overrides are not allowed. Errors stop the launch; None bypasses the hook entirely.

The selected plugin ID persists with the workspace and runs again on resume. Current entry points are local Claude PTY/stream and Codex managed/hybrid sessions launched from the Electron app. This is a trusted-sidecar contribution. The [Headroom example](https://github.com/DJTouchette/workspacer/blob/master/plugins/examples/headroom/README.md) uses supervisor-issued `HUB_TOKEN` and `WKS_SETTINGS`, with no runtime npm dependencies. See the [full contract](https://github.com/DJTouchette/workspacer/blob/master/docs/launch-integrations.md) for limits and [setup instructions](docs-electron.html#launch-integrations).

## The bus protocol

Webviews should reach for the injected `window.workspacer` SDK (see your first plugin) rather than these raw frames, but this is the protocol it speaks under the hood, and the one a sidecar (or the `wks.js` twin above) talks directly. Whether webview or sidecar, a plugin is just another client on the hub bus. It opens one bidirectional WebSocket to the hub's bus, `ws://127.0.0.1:7895/bus?token=<busToken>` by default (webviews get the token in the pane URL as `?busToken=`; sidecars get theirs in `HUB_TOKEN` and the address in `HUB_URL`, which is the one to trust, since the native app's hub is not on `7895`) and exchanges JSON frames. Publish and subscribe share the same pipe. Four ops matter:

```js
// subscribe to events (topics you declared in "consumes")
ws.send(JSON.stringify({ op: 'subscribe', topics: ['agent.*', 'ui.*'] }));

// publish an event (a type you declared in "emits")
ws.send(JSON.stringify({ op: 'publish',
  event: { type: 'command.focus_agent', source: 'example.hello', data: { sessionId } } }));

// call a capability (a method you declared in "capabilities"); reply comes back as op:'result'
ws.send(JSON.stringify({ op: 'call', id: 'c1', method: 'agents.list', params: {} }));

// register methods you answer (declared in "provides"); calls arrive as op:'call'
ws.send(JSON.stringify({ op: 'register', methods: ['myplugin.status'] }));
```

An incoming event is `{"op":"event","event":{ id, type, source, time, data }}`, and the hub stamps `id`/`time` if you leave them blank. A reply to your `call` comes back as `{"op":"result","id":"c1","result":{…}}`, or `{"op":"error","id":"c1","error":"…"}`. When you `provide`, a caller's request arrives as an `op:'call'` frame you answer with an `op:'result'` carrying the same id.

Enabled plugins are trusted local code. Manifest `capabilities`, `emits`, and `consumes` document and route the integration; they are not an OS sandbox or grant boundary. Plugin identities still authenticate separately for provenance, and the router remains **single-owner per method**. Every `provides` entry must sit under your own plugin id (`<id>.something`); the loader rejects a manifest that claims anything else, and an id may not start with a core namespace such as `agents` or `sessions`.

## Capabilities & permissions

Capabilities are request/reply methods. List the ones you **call** in `capabilities`; register the ones you **answer** in `provides`. These fields support discovery, routing, and provenance. They do not contain enabled plugin code, which runs with the Workspacer user's machine access.

### legacy path metadata

The object form can document intended filesystem roots for host integrations, but it is not a security boundary. Enabled plugin code may access any path available to the Workspacer user:

```json
"capabilities": [
  "agents.list",
  { "method": "fs.read",  "paths": ["${pluginDir}"] },
  { "method": "fs.write", "paths": ["${pluginDir}"] }
]
```

Path tokens such as `${pluginDir}` and `${agentCwd}` remain useful routing placeholders. Webview origin isolation and per-pane identity still protect the app document and preserve provenance, but they do not sandbox a plugin sidecar from the host filesystem.

### common host capabilities

The methods the host registers today are provided by the Rust hub itself, in the native app and under `workspacer serve` alike; the Electron app registers a few desktop-only ones on top. They are the same surface the MCP facade re-exposes as tools. The ones you'll reach for first:

- `agents.list`, running agents with state / usage / pending asks.
- `agents.sendMessage`, send a prompt to an agent (`{ sessionId, text }`).
- `agents.spawn`, start a new agent (returns its sessionId). There is no `agents.kill`; terminate one with `claude.signal` (`{ sessionId, signal: 'SIGKILL' }`).
- `notifications.post`, notify the user (`{ title, body }` at minimum). What happens next depends on the host. In the Electron app every call lands in the in-app notification center (the bell in the top bar) and, unless the user disabled OS notifications or you pass `inAppOnly: true`, also shows a clickable desktop notification. Under the native app and `workspacer serve` the call succeeds and the hub writes the title and body to its log; nothing is shown yet, so do not make it your only signal there. Optional in the Electron app: `level` (`info`|`success`|`warn`|`error`), `source` (shown in the center, e.g. `"plugin:ci"`), a click target (`sessionId` to focus that agent, `paneType` to open your pane, or `url`), `key` (same-key replaces instead of stacking), `silent` (no toast, history only).
- `claude.approve` / `claude.answer` / `claude.signal`, resolve an approval, answer an AskUserQuestion, send a signal.
- `sessions.snapshot` / `sessions.transcript` / `sessions.conversation`, live session state and history.
- `fs.read` / `fs.write` / `fs.watch` / `search.project`, host file I/O and search. Manifest paths document intended use but do not sandbox enabled code.

List method names in `provides` and answer them on the bus, and you become a first-class capability provider the rest of the fleet (dashboards, rules, supervisors, the MCP facade) can call. A bridge sidecar, for instance, can consume an external MCP server and re-expose its tools as hub capabilities.

### agent tools

A plugin can hand its capabilities straight to the user's **agents** as MCP tools. Declare a `tools` array in the manifest, each entry bound to a method your `provides` covers, and answer the method on the bus:

```json
"provides": ["myorg.jira.search"],
"tools": [{
  "name": "search",
  "description": "Search Jira issues with a JQL query.",
  "inputSchema": { "type": "object", "properties": { "jql": { "type": "string" } } },
  "method": "myorg.jira.search"
}]
```

```js
// sidecar (wks.js) or webview (window.workspacer) — same API:
wks.provide('myorg.jira.search', async (params) => {
  const issues = await searchJira(params.jql);
  return { issues }; // raw data — the caller is a model, not your pane
});
```

The Workspacer MCP facade exposes the tool as `mcp__workspacer__myorg_jira_search` to every supported agent while the plugin is enabled. The initial catalog is loaded before the facade reports ready; later changes are polled (normally within ~15s). Clients that cache `tools/list` may need to reconnect or refresh because no list-changed notification is currently emitted.

Rules, all enforced at load or serve time: the tool `name` is lowercase `[a-z0-9_]` starting with a letter; `description` is required; `inputSchema` must be a JSON Schema object; and `method` must be covered by the plugin's own-namespace `provides`. Return plain JSON-serializable data; a thrown error becomes the agent's error result verbatim.

> **Source of truth.** The exact params/return shape of each host capability isn't a frozen public API yet. Treat the handlers in `services/hub-rs/src/services/` and `plugins/examples/` as authoritative for field names (`apps/desktop/src/main/services/hubCapabilities.ts` for the Electron app's own), and the MCP tool list (`services/hub-rs/assets/mcp-effective-tools.json`, implemented in `services/hub-rs/src/mcp.rs`) for the stable subset.

## Events you can consume

Events are fire-and-forget pub/sub (state changes, lifecycle, UI activity). Declare the ones you subscribe to in `consumes` and the ones you publish in `emits`. Use capabilities (previous section) when you need an answer or want to *make* something happen; use events to react.

### consume: events the fleet publishes to you

- `agent.state_changed`, an agent's live state changed. This is the workhorse wake-up signal, bridged from claudemon: the sessionId, the triggering hook event, mode, and cwd. For context %, token/cost, and pending approvals, call `agents.list` or consume `agent.snapshot`. (There are no `agent.spawned` / `agent.done` / `agent.terminated` events today.)
- `agent.snapshot`, a per-agent snapshot (state + usage), published by the hub. **The conversation on it is a bounded window, not the whole transcript**, because this fires on every flush of every session and shipping full history would push megabytes a second at every subscriber. You get the newest turns plus `conversationOffset`, the absolute index of the array's first turn (absolute index = `conversationOffset` + array index); tool-call and file-change lists are tailed and their payloads truncated the same way. Need only to know *that* something changed? Treat it as a wake-up and re-query `agents.list`, which is what the catalog's `fleet-radar` does, and it is immune to payload changes. Need the actual transcript? Call `sessions.conversation` with `{ sessionId, sinceSeq, limit }`, which returns the items after a sequence instead of the whole log; `limit` keeps only the newest N, and the reply's `window_first_seq` is greater than its `first_seq` while older items remain.
- `agent.statusline`, the context % / cost / rate-limit status line for an agent.
- `workflow.started` / `workflow.completed` / `workflow.failed`, a workflow run started, finished, or failed; `workflow.agent.finished`, one agent in a run reached done/failed.
- `ui.pane.opened` / `ui.pane.focused` / `ui.tab.focused`, renderer activity naming which pane or tab the user is looking at (subscribe to `ui.*` to follow focus). Published by the Electron app and the web app only; the native app has no panes and sends none of these.
- `fs.changed`, a file you're watching changed (paired with the `fs.watch` capability).
- `sidecar.*` / `plugin.*`, plugin supervisor and lifecycle state (these back the health colors in the Electron app's Plugins Manager).

Subscribing to a namespace (`agent.*`, `ui.*`, `sidecar.*`) is the easy way to catch a whole family without listing each type.

### emit: events you publish

- **Your own namespaced events**, anything under your plugin's namespace, e.g. `example.hello.tick`. Declare each type (or a `myplugin.*` wildcard) in `emits`. Other plugins can `consume` them.
- `command.focus_agent` (`data: { sessionId }`), ask whichever window is open to focus an agent. The native app honours it, as do the Electron app and the web app. `command.open_spawn_dialog` opens the new-agent form for the person to confirm; `command.spawn_agent` is acted on by the Electron renderer only. The `command.*` namespace is the "ask the host to do something" channel: you publish, the client acts. A hotkey can fire one directly with `"command": "emit:<eventType>"`.
- `command.run_action` (`data: { action, digit? }`) runs one keyboard action by registry id, the command layer's vocabulary as an event. In the Electron app that is its whole action registry: `zoom-pane`, `next-tab`, `toggle-sidebar`, `chat-scroll-down`, `jump-pinned` (+ `digit`), and so on. The native app applies the ones it has a screen for (session navigation, the new-agent form, settings, review) and says so when it has not. Navigation-grade only on both: the client validates the id and refuses the decision verbs (`approve-attention`/`deny-attention`), since approvals ride their own scoped facade tools.
- `notify.post`, fire-and-forget path into the in-app notification center (same `data` fields as the `notifications.post` capability). Every connected client running the web renderer (the Electron app, the web app) ingests it; the native app does not listen for it. If the window isn't focused when it arrives, it auto-escalates to a clickable OS notification (browser notification on web), so it is never invisible. The difference from the capability is delivery semantics (broadcast, no ack, OS only when unwatched) rather than reach.

## Settings

Declare typed `settings` in the manifest and the hub stores, validates and delivers them. The Electron app renders them for you: its Settings lists every installed plugin under **Plugins**, and each plugin gets its own settings page. The native app has no screen for them yet; there the values are whatever your `default`s say until something changes them over the bus (`plugins.setSettings` with `{ pluginId, values }`, which the hub accepts only from its owner's token). Each entry is a `key`, a `label`, a `type` (`boolean` / `number` / `string` / `select`), a `default`, an `options` list (for `select`), and optional `help` text:

```json
"settings": [
  { "key": "interval", "label": "Poll interval (s)", "type": "number", "default": 30,
    "help": "How often to re-check the fleet." },
  { "key": "loud", "label": "Notify on done too", "type": "boolean", "default": false },
  { "key": "channel", "label": "Alert channel", "type": "select",
    "default": "desktop", "options": ["desktop", "slack", "none"] }
]
```

**Secrets.** Add `"secret": true` to a string setting that holds a PAT/API key. The host renders a masked, write-only input (set, replace or clear, never displayed), and the stored value never leaves the hub on any read surface: settings reads, the `plugin.settings.changed` broadcast, and the webview `window.__WKS_SETTINGS__` injection all report the sentinel `"__WKS_SECRET__"` instead. The plaintext is delivered ONLY in the sidecar's `WKS_SETTINGS` env, so a secret is for sidecar code and never for webview code. Storage is a `0600` file in the plugin's directory. A secret must not declare a non-empty `default` (the manifest listing is public). Writing the sentinel back is ignored ("unchanged"), writing `""` clears. Older hosts ignore the flag and fall back to a plain string input, so adding it is backward-compatible.

Values are delivered to a **webview** as `window.__WKS_SETTINGS__` (secrets redacted) and re-delivered live on every change via a `wks-settings` event, so a webview updates without reloading:

```js
let settings = window.__WKS_SETTINGS__ || {};
window.addEventListener('wks-settings', (e) => { settings = e.detail; render(); });
```

**Per-project settings.** Add `"scope": "project"` to a setting whose value belongs to a *directory* rather than to the plugin: which Jira project a repo maps to, which GitHub repo it ships from. The host stores it under `config.projects[<dir>].plugins[<your-id>]`, and the Electron app renders it on its **Projects** settings page, beside that project's name and icon. Read it with `config.get`, looking up your own id for the pane's `cwd`. A project setting cannot be secret; the manifest is rejected at load if you try.

For a **sidecar**, a settings edit publishes `plugin.settings.changed` on the bus and the sidecar is **restarted** so it re-reads them at boot. So: webviews get settings live, sidecars pick them up on restart.

## Developing with hot-reload

With the `workspacer` CLI on your PATH (it ships in the server bundle; `make build-cli` builds it from source), point a development control plane at your plugin directory:

```
workspacer serve --hub-only --plugins-dir /absolute/path/to/plugins
```

The hub loads the directory at startup and emits `plugin.loaded`. For a tighter loop, `workspacer plugin dev <dir>` starts a hub of its own around a single plugin folder and hot-reloads it on change: it validates the manifest before anything starts, poll-watches your source, runs the manifest's `install` build step when files change (keeping the running sidecar if the build fails), reloads the plugin, and streams the sidecar's logs and bus events to your terminal. It takes the same flags as `serve`, so `--hub-port` moves it out of the way of a hub you already have running.

```
workspacer plugin dev ./my-hello     # watch → rebuild → hot-reload → tail logs
```

A webview plugin needs no build step: edit the files and reopen the pane in the Electron app or the web app. A sidecar with an `install` step is rebuilt before reload. The Electron app's Plugins Manager has an Enable/Disable toggle that writes a `.disabled` marker and reloads, so you can flip a plugin off without uninstalling it; anywhere else, create or delete that file yourself and restart the hub.

## Publishing

A plugin is just a folder in a git repo. To share one, push it to **its own GitHub repo** with `plugin.json` at the root, then install it by reference. Install-by-reference is a feature of the Plugins Manager, which lives in the Electron app and the web app; the bullets below describe it. A native-app user installs by hand, as in your first plugin: copy the built folder into the app's plugins directory and restart. So if you want native users, commit a prebuilt sidecar or one with no build step.

- **Install** from the Plugins Manager by pasting an `owner/repo` reference (or a full URL, a `/tree/<ref>` URL, or a direct `.tar.gz` URL). work{spacer} downloads it, runs the manifest's `install` build step, and loads it.
- Installation is the **trusted-install** model, like a VS Code extension: it downloads and runs code from the internet, so the confirmation shows the manifest before enabling it. Extraction is zip-slip-guarded and atomic; manifest capability/path fields do not sandbox runtime access.
- **Updates** are version-driven. The Plugins Manager's *Check for updates* re-fetches each installed plugin's manifest from the repo it was installed from and compares the published `version` to the one on disk. It surfaces an **Update** button only when the source is newer (otherwise the button reads **Reinstall**, which pulls a fresh copy on demand). So cut a release by bumping `version` in `plugin.json` and pushing; users installed from your repo will see the update on their next check.
- Give your repo a clear README with the manifest's declared `capabilities` so installers know what it can reach.

There's a public catalog of install-ready plugins at [github.com/DJTouchette/workspacer-plugins](https://github.com/DJTouchette/workspacer-plugins): dashboards (Fleet Radar, Cost HUD, Focus Tracker), fleet automations (Policy Approver, Fleet Guardian, Test on Save, CI Watcher), and remote reach (Slack Bridge, Phone Push, Standup Digest). Each is a zero-dependency repo you can install straight from the Plugins Manager (paste `DJTouchette/workspacer-plugin-<name>`), and together they double as reference implementations for every plugin shape on this page. To list yours there, open a PR against the catalog repo's index.
