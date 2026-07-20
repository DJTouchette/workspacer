# How to build a work{spacer} plugin

> HTML version: build-plugin.html

A plugin extends work{spacer} with new panes, automations, and capabilities. It's a folder with a `plugin.json` manifest at its root that declares what it contributes. The hub reads `<plugins-dir>/<name>/plugin.json`, validates it, starts whatever it needs, and announces the contribution on the bus (`plugin.loaded`). The desktop renderer picks that up and registers your pane types, command-palette entries, and hotkeys.

## The two kinds

Every plugin is one of two shapes, and it's the first decision you make. The split is exactly the `server` vs `ui` field in the manifest.

### webview plugin (`ui`)

Set `ui` to a subdirectory of static assets and **omit `server`**. There is no process to run: the hub (trusted) serves your files at `/plugins/ui/<id>/`, and your page opens as a pane in a webview. The webview talks to the bus over a WebSocket using a **per-plugin token the host injects into the pane URL** (`?busToken=…`), scoped to exactly the capabilities your manifest declares. Because there's no arbitrary process, there's nothing to escape the bus through, so capability scoping fully confines it.

- Reach for it when your plugin *is* a UI: a dashboard, a panel, an editor, a rule editor.
- Any language that compiles to static HTML/JS/CSS. No build step needed if you ship plain files.
- Bundled examples (in `services/hub/examples/`): the sandboxed `editor` and the `transcript-timeline` replay pane.

### sidecar plugin (`server`)

Set `server.command` to a long-lived process the hub spawns and supervises (spawn → health-poll → restart-on-crash, SIGTERM then SIGKILL on stop). It's a polyglot sidecar: **any language**. It connects to the bus itself, so it can run *always-on with no pane at all* (an automation), `provide` capabilities other clients call, and/or serve its own webview panes from its port (the host points the pane's webview straight at `http://127.0.0.1:<port><path>` — nothing is proxied through the hub).

- Reach for it when your plugin *does* something in the background: reacts to events, calls out to another service, answers capabilities for the rest of the fleet.
- Give it a `port` + `health` path so the supervisor can health-check it and surface `sidecar.*` status colors.
- Examples: the bundled `clock-plugin` (the minimal sidecar demo) and the catalog's automations — `policy-approver`, `fleet-guardian`, `test-on-save`, `slack-bridge`.

The two aren't mutually exclusive in spirit — a sidecar can also serve panes — but a single manifest sets *either* `server` *or* `ui` as the way its panes are served. If it declares panes with neither, the loader rejects it (the webview would have no URL to load).

## Your first plugin

The fastest path is a **webview** plugin that lists the running agents. Three files, no build step.

### 1. the folder

```
my-hello/
  plugin.json
  ui/
    index.html
```

### 2. the manifest

`apiVersion` MUST be `"1"` (the loader rejects anything else), `ui: "ui"` makes it webview-only, and you ask for only the caps you use:

```json
{
  "id": "example.hello",
  "name": "Hello",
  "apiVersion": "1",
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

`panes` contributes one pane type; `hotkeys` binds a key to open it; `capabilities` asks for the single verb `agents.list` and nothing else. Ask for only what you use — the bus rejects any call you didn't declare.

### 3. the page

The host **auto-injects the Plugin SDK** into your served HTML, so `window.workspacer` is just there — no bus WebSocket to hand-roll. Await `ready`, then `call` a capability, subscribe with `on`, `publish`, and read live `settings`. The whole client is a few lines:

```js
await window.workspacer.ready;                     // resolves when connected

// receive events (only the types you declared in "consumes")
window.workspacer.on('agent.state_changed', (data) => {
  console.log('state', data.sessionId, data.mode);
});

// call a capability (only the methods you declared in "capabilities")
const agents = await window.workspacer.call('agents.list');
document.body.textContent = `${agents.length} agent(s) running`;

// publish an event you declared in "emits"
window.workspacer.publish('command.focus_agent', { sessionId: agents[0]?.sessionId });

// typed settings, delivered live
let settings = window.workspacer.settings;
window.workspacer.onSettings((next) => { settings = next; });
```

> **Plugin SDK.** The hub serves `/plugins/sdk.js` and injects `<script src="/plugins/sdk.js"></script>` (plus `window.__WKS_PLUGIN_ID__` and `window.__WKS_SETTINGS__`) into every webview's HTML, so `window.workspacer` is present with no setup. It stays fully inside the manifest sandbox: the SDK subscribes to `*` under the hood, but **delivery is still capability-scoped** — `on(type)` only fires for events you listed in `consumes`, and `call(method)` only works for methods you listed in `capabilities`. It is a convenience wrapper over the bus, not a way around it. `window.workspacer` also exposes a live `.connected` boolean (true while the socket is up), `onStatus(connected => …)` to react to every connect/disconnect (including reconnect cycles), plus `.token` and `.url` if you need the raw connection.

Style the page with the injected `--wks-*` theme tokens (with fallbacks, e.g. `background: var(--wks-bg-base, #1a1a1a)`) so it matches the app. The host re-injects them on every live theme switch, and also exposes `window.__WKS_THEME__` + a `wks-theme` event for canvas UIs.

### 4. load it

Drop `my-hello/` into your plugins dir (in the app: `<configDir>/plugins`; in dev: whatever you passed to `--plugins-dir`). The hub scans it, validates the manifest, and emits `plugin.loaded`; the renderer registers your pane and hotkey. Open it with `Ctrl+Shift+H` or from the command palette.

### make it a sidecar instead

To make this a **sidecar**, drop `ui`, add a `server` block pointing at your process (and an `install` build step if it needs compiling), and connect to the bus from that process. A sidecar has no injected SDK — it speaks the bus frames directly. It gets its token in the `HUB_TOKEN` environment variable instead of the URL; the bus address isn't passed in the environment, so connect to the default `127.0.0.1:7895` (or whatever `--addr` your hub listens on). A tiny Node sidecar that watches for agents that need you and posts a notification, raw frames and all:

```js
// server.js — a headless sidecar (no pane at all)
const WebSocket = require('ws');
const token = process.env.HUB_TOKEN || '';
const addr  = process.env.HUB_ADDR  || '127.0.0.1:7895';
const ws = new WebSocket(`ws://${addr}/bus?token=${encodeURIComponent(token)}`);

ws.on('open', () => {
  // declared in "consumes"
  ws.send(JSON.stringify({ op: 'subscribe', topics: ['agent.state_changed'] }));
});
ws.on('message', (buf) => {
  const f = JSON.parse(buf.toString());
  if (f.op !== 'event') return;
  const a = f.event.data || {};
  if (a.needsApproval) {
    // declared in "capabilities"
    ws.send(JSON.stringify({ op: 'call', id: 'n' + Date.now(), method: 'notifications.post',
      params: { title: 'Agent needs you', body: a.name || a.sessionId } }));
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
  "consumes": ["agent.state_changed"],
  "capabilities": ["notifications.post"],
  "install": ["npm", "install"]
}
```

A sidecar with no `panes` and no `ui` is a pure automation: it never shows a window, it just reacts on the bus. If you give it a `port` + `health`, the supervisor health-checks it and colors it in the Plugins Manager.

### the Node twin (`wks.js`)

Sidecars have no injected SDK, but you can vendor one file that gives you the same surface as `window.workspacer`. Drop this zero-dependency `wks.js` next to your `server.js` (it uses only Node >=22 built-ins — the global `WebSocket`, `fs`, `path`), then `require('./wks.js')`. It reads the token from `HUB_TOKEN` (falling back to `WKS_BUS_TOKEN` then a `.bus-token` file), connects to `ws://127.0.0.1:7895/bus`, and reconnects on drop:

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

// connect() -> { ready, connected, call, publish, on, onStatus, settings } — mirrors window.workspacer.
function connect(opts = {}) {
  const url = opts.url || 'ws://127.0.0.1:7895/bus';
  const source = opts.source || 'sidecar';
  const listeners = new Map(); // type -> Set(cb)
  const pending = new Map(); // id -> { resolve, reject }
  const statusListeners = new Set(); // cb(connected)
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

Your `server.js` then reads like the webview client — the same `ready` / `on` / `call` / `publish` / `settings` surface, and it mirrors the live connection status too (`.connected` and `onStatus(connected => …)`, firing on every drop and reconnect), so a sidecar can track the bus the same way a webview does (the bus enforces your manifest either way):

```js
// server.js
const { connect } = require('./wks.js');
const wks = connect();

(async () => {
  await wks.ready;
  wks.on('agent.state_changed', async (a) => {
    if (a.needsApproval) {
      await wks.call('notifications.post', { title: 'Agent needs you', body: a.name || a.sessionId });
    }
  });
})();
```

## The manifest

Schema version is `"apiVersion": "1"` (the loader rejects anything else). The authoritative field list is `internal/plugin/manifest.go`; the real fields:

- `id` (required), `name`. `id` is the install dir name and the token key; namespace it (`owner.thing`).
- `apiVersion` — MUST be the string `"1"`.
- `server`, the sidecar process: `command` (required when `server` is set), `args`, `port`, `health` (a path, e.g. `/health`). The hub serves the plugin's panes from `http://127.0.0.1:<port><path>`.
- `ui`, instead of `server`, a subdir of static assets the hub serves itself at `/plugins/ui/<id>/`. This is a webview plugin with no sidecar process. Only the named subdir is exposed (not `plugin.json` or `.bus-token`).
- `panes`, pane types injected into the UI. Each: `type` (unique id), `title`, `icon`, `path` (the URL path served for the pane), and `scope` (`global` = Overview only, `agent` = inside an agent workspace and gets its sessionId/cwd, `both` = wherever you are, the default).
- `hotkeys`, each with `id`, `default` (e.g. `ctrl+shift+a`), and `command`, which is either `open-pane:<paneType>` or `emit:<eventType>`.
- `settings`, typed settings the host renders in Settings. Each: `key`, `label`, `type` (`boolean`/`number`/`string`/`select`), `default`, `options` (for `select`), and `help`. Delivered into the webview as `window.__WKS_SETTINGS__` + a `wks-settings` event.
- `capabilities`, bus methods the plugin may **call**. A bare string (`"agents.list"`) for an unscoped verb, or the object form `{ "method": "fs.read", "paths": ["${pluginDir}"] }` for a filesystem-scoped one.
- `provides`, capabilities the plugin **answers** on the bus (it becomes a provider other clients can call).
- `emits` / `consumes`, event types it publishes / subscribes to.
- `install`, a one-time setup argv run in the plugin dir after a GitHub install (e.g. `["go","build","-o","server","."]`).

## The bus protocol

Webviews should reach for the injected `window.workspacer` SDK (see "your first plugin") rather than these raw frames — but this is the protocol it speaks under the hood, and the one a sidecar (or the `wks.js` twin above) talks directly. Whether webview or sidecar, a plugin is just another client on the hub bus. It opens one bidirectional WebSocket to `ws://127.0.0.1:7895/bus?token=<busToken>` (webviews get the token in the pane URL as `?busToken=`; sidecars get theirs in the `HUB_TOKEN` environment variable) and exchanges JSON frames. Publish and subscribe share the same pipe. Four ops matter:

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

An incoming event is `{"op":"event","event":{ id, type, source, time, data }}` — the hub stamps `id`/`time` if you leave them blank. A reply to your `call` comes back as `{"op":"result","id":"c1","result":{…}}`, or `{"op":"error","id":"c1","error":"…"}`. When you `provide`, a caller's request arrives as an `op:'call'` frame you answer with an `op:'result'` carrying the same id.

Two rules the bus enforces against your declared grants: you can only `call` a method you listed in `capabilities`, and you can only `publish` a type you listed in `emits` — an undeclared call or publish is refused. Topic patterns (in `subscribe`, `consumes`, `emits`) are exact (`agent.state_changed`), namespace wildcard (`agent.*`), or all (`*`). The router is **single-owner per method**, so when you `provide`, pick a namespace nobody else claims.

## Capabilities & permissions

Capabilities are request/reply methods. List the ones you **call** in `capabilities`; register the ones you **answer** in `provides`. The whole model is **fail-closed**: a plugin gets exactly the grants its manifest declares and nothing more, and an undeclared call is refused at the bus.

### the path-scoped rule

Filesystem and project-search methods (`fs.*`, `search.project`) are the only ones that **must** use the object form and declare `paths`, or the loader rejects them — a plugin can never get unrestricted host filesystem access:

```json
"capabilities": [
  "agents.list",
  { "method": "fs.read",  "paths": ["${pluginDir}"] },
  { "method": "fs.write", "paths": ["${pluginDir}"] }
]
```

Path tokens: `${pluginDir}` (your own folder), `${agentCwd}`, or an absolute path. `${agentCwd}` only resolves on a **per-pane webview token** for an agent-scoped pane (the pane mints an ephemeral token confined to that agent's directory on mount and revokes it on unmount) — it grants nothing on a static per-plugin or sidecar token. Anything unresolved grants nothing.

### common host capabilities

The methods the host registers today (provided by the desktop app, or headlessly by `cmd/brain`) — the same surface the MCP facade re-exposes as tools. The ones you'll reach for first:

- `agents.list`, running agents with state / usage / pending asks.
- `agents.sendMessage`, send a prompt to an agent (`{ sessionId, text }`).
- `agents.spawn`, start a new agent (returns its sessionId). There is no `agents.kill` — to stop or steer a session, use `claude.signal` (`{ sessionId, signal }`).
- `notifications.post`, show a desktop notification (`{ title, body }`).
- `claude.approve` / `claude.answer` / `claude.signal`, resolve an approval, answer an AskUserQuestion, send a signal.
- `sessions.snapshot` / `sessions.transcript` / `sessions.conversation`, live session state and history.
- `fs.read` / `fs.write` / `fs.watch` / `search.project`, path-scoped file I/O and ripgrep search (object form, `paths` required).

List method names in `provides` and answer them on the bus, and you become a first-class capability provider the rest of the fleet (dashboards, rules, supervisors, the MCP facade) can call — e.g. a bridge sidecar can consume an external MCP server and re-expose its tools as hub capabilities.

> **Source of truth.** The exact params/return shape of each host capability isn't a frozen public API yet. Treat `apps/desktop/src/main/services/hubCapabilities.ts` and `services/hub/examples/` as authoritative for field names, and the MCP tool list (`cmd/mcp/main.go`) for the stable subset.

## Events you can consume

Events are fire-and-forget pub/sub (state changes, lifecycle, UI activity). Declare the ones you subscribe to in `consumes` and the ones you publish in `emits`. Use capabilities when you need an answer or want to *make* something happen; use events to react.

### consume — events the fleet publishes to you

- `agent.state_changed`, the workhorse transition ping — published from claudemon's stream by the hub bridge, payload `{ sessionId, hookEvent, mode, cwd }` (the state dot). There are no separate `agent.spawned` / `agent.done` / `agent.terminated` events — subscribe to this for mode transitions, and to `workflow.*` for lifecycle.
- `agent.snapshot`, a full per-agent snapshot (state + usage). Context %, token/cost, and pending approvals/questions ride here and on the enriched `agents.list` rows.
- `agent.statusline`, the context % / cost / rate-limit status line for an agent.
- `workflow.started` / `workflow.completed` / `workflow.failed` / `workflow.agent.finished`, a workflow run started, finished, failed, or one of its agents finished.
- `ui.pane.opened` / `ui.pane.focused` / `ui.tab.focused`, renderer activity — which pane/tab the user is looking at (subscribe to `ui.*` to follow focus).
- `fs.changed`, a file you're watching changed (paired with the `fs.watch` capability).
- `sidecar.*` / `plugin.*`, plugin supervisor and lifecycle state (these back the health colors in the Plugins Manager).

Subscribing to a namespace (`agent.*`, `ui.*`, `sidecar.*`) is the easy way to catch a whole family without listing each type.

### emit — events you publish

- **Your own namespaced events**, anything under your plugin's namespace, e.g. `example.hello.tick`. Declare each type (or a `myplugin.*` wildcard) in `emits`. Other plugins can `consume` them.
- `command.focus_agent` (`data: { sessionId }`) / `command.spawn_agent`, ask the desktop to focus or spawn an agent. The `command.*` namespace is the "ask the host to do something" channel: you publish, the renderer acts. A hotkey can fire one directly with `"command": "emit:<eventType>"`.

## Settings

Declare typed `settings` in the manifest and the host renders them in Settings for you. Each entry is a `key`, a `label`, a `type` (`boolean` / `number` / `string` / `select`), a `default`, an `options` list (for `select`), and optional `help` text:

```json
"settings": [
  { "key": "interval", "label": "Poll interval (s)", "type": "number", "default": 30,
    "help": "How often to re-check the fleet." },
  { "key": "loud", "label": "Notify on done too", "type": "boolean", "default": false },
  { "key": "channel", "label": "Alert channel", "type": "select",
    "default": "desktop", "options": ["desktop", "slack", "none"] }
]
```

Values are delivered to a **webview** as `window.__WKS_SETTINGS__` and re-delivered live on every change via a `wks-settings` event, so a webview updates without reloading:

```js
let settings = window.__WKS_SETTINGS__ || {};
window.addEventListener('wks-settings', (e) => { settings = e.detail; render(); });
```

For a **sidecar**, a settings edit publishes `plugin.settings.changed` on the bus and the sidecar is **restarted** so it re-reads them at boot. So: webviews get settings live, sidecars pick them up on restart.

## Developing with hot-reload

For iterating on a plugin, point a dev hub at your own dir and drop the folder in:

```
go run ./cmd/hub --plugins-dir /path/to/plugins
```

The hub scans on load and emits `plugin.loaded`. For a tighter loop, `workspacer plugin dev <dir>` watches a single plugin folder and hot-reloads it on change: it poll-watches your source, runs the manifest's `install` build step when files change, tells the hub to reload the plugin, and streams the sidecar's logs to your terminal.

```
workspacer plugin dev ./my-hello     # watch → rebuild → hot-reload → tail logs
```

> **Note.** `workspacer plugin dev` is a sibling task in flight — the poll-based watch → rebuild → hot-reload → logs loop. Until it lands, iterate with a dev hub pointed at your plugins dir and re-drop the folder (or toggle Enable/Disable in the Plugins Manager) to reload.

A webview plugin needs no build step — edit the files and reopen the pane. A sidecar with an `install` step is rebuilt before reload. The Plugins Manager's Enable/Disable toggles a `.disabled` marker and reloads, so you can flip a plugin off without uninstalling it.

## Publishing

A plugin is just a folder in a git repo. To share one, push it to **its own GitHub repo** with `plugin.json` at the root, then install it by reference.

- **Install** from the Plugins Manager by pasting an `owner/repo` reference (or a full URL, a `/tree/<ref>` URL, or a direct `.tar.gz` URL). work{spacer} downloads it, runs the manifest's `install` build step, and loads it.
- Installation is the **trusted-install** model, like a VS Code extension: it downloads and runs code from the internet, so it asks for consent and shows the manifest and permissions first. Extraction is zip-slip-guarded and atomic.
- Give your repo a clear README with the manifest's declared `capabilities` so installers know what it can reach.

There's a public catalog of install-ready plugins at [github.com/DJTouchette/workspacer-plugins](https://github.com/DJTouchette/workspacer-plugins): dashboards (Fleet Radar, Cost HUD, Focus Tracker), fleet automations (Policy Approver, Fleet Guardian, Test on Save, CI Watcher), and remote reach (Slack Bridge, Phone Push, Standup Digest). Each is a zero-dependency repo you can install straight from the Plugins Manager (paste `DJTouchette/workspacer-plugin-<name>`), and together they double as reference implementations for every plugin shape on this page. To list yours there, open a PR against the catalog repo's index.
