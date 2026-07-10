# Rules Engine Plugin — Build Spec

> A workspacer plugin: a tiny always-on **event → action interpreter** sitting on the
> hub bus. "When an agent needs approval, ping me and jump to it." "When an agent
> finishes, send my phone a push." The plugin is a **sidecar (the brain)** plus a
> **webview (the rule editor)**.
>
> This doc is self-contained — a fresh session can build the whole thing from it.
> Read §1 first if you don't know the hub yet.

---

## 0. TL;DR

- Build the plugin as its own GitHub repo (installable via “Install from GitHub…”).
- **Sidecar** (Go recommended): connects to the hub bus, subscribes to events,
  evaluates a rule list, and fires actions (call capabilities / publish commands /
  hit webhooks). It also serves the webview + a small `/rules` HTTP API, and
  persists rules to a JSON file.
- **Webview**: a rule editor (list / add / edit / enable / delete) that reads &
  writes rules via the sidecar's `/rules` API.
- Ship with a handful of **starter rules** (§10).

---

## 1. Background: the workspacer hub + plugin system

**The hub** is a Go daemon at `hub/` (module `github.com/djtouchette/workspacer-hub`).
It's an event bus + capability router + plugin supervisor that everything connects
to. Electron `main`, plugin sidecars, plugin webviews, and a claudemon bridge are
all just clients of it.

- Bus + HTTP listen on **`127.0.0.1:7895`** by default.
- WebSocket bus endpoint: **`ws://127.0.0.1:7895/bus`**.
- Plugins live in **`~/.config/workspacer/plugins/<name>/`**, each with a
  `plugin.json`. The Electron app spawns the hub with `--plugins-dir <that dir>`
  and seeds it from `hub/examples/` on first run.
- Read `hub/README.md` for the protocol summary. The bundled `hub/examples/editor/`
  is a working **bus-native webview** (it connects to the bus and calls
  capabilities directly from the browser); the workspacer-plugins repos
  (fleet-radar, cost-hud, …) are fuller webview + sidecar references.

**A plugin** = a sidecar process + a manifest declaring what it contributes (panes,
hotkeys, capabilities, events). The hub supervises the sidecar (spawn / health /
restart) and announces the plugin so the UI injects its pane + hotkey.

**Install path:** push the plugin to a public GitHub repo → in workspacer, command
palette → "Install from GitHub…" → `owner/repo`. The hub downloads the tarball,
runs the manifest's `install` command (the build step), and supervises the sidecar.
For local dev, just drop the folder in `hub/examples/` (dev seeds from there) or in
`~/.config/workspacer/plugins/`.

**Important runtime note:** today the hub is a child of the Electron app, so the
sidecar runs **as long as workspacer is running** — not literally 24/7. The win
over a webview-only plugin is that rules fire **even when the rules pane is closed**
(you're looking at an agent, not the editor). True app-closed operation would need
the hub run standalone — out of scope here.

---

## 2. Architecture: why the brain is the sidecar

A webview pane only runs **while it's open**. Rules must keep firing in the
background while you look at other panes — so the evaluation engine lives in the
**sidecar** (the supervised server process), which stays alive while the plugin is
installed. The webview is only the config screen.

```
   ┌───────────────────────────── rules-engine plugin ──────────────────────────┐
   │  sidecar (Go, always-on)                          webview (editor, on-open) │
   │  • WS client → hub bus                             • lists / edits rules     │
   │  • subscribes agent.* ui.* ...                     • GET/POST sidecar /rules │
   │  • evaluates rules on each event                   • served by the sidecar   │
   │  • actions: call capability / publish command /                              │
   │             HTTP webhook / emit event                                        │
   │  • persists rules.json; polls agents.list (cost)                             │
   └──────────────────────────────────────────────────────────────────────────────┘
                 │ ws://127.0.0.1:7895/bus                  │ http://127.0.0.1:<port>/rules
                 ▼                                          ▼
              HUB BUS  ◀── agent.state_changed (claudemon), ui.* (workspacer), ...
                       ──▶ command.* / capability calls
```

The webhook action is a second reason the brain is the sidecar: a webview is
CORS-boxed, but the sidecar process can POST to any external service (ntfy / Slack /
Pushover → your phone).

---

## 3. Bus protocol (what the sidecar speaks)

Open one WebSocket to `ws://127.0.0.1:7895/bus`. Exchange JSON frames.

**Client → hub:**
```jsonc
{ "op": "subscribe",   "topics": ["agent.*", "ui.*"] }
{ "op": "unsubscribe", "topics": ["ui.*"] }
{ "op": "publish",     "event": { "type": "command.focus_agent", "source": "rules-engine", "data": { "sessionId": "…" } } }
{ "op": "call",        "id": "c1", "method": "notifications.post", "params": { "title": "…", "body": "…" } }
{ "op": "register",    "methods": ["rules.list"] }   // only if the plugin provides capabilities (optional)
```

**Hub → client:**
```jsonc
{ "op": "hello" }
{ "op": "subscribed", "topics": ["agent.*","ui.*"] }
{ "op": "event",  "event": { "id":"ev-12", "type":"agent.state_changed", "source":"claudemon", "time":"…", "data":{…} } }
{ "op": "result", "id": "c1", "result": { … } }      // reply to your call
{ "op": "error",  "id": "c1", "error": "no provider for notifications.post" }
```

- **Event envelope:** `{ id, type, source, time, data }`. The hub stamps `id`/`time`.
- **Topic patterns:** exact (`agent.state_changed`), namespace wildcard (`agent.*`),
  or `*` (everything).
- **Calls** are request/reply correlated by your chosen `id`. For fire-and-forget
  actions you can ignore the `result`. Capabilities are provided by the Electron
  `main` process — so a call errors with `no provider` if the app side isn't
  connected; handle that gracefully.

---

## 4. The rule vocabulary: what's actually on the bus today

### Events you can trigger on

| Event `type` | `source` | `data` fields | Meaning |
|---|---|---|---|
| `agent.state_changed` | `claudemon` | `sessionId`, `hookEvent`, `mode`, `cwd` | An agent transitioned. Edge-triggered. |
| `ui.pane.opened` / `ui.pane.closed` / `ui.pane.focused` | `workspacer.ui` | `paneId`, `type`, `workspaceId`, `tabId` | UI pane lifecycle/focus |
| `ui.tab.focused` / `ui.workspace.focused` | `workspacer.ui` | `tabId` / `workspaceId` | Focus changes |
| `plugin.loaded` / `plugin.unloaded` | `hub` | manifest / `{id}` | Plugins (un)installed |
| `sidecar.running` / `sidecar.healthy` / `sidecar.crashed` / … | `supervisor` | `name`, `state`, `pid` | Plugin sidecar lifecycle |
| `command.*` | various | — | Commands others publish (you can match these too) |

**`agent.state_changed` decoding** (the workhorse):
- `mode` ∈ `unknown` | `input` | `responding` | `approval` | `question` | `stopped`
  - `approval` → waiting on a tool-permission yes/no → **"needs approval"**
  - `question` → asking the user a multiple-choice question → **"needs input"**
  - `input` → back at the prompt → **"idle / your turn / finished"**
  - `responding` → actively working
  - `stopped` → session ended
- `hookEvent` = the raw Claude Code hook name (`SessionStart`, `UserPromptSubmit`,
  `PreToolUse`, `PostToolUse`, `Stop`, `SessionEnd`, `Notification`, …). Match
  `hookEvent == "Stop"` for "finished a turn".

### Capabilities you can call (actions)

| Method | params | returns | notes |
|---|---|---|---|
| `agents.list` | `{}` | `[{ sessionId, cwd, state, model, contextTokens, contextLimit, costUSD }]` | The roster + live cost/context. **Poll this** for cost-threshold rules (see below). |
| `agents.sendMessage` | `{ sessionId, text }` | `{ ok, mode? }` | Send a prompt to an agent (only succeeds when it's at an input prompt). |
| `notifications.post` | `{ title, body }` | `{ ok }` | OS notification. |

### Commands you can publish (actions)

Publish an event with `type: "command.<x>"`; workspacer obeys it:
`command.focus_agent {agentId|sessionId}`, `command.spawn_agent {cwd,name,model}`,
`command.open_pane {paneType}`, `command.open_plugin {type}`, `command.close_pane {paneId}`.

### ⚠️ Cost/usage is NOT an event (yet)

Per-agent **cost/context** live in `agents.list`, not on the bus as events. So a
"cost over $5" rule can't be purely event-driven today — the sidecar must **poll
`agents.list` on a timer** (e.g. every 15–30s) and evaluate threshold rules against
the result. (A future `agent.snapshot` event carrying lightweight usage would make
this event-driven; noted in §12.)

---

## 5. Rule schema

`rules.json` is an array of rules:

```jsonc
{
  "id": "approve-ping",
  "name": "Ping me on approvals",
  "enabled": true,

  // TRIGGER
  "when": {
    "event": "agent.state_changed",        // event type, or a wildcard like "agent.*"
    "match": { "mode": "approval" },        // shallow equality on event.data (AND of all keys)
    "where": [                              // optional richer conditions (AND)
      { "path": "data.cwd", "op": "contains", "value": "worky" }
    ]
  },

  // ACTIONS (run in order)
  "do": [
    { "type": "notify",  "title": "Needs approval", "body": "{{data.cwd}} ({{data.sessionId}})" },
    { "type": "command", "command": "focus_agent", "params": { "sessionId": "{{data.sessionId}}" } }
  ],

  // GUARDS
  "cooldownMs": 5000,                        // min gap between firings of this rule
  "once": false                             // if true, fire at most once per (rule, sessionId)
}
```

- **`when.match`** — shallow key/value equality on `event.data`. All keys must match.
- **`when.where`** — optional list of `{ path, op, value }` for richer checks.
  `path` is dotted into the event (`data.usage.costUSD`). `op` ∈ `eq` `ne` `gt`
  `lt` `gte` `lte` `contains` `regex`. All AND together.
- **Templating** — any action string may contain `{{path}}` (e.g. `{{data.sessionId}}`,
  `{{type}}`), interpolated from the matched event.
- **Cost rules** are special: they trigger off the internal `agents.list` poll, so
  use `"event": "agents.poll"` (a synthetic event the engine emits per agent each
  poll cycle, with `data` = one agent's list entry). See §7.

---

## 6. Action types → bus operations

| `do[].type` | fields | sidecar does |
|---|---|---|
| `notify` | `title`, `body` | `call notifications.post {title, body}` |
| `sendMessage` | `sessionId`, `text` | `call agents.sendMessage {sessionId, text}` |
| `command` | `command`, `params` | `publish { type: "command."+command, data: params }` |
| `emit` | `event` (type), `data` | `publish { type: event, source: "rules-engine", data }` |
| `webhook` | `url`, `method?`, `headers?`, `body?` | HTTP request from the sidecar (ntfy/Slack/Pushover/…) |

All string fields support `{{…}}` templating. `command`/`emit`/`webhook` are
fire-and-forget; `notify`/`sendMessage` ignore the capability result (but log errors,
e.g. `no provider` when the app side is down).

---

## 7. Evaluation semantics

```text
on each bus event ev:
  for rule in rules where rule.enabled:
    if ev.type does not match rule.when.event (pattern): continue
    if not all rule.when.match[k] == ev.data[k]:          continue
    if not all rule.when.where conditions pass:           continue
    key = rule.id + (ev.data.sessionId or "")
    if rule.once and fired.has(key):                      continue
    if now - lastFired[rule.id] < rule.cooldownMs:        continue
    for action in rule.do: run(action, ev)
    lastFired[rule.id] = now; if rule.once: fired.add(key)

every POLL_INTERVAL (e.g. 20s):
  agents = call agents.list
  for a in agents:
    feed a synthetic event { type:"agents.poll", data: a } through the same loop
```

- **Edge-triggered for free:** `agent.state_changed` fires on transitions, so
  "entered approval" is natural — no state diffing needed.
- **Cooldown + once** prevent spam (critical for `agents.poll` cost rules, which
  would otherwise refire every cycle — use `once` keyed by sessionId, or a high
  cooldown).
- **Templating** resolves against the triggering event (`ev`).
- **Composition:** `emit` lets one rule fire a custom event that another rule's
  `when.event` matches — rules-of-rules.

---

## 8. Plugin layout + manifest

```
rules-engine/
├── plugin.json
├── main.go            # sidecar: bus client + rule engine + /rules API + static server
├── go.mod             # requires github.com/coder/websocket
├── rules.json         # default/starter rules (also where edits persist)
└── web/index.html     # the rule-editor webview
```

`plugin.json`:
```json
{
  "id": "workspacer.rules-engine",
  "name": "Rules Engine",
  "apiVersion": "1",
  "server": { "command": "./rules-engine", "args": ["--port", "9120"], "port": 9120, "health": "/health" },
  "install": ["go", "build", "-o", "rules-engine", "."],
  "panes":   [{ "type": "workspacer.rules-engine", "title": "Rules", "icon": "⚙", "path": "/", "scope": "global" }],
  "hotkeys": [{ "id": "open-rules", "default": "ctrl+shift+r", "command": "open-pane:workspacer.rules-engine" }],
  "consumes": ["agent.state_changed", "ui.*"],
  "capabilities": ["agents.list", "agents.sendMessage", "notifications.post"]
}
```

Notes:
- `install` runs once post-clone (the hub runs it in the plugin dir). For a
  zero-build alternative, write the sidecar in Node/Python and set `server.command`
  accordingly (no `install`).
- `scope: "global"` → the Rules pane lives in the Overview workspace (it's
  cross-agent). Persist `rules.json` next to the binary (the plugin dir) or in
  `~/.config/workspacer/rules-engine/`.

---

## 9. Implementation plan

**Sidecar (`main.go`):**
1. Flags: `--port`. Resolve rules file path (alongside binary).
2. HTTP server: `GET /health`, `GET /rules`, `PUT /rules` (replace list), static
   files from `web/` at `/`.
3. Bus client (coder/websocket): connect to `ws://127.0.0.1:7895/bus`, on open send
   `subscribe {topics:["agent.*","ui.*","command.*"]}`, reconnect with backoff.
4. Rule engine: load rules, the eval loop (§7), action runner (§6), template
   interpolation, cooldown/once state. A `call(method, params)` helper that sends a
   `call` frame with a fresh id and (optionally) waits for the `result` by id.
5. Poll loop: every 20s, `call agents.list`, feed `agents.poll` synthetic events.

**Webview (`web/index.html`):** a single page (mirror the bundled
`hub/examples/editor/` webview's style) that `GET /rules` on load, renders the list with enable toggles + delete, an
"Add rule" form (event dropdown, match key/value rows, action builder), and `PUT
/rules` on save. It does NOT need a bus connection — it talks to the sidecar's HTTP
API. (Optionally also subscribe to the bus to show a live "recent firings" log if
the engine `emit`s a `rules.fired` event per action.)

**Milestones:**
1. Sidecar connects + logs every event (prove subscription). 
2. Rule eval + `notify`/`command` actions, hardcoded rules. 
3. `/rules` API + persistence + webhook + sendMessage + poll/cost. 
4. Webview editor. 
5. Starter rules + README; publish to GitHub for install-from-URL.

---

## 10. Starter rules (ship these in `rules.json`)

```jsonc
[
  { "id": "needs-approval", "name": "Jump to agents needing approval", "enabled": true,
    "when": { "event": "agent.state_changed", "match": { "mode": "approval" } },
    "do": [ { "type": "notify", "title": "Agent needs approval", "body": "{{data.cwd}}" },
            { "type": "command", "command": "focus_agent", "params": { "sessionId": "{{data.sessionId}}" } } ],
    "cooldownMs": 4000 },

  { "id": "needs-input", "name": "Notify on questions", "enabled": true,
    "when": { "event": "agent.state_changed", "match": { "mode": "question" } },
    "do": [ { "type": "notify", "title": "Agent has a question", "body": "{{data.cwd}}" } ],
    "cooldownMs": 4000 },

  { "id": "finished", "name": "Notify when an agent finishes a turn", "enabled": false,
    "when": { "event": "agent.state_changed", "match": { "hookEvent": "Stop" } },
    "do": [ { "type": "notify", "title": "Agent finished", "body": "{{data.cwd}}" } ],
    "cooldownMs": 8000 },

  { "id": "over-budget", "name": "Warn when an agent passes $5", "enabled": false,
    "when": { "event": "agents.poll", "where": [ { "path": "data.costUSD", "op": "gt", "value": 5 } ] },
    "do": [ { "type": "notify", "title": "Agent over budget", "body": "{{data.cwd}} — ${{data.costUSD}}" } ],
    "once": true },

  { "id": "phone-on-approval", "name": "Push to phone on approval (edit URL)", "enabled": false,
    "when": { "event": "agent.state_changed", "match": { "mode": "approval" } },
    "do": [ { "type": "webhook", "method": "POST", "url": "https://ntfy.sh/YOUR-TOPIC",
              "body": "Agent in {{data.cwd}} needs approval" } ],
    "cooldownMs": 10000 }
]
```

---

## 11. Testing (headless, no Electron needed)

The bus + a fake provider are enough to test the whole engine without the app:

1. Build & run the hub: `cd services/hub && go build -o hub ./cmd/hub && ./hub --addr 127.0.0.1:7895`.
2. Start the sidecar pointed at it.
3. From a throwaway WS client (Node `ws` or a Go test): connect, **`register`** the
   methods `agents.list` / `notifications.post` / `agents.sendMessage` and answer
   `call` frames (mimic the Electron `main` provider) — see
   `hub/internal/bus/rpc_test.go` and the `*_smoke.mjs` patterns in git history.
4. **`publish`** `agent.state_changed` events with various `mode`s and assert the
   sidecar fires the right actions (your fake provider records the `notifications.post`
   calls; assert `command.focus_agent` was published by subscribing to `command.*`).
5. For cost rules, have the fake `agents.list` return an agent with `costUSD > 5`
   and assert one (not repeated) `notify`.

Mirror the bus-native pattern in the bundled examples (`hub/examples/*`) and the
workspacer-plugins repos.

---

## 12. Open questions / future

- **`agent.snapshot` event:** if workspacer later publishes a lightweight per-agent
  status/usage event (it was scoped but deferred — see the
  `hub-spine-migration-plan` decision: keep the hub *additive + lightweight*), cost
  rules become event-driven and the poll loop can go away.
- **Rules authored by Claude:** once the MCP façade lands (Claude as a bus client),
  Claude could add/inspect rules via a capability — e.g. "set up a rule to notify me
  when this agent finishes."
- **Persistence location & multi-profile:** decide whether rules live in the plugin
  dir or `~/.config/workspacer/`.
- **Safety:** `sendMessage`/`webhook` actions act autonomously — consider a global
  "rules paused" switch and an audit trail (`emit rules.fired`).

---

## References

- `hub/README.md` — bus protocol + plugin status overview.
- `hub/examples/editor/` — bus-native, capability-scoped webview plugin to copy.
- `hub/examples/clock-plugin/` — minimal self-contained plugin (manifest + static webview).
- `hub/internal/bus/` — `bus.go` (frames, `/bus`, `/health`), `rpc.go` (capability router), `bench_test.go` / `rpc_test.go` (test patterns).
- `hub/internal/claudemon/bridge.go` — source of `agent.state_changed` (shows exact payload mapping).
- `hub/internal/plugin/` — manifest schema (`manifest.go`) + installer (`install.go`).
- Capability providers (what `agents.list` etc. return): `src/main/services/hubCapabilities.ts`.
- UI event emitter (`ui.*`) and command listener (`command.*`): `src/renderer/src/hooks/useUiEventBus.ts`, `useUiCommands.ts`.
