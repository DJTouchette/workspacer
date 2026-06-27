# workspacer hub

The control-plane daemon for workspacer. It runs **independently of the UI** so
it can broker events between sidecars/plugins, supervise those processes, and
(later) host an MCP facade that lets Claude Code drive workspacer headlessly.

Deliberately separate from `claudemon` (which stays focused on Claude sessions)
— claudemon becomes the *first producer* on this bus, not the bus itself.

## Status

- **M1 event bus** ✅ — pub/sub broker with **non-blocking fan-out** (slow client
  gets events dropped + counted, never stalls others), one bidirectional
  **WebSocket** per client (`/bus`), `/health`.
- **M2 supervisor** ✅ — spawn / health-check / restart-on-crash / graceful stop
  (SIGTERM then SIGKILL), lifecycle reported as `sidecar.*` events.
- **M3 claudemon bridge** ✅ — consumes claudemon's `/events` SSE and re-publishes
  each session update as an `agent.state_changed` event. claudemon is now the
  first producer on the bus.
- **M4 capability router** ✅ — request/reply over the same WS. Providers
  `register` method names; callers `call` them; the hub routes the call to the
  owning provider and the reply back, correlating by a global id. Handles no-
  provider, provider-error, provider-disconnect, and timeout. The hub never
  *executes* a capability — it routes. (This is the seam the MCP facade plugs
  into.) Authorization is a stubbed seam (allow-all) for capability tokens later.

All tested with unit + end-to-end tests, clean under `-race`. `integration`
proves the full event spine (claudemon SSE → bridge → broker → bus → client);
`bus/rpc_test.go` proves the capability round-trip + every failure path.

- **M5 plugin system** ✅ — `plugin.LoadDir` reads `<dir>/<name>/plugin.json`
  manifests (id, server, panes, hotkeys, capabilities, events); `plugin.Manager`
  supervises each plugin's sidecar (via M2), emits `plugin.loaded`/`unloaded`,
  and exposes manifests at `GET /plugins`. Run with `--plugins-dir`. See
  `examples/clock-plugin/` for a working one (a webview pane + a hotkey).
  Plugin panes get the app theme injected as `--wks-*` CSS variables — see
  [docs/plugin-theming.md](docs/plugin-theming.md).

Wired into the app: Electron main spawns the hub, connects as a client,
forwards events to the renderer, **provides** capabilities (`agents.list`,
`agents.sendMessage`, `notifications.post`), and the renderer consumes plugin
contributions — injected **pane types** (rendered as webviews), command-palette
entries, and **hotkeys** (`open-pane:` / `emit:`).

Wire the bridge into the running daemon:

```sh
go run ./cmd/hub --claudemon-events http://127.0.0.1:7891/events
```

- **M6 MCP facade** ✅ — `cmd/mcp` is a standalone MCP server that connects to
  the hub as a capability **caller** and re-exposes each capability as an MCP
  tool, so Claude Code (or any MCP client) can drive workspacer headlessly. It
  is a thin adapter: a tool call becomes a bus `call`, the provider (Electron
  main) executes it, the reply becomes the tool result. The facade never touches
  workspacer state — it routes, exactly like the hub. Reusable client lives in
  `internal/busclient`. See [MCP facade](#mcp-facade) below.

## MCP facade

`cmd/mcp` exposes the hub's capabilities as MCP tools over HTTP. It serves two
transports off the same server — `/mcp` (Streamable HTTP, the current MCP HTTP
transport) and `/sse` (legacy SSE) — plus `/health`.

```sh
# hub first (the bus), then the facade pointed at it
go run ./cmd/hub --addr 127.0.0.1:7895
go run ./cmd/mcp --addr 127.0.0.1:7897 --hub ws://127.0.0.1:7895/bus
# (pass --token / $HUB_TOKEN when the hub requires auth)
```

Attach it to Claude Code via `--mcp-config`:

```json
{
  "mcpServers": {
    "workspacer": { "type": "http", "url": "http://127.0.0.1:7897/mcp" }
  }
}
```

Tools (each maps 1:1 to a hub capability provided by Electron main):

| Tool | Capability | What it does |
| --- | --- | --- |
| `list_agents` | `agents.list` | List running agents + state/usage/pending asks |
| `get_transcript` | `sessions.transcript` | Read a session's transcript |
| `spawn_agent` | `agents.spawn` | Start a new Claude Code agent; returns its sessionId |
| `create_terminal` | `terminals.create` | Open a new shell PTY; returns its sessionId |
| `send_message` | `agents.sendMessage` | Send a prompt to an agent |
| `approve` | `claude.approve` | Resolve a permission prompt (yes/no/always) |
| `answer` | `claude.answer` | Answer an AskUserQuestion picker |
| `signal` | `claude.signal` | Send a signal (SIGINT/SIGTERM/…) |
| `terminal_input` | `sessions.terminalInput` | Write raw bytes into a PTY |
| `notify` | `notifications.post` | Show a desktop notification |

The `spawn_agent` / `create_terminal` tools require the matching capabilities to
be registered by Electron main (`src/main/services/hubCapabilities.ts`); the
session runs headless in claudemon and a desktop pane can attach to it later.

Next milestones: per-method capability tokens (the `SetAuthorize` seam) →
surfacing MCP-spawned sessions as panes in the UI automatically.

## Headless brain (`cmd/brain`)

The capabilities above (`agents.*`, `claude.*`, `sessions.*`) are normally
*provided* by the Electron main process — so they only exist while the desktop
app is running. That's why the TUI bypasses the hub and re-implements a slice of
that logic itself.

`cmd/brain` is a standalone **provider** that fills the gap: it connects to the
hub bus and registers the core agent capabilities headlessly, backing each by
claudemon's HTTP API plus the same profile/argv logic the app and TUI use. Run
it (instead of, or alongside, the desktop app) and any caller — the MCP facade,
the web client, a future thin TUI — gets the surface **without a GUI open**.

```sh
# hub + claudemon already running, then:
go run ./cmd/brain --hub ws://127.0.0.1:7895/bus --claudemon http://127.0.0.1:7891
# (pass --token / $HUB_TOKEN when the hub requires auth)
```

Capabilities registered today:

| Capability | Backed by |
| --- | --- |
| `agents.list` / `sessions.snapshots` | claudemon `GET /sessions` |
| `sessions.snapshot` | claudemon `GET /sessions/:id` |
| `agents.spawn` | profile→argv + claudemon `POST /sessions/spawn` |
| `terminals.create` | shell argv + claudemon `POST /sessions/spawn` |
| `agents.sendMessage` | claudemon `POST /sessions/:id/message`, PTY fallback on 409 |
| `claude.approve` / `claude.signal` | claudemon `POST /sessions/:id/{approve,signal}` |
| `claude.answer` | typed into the PTY (`option`/`text`/`answers[]`), like the app |
| `claude.gate` | claudemon `POST /sessions/:id/gate` |
| `sessions.terminalInput` / `sessions.terminalResize` | claudemon `POST /sessions/:id/{input,resize}` |
| `sessions.transcript` / `sessions.conversation` | claudemon `GET /sessions/:id/{transcript,conversation}` |
| `claude.profiles.list` / `add` / `update` / `remove` | `~/.config/workspacer/claude-profiles.json` |
| `claude.listModels` | static aliases + config defaults + live models from claudemon |
| `config.get` / `reload` / `getPath` / `save` | `~/.config/workspacer/config.yaml` (defaults + deep-merge, like the app) |
| `layouts.list` / `save` / `delete` | `~/.config/workspacer/layouts/*.yaml` |
| `sessions.list` / `load` / `save` / `delete` | `~/.config/workspacer/sessions/*.yaml` (saved workspaces) |
| `library.list` / `save` / `remove` | global/project markdown + `.claude/{skills,agents}` |
| `app.getCwd` / `fs.listDir` / `fs.read` / `fs.write` | the host filesystem |

It reuses the provider pattern from `examples/rivet-bridge`. The endgame is for
every client (app, TUI, web, MCP) to be a thin caller of this one brain, so they
mirror each other by construction instead of duplicating logic across TS/Rust/Go.

**Not yet provided headlessly** (still app-only, deferred): the live PTY/event
streams (`sessions.attachTerminal` & co — the byte stream needs a hub proxy, see
the web client), `analytics.*`, `claude.sessionsForDir`, `fs.{listEntries,watch}`,
`search.project`, `notifications.post`, and supervisor/`mcpFacade` spawn args.
Saved-session `save` persists the blob as given — it skips the desktop's
terminal-cwd enrichment, which needs the GUI's in-process pty→cwd map.

## Protocol

Clients open one WebSocket to `ws://<addr>/bus` and exchange JSON frames.

```
client → hub:   {"op":"subscribe","topics":["agent.*"]}
                {"op":"unsubscribe","topics":["agent.*"]}
                {"op":"publish","event":{"type":"agent.spawned","source":"plugin.x","data":{...}}}

hub → client:   {"op":"hello"}
                {"op":"subscribed","topics":[...]}      // ack
                {"op":"unsubscribed","topics":[...]}
                {"op":"event","event":{...}}            // a delivered event
                {"op":"error","error":"..."}
```

Capabilities (request/reply):

```
provider → hub: {"op":"register","methods":["agents.list","agents.sendMessage"]}
caller   → hub: {"op":"call","id":"req-1","method":"agents.list","params":{...}}
hub → provider: {"op":"call","id":"<global>","method":"agents.list","params":{...}}
provider → hub: {"op":"result","id":"<global>","result":{...}}   // or op:"error"
hub → caller:   {"op":"result","id":"req-1","result":{...}}      // or op:"error"
```

Event envelope: `{ id, type, source, time, data }`. The hub stamps `id`/`time`
if the publisher leaves them blank.

Topic patterns: exact (`agent.spawned`), namespace wildcard (`agent.*`), or all
(`*`).

## Run / test

```sh
go run ./cmd/hub --addr 127.0.0.1:7895
go test ./...
go test -race ./...
```

The single bidirectional connection (vs claudemon's SSE-down/POST-up split) is
intentional: publish and subscribe share one pipe, which keeps plugins simple.
