# claudemon

Standalone observability daemon for Claude Code sessions. Ingests hook events,
keeps live session state in memory, and exposes it over HTTP + Server-Sent
Events so any UI (TUI, web, mobile, tray app) can be a thin client.

v0.2: hook ingestion, session state, REST/SSE API, and the **PTY wrapper
with bidirectional input + transcript reading** are all real. Settings.json
auto-merge (`claudemon init`) and the TUI client (`claudemon watch`) are
still stubs.

## Why

Today, monitoring a `claude` CLI session means watching the terminal it runs in.
That breaks down the moment you have:

- More than one session running in parallel
- A session running on a remote host you SSH'd into
- Long unattended runs you'd like notifications for
- An approval prompt you want routed somewhere other than the terminal

`claudemon` runs as a background service. Claude Code's hooks POST to it; any
client subscribes and gets a live view. Optional PTY-wrapper mode (planned)
lets clients send input back — approve/deny prompts, inject new prompts,
Ctrl-C — from anywhere.

## Architecture

```
       ┌─────────────────────┐
       │   claude CLI        │   user runs this normally,
       │   (any terminal)    │   in any terminal, anywhere
       └──────────┬──────────┘
                  │  POST http://127.0.0.1:7890/hook
                  ▼
       ┌─────────────────────┐
       │  claudemon daemon   │ ── reads ~/.claude/projects/*/*.jsonl
       │  - hook listener    │    (planned)
       │  - session store    │
       │  - REST + SSE API   │ ◄──── any client
       └─────────────────────┘
                  ▲
                  │  POST /sessions/:id/input  (planned, needs `wrap`)
                  │
       Clients: TUI, web UI, tmux statusline, menu-bar, Workspacer, ...
```

## Build

```
cargo build --release
```

The binary lands at `target/release/claudemon`.

## Run

```
claudemon serve
```

Defaults: hook ingress on `127.0.0.1:7890`, API on `127.0.0.1:7891`.
Override with `--hook-port` / `--api-port` / `--host`.

## Wire up Claude Code

```
claudemon init
```

Prints the JSON snippet to merge into `~/.claude/settings.json`. (Auto-merge
is the next thing to land.)

## Try it

### Hook-only mode (observe Claude Code without changing how you launch it)

```
# Terminal 1:
claudemon serve

# Terminal 2:
curl -X POST http://127.0.0.1:7890/hook \
  -H 'content-type: application/json' \
  -d '{"event":"SessionStart","session_id":"demo","cwd":"/tmp"}'

curl http://127.0.0.1:7891/sessions
# [{"session_id":"demo","status":"active",...}]

curl -N http://127.0.0.1:7891/events    # live updates
```

### Wrapped mode (send messages and stream output)

```
# Terminal 1:
claudemon serve

# Terminal 2 — wrap any interactive program (claude, bash, cat, etc.):
claudemon wrap -- claude

# Terminal 3 — drive it from anywhere:
SID=$(curl -s http://127.0.0.1:7891/sessions | jq -r '.[0].session_id')

# Send a message:
curl -X POST http://127.0.0.1:7891/sessions/$SID/input \
  -H 'content-type: application/json' \
  -d '{"text":"refactor this for me"}'

# Read everything the session has produced:
curl http://127.0.0.1:7891/sessions/$SID/output

# Stream new bytes live (each SSE frame is base64-encoded PTY bytes):
curl -N http://127.0.0.1:7891/sessions/$SID/stream

# Pull the parsed conversation from Claude's JSONL transcript on disk:
curl http://127.0.0.1:7891/sessions/$SID/transcript

# Send Ctrl-C:
curl -X POST http://127.0.0.1:7891/sessions/$SID/signal \
  -H 'content-type: application/json' -d '{"signal":"SIGINT"}'
```

## Session modes

The daemon tracks what Claude is doing as a single `mode` field, driven by
hook events. When Claude is paused waiting on the user, the session state
also carries a `pending` payload with the structured content the client
needs to render a picker.

| Mode         | Set by                              | Meaning                                              |
|--------------|-------------------------------------|------------------------------------------------------|
| `unknown`    | initial / wrapper-only registration | Before any hook fires (first-run pickers, OAuth, etc.) |
| `input`      | `SessionStart`, `Stop`              | Chat prompt is open, accepting a user message        |
| `responding` | `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, subagents | Claude is working — streaming, thinking, or running a tool |
| `approval`   | `PermissionRequest`                 | Permission picker up. `pending.kind == "approval"`.  |
| `question`   | `PreToolUse` for `AskUserQuestion`  | Assistant is asking the user. `pending.kind == "question"` with the questions array. |
| `stopped`    | `SessionEnd`                        | Session ended                                        |

`approval` and `question` both override `responding` and stick until a
resolving event (`PostToolUse`, `Stop`, `SessionEnd`) clears `pending`.

Mode + pending appear in `/sessions`, `/sessions/:id`, and every
`session.update` SSE frame, so a client can react to transitions without
polling.

### `pending` shapes

```jsonc
// mode == "approval"
"pending": {
  "kind": "approval",
  "tool": "Bash",
  "summary": "Run `rm -rf /tmp/scratch`?",
  "raw": { ... full hook payload ... }
}

// mode == "question"
"pending": {
  "kind": "question",
  "questions": [
    {
      "question": "Which library should we use for dates?",
      "header": "Library",
      "multi_select": false,
      "options": [
        { "label": "date-fns", "description": "Functional, tree-shakeable" },
        { "label": "dayjs",    "description": "..." }
      ]
    }
  ],
  "raw": { ... }
}
```

## API surface

| Method | Path                          | Purpose                                          |
|--------|-------------------------------|--------------------------------------------------|
| POST   | `/hook` (7890)                | Hook ingress from Claude Code                    |
| GET    | `/sessions`                   | List all known sessions                          |
| GET    | `/sessions/:id`               | Single session state (includes `mode`)           |
| POST   | `/sessions/:id/message`       | **Send chat message — requires mode=`input`**    |
| POST   | `/sessions/:id/approve`       | **Resolve permission picker — requires mode=`approval`** |
| POST   | `/sessions/:id/answer`        | **Answer Claude's question — requires mode=`question`** |
| POST   | `/sessions/:id/input`         | Raw escape hatch: bytes (`text` or `bytes_b64`)  |
| POST   | `/sessions/:id/signal`        | Deliver a signal (`{"signal":"SIGINT"}`)         |
| GET    | `/sessions/:id/output`        | Snapshot of buffered PTY output                  |
| GET    | `/sessions/:id/stream`        | SSE of live PTY bytes (base64-encoded frames)    |
| GET    | `/sessions/:id/transcript`    | Parsed conversation from `~/.claude/projects/.../*.jsonl` |
| GET    | `/events`                     | SSE stream of session state updates              |
| WS     | `/wrapper/:id`                | Wrapper registration + bidirectional control     |
| GET    | `/health`                     | Liveness                                         |

Mode-gated endpoints return HTTP **409 Conflict** with the current mode in
the body when called at the wrong time:

```
$ curl -X POST .../sessions/$SID/message -d '{"text":"hi"}'
HTTP 409
{"error":"session is not accepting chat input","mode":"responding","expected":"input"}
```

### `/message`

```
POST /sessions/:id/message
{ "text": "refactor this for me" }
```

Appends `\r` automatically if the text doesn't already end in a line
terminator. Only succeeds when `mode == input`.

### `/approve`

```
POST /sessions/:id/approve
{ "decision": "yes" }   // → sends "1\r"  (first picker option)
{ "decision": "always" }// → sends "2\r"
{ "decision": "no" }    // → sends "3\r"
{ "option": 2 }         // explicit option override (1–9)
```

The picker layout varies between Claude releases — `option` is the escape
hatch when the convention drifts.

### `/answer`

For `AskUserQuestion`. The client reads `pending.questions` off the session
state and submits one of three forms:

```
POST /sessions/:id/answer
{ "option": 2 }                       // pick option 2 of the (only) question
{ "text": "some custom answer" }      // free-text (when the picker has Other)
{ "answers": ["1", "Custom value"] }  // multi-question: one entry per question,
                                      // sent back-to-back with \r between
```

## Layout

```
src/
  main.rs                 CLI entry
  cli.rs                  clap subcommands
  protocol.rs             Shared wrapper<->daemon WS message enum
  daemon/
    mod.rs                axum servers, lifecycle
    hook.rs               POST /hook
    api.rs                Session REST + SSE endpoints
    wrapper_ws.rs         WebSocket endpoint for wrappers
    init.rs               `claudemon init` (stub)
  session/
    state.rs              SessionState, HookEvent, status machine
    store.rs              In-memory store + wrapper handles + byte buffers
    transcript.rs         JSONL transcript reader
  wrapper/
    mod.rs                PTY wrapper orchestration
    pty.rs                PTY spawn + blocking I/O bridges
```

## Roadmap

- [ ] `claudemon init` actually merges into `~/.claude/settings.json`
- [ ] `claudemon wrap` — PTY wrapper, byte mirroring, input relay
- [ ] JSONL transcript ingestion (replay past sessions on startup)
- [ ] `claudemon watch` TUI
- [ ] Auth token for non-loopback API binds
- [ ] systemd / launchd unit files

## License

MIT
