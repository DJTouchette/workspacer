# Rules Engine — a workspacer plugin

A tiny always-on **event → action interpreter** on the hub bus. "When an agent
needs approval, ping me and jump to it." "When an agent finishes, push my phone."

It's a **sidecar (the brain)** plus a **webview (the rule editor)**. The brain
lives in the sidecar so rules keep firing while the editor pane is closed. Full
design: [`hub/docs/rules-engine-plugin.md`](../../docs/rules-engine-plugin.md).

## Layout

```
rules-engine/
├── plugin.json     manifest (pane, hotkey, capabilities)
├── main.go         sidecar: HTTP (/health, /rules, webview) + wiring
├── bus.go          reconnecting hub bus client (subscribe / publish / call)
├── engine.go       rule schema, eval loop, conditions, templating, actions
├── rules.json      starter rules (and where edits persist)
└── web/index.html  the rule-editor webview
```

## How it works

- The sidecar opens one WebSocket to `ws://127.0.0.1:7895/bus`, subscribes to
  `agent.*`, `ui.*`, `command.*`, and evaluates each event against the rule list.
- Rule evaluation runs on a **dedicated worker goroutine** (`engine.run`), fed by
  a channel. This is load-bearing: actions make synchronous capability calls
  (`notifications.post`, `agents.sendMessage`) whose `result` frames are read by
  the bus read-loop — evaluating inline would deadlock the call until timeout.
- Cost/usage isn't on the bus, so the sidecar **polls `agents.list`** every 20s
  (`--poll`) and feeds one synthetic `agents.poll` event per agent through the
  same engine. Use `cooldownMs`/`once` to avoid refiring every cycle.
- The webview talks plain HTTP to the sidecar (`GET`/`PUT /rules`); it needs no
  bus connection.

## Controls: pause + audit trail

- **Global kill-switch.** A `paused` flag stops every rule from firing without
  deleting anything (events are still consumed, just dropped). Toggle it from the
  editor's **Pause/Resume** button, or `PUT /state {"paused":true}`. It's
  persisted to `state.json`, so a paused engine stays paused across restarts.
- **Audit trail.** Each firing is recorded to an in-memory ring buffer (last
  100) exposed at `GET /log`, and emitted on the bus as a `rules.fired` event
  (`{ time, ruleId, ruleName, event, sessionId, actions }`) so dashboards can
  watch it live. The editor's **"Recent firings"** panel polls `/log`.

## HTTP API

| Method + path | purpose |
|---|---|
| `GET /health` | `{status, rules, paused}` |
| `GET /rules` / `PUT /rules` | read / replace the rule list (PUT persists to `rules.json`) |
| `GET /state` / `PUT /state` | read / set the `{paused}` kill-switch (persists to `state.json`) |
| `GET /log` | recent firings, newest-first |
| `GET /` | the rule-editor webview |

## The editor

Each rule has a **Form ⇄ JSON** toggle: the Form view is a structured builder
(trigger event, match key/values, where conditions, an action builder per action
type, cooldown/once); the JSON view is the raw rule for power edits. Switching
syncs both ways. Edits are in-memory until **Save** (`PUT /rules`).

## Rule schema (see the spec §5 for the full grammar)

```jsonc
{
  "id": "needs-approval", "name": "Jump to agents needing approval", "enabled": true,
  "when": { "event": "agent.state_changed", "match": { "mode": "approval" } },
  "do": [
    { "type": "notify",  "title": "Needs approval", "body": "{{data.cwd}}" },
    { "type": "command", "command": "focus_agent", "params": { "sessionId": "{{data.sessionId}}" } }
  ],
  "cooldownMs": 4000
}
```

- `when.match` — shallow equality on `event.data` (AND of all keys).
- `when.where` — `[{ path, op, value }]`, `op` ∈ `eq ne gt lt gte lte contains regex`.
- Actions: `notify`, `sendMessage`, `command`, `emit`, `webhook`. Any string
  field supports `{{dotted.path}}` templating against the triggering event.
- Guards: `cooldownMs` (min gap per rule), `once` (fire at most once per
  rule+sessionId).

## Run / build

```sh
go build -o rules-engine .
./rules-engine --port 9120            # serves editor + /rules; connects to the bus
```

Flags: `--port` (HTTP, default 9120), `--bus` (default `ws://127.0.0.1:7895/bus`),
`--poll` (agents.list interval, default 20s).

## Install into workspacer

Push to a public GitHub repo, then in workspacer: command palette →
"Install from GitHub…" → `owner/repo`. The hub runs the manifest's `install`
(`go build -o rules-engine .`) and supervises the sidecar. For local dev, this
folder already lives under `hub/examples/` (dev seeds plugins from there).

## Testing headlessly

No Electron needed — a fake provider on the bus is enough (spec §11): build & run
the hub, start this sidecar, then from a throwaway WS client `register` the
`agents.list` / `notifications.post` / `agents.sendMessage` methods, `publish`
`agent.state_changed` events, and assert the right actions fire (recorded
`notifications.post` calls, `command.focus_agent` published on `command.*`).
