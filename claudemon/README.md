# claudemon

Standalone observability daemon for Claude Code sessions. Ingests hook events,
keeps live session state in memory, and exposes it over HTTP + Server-Sent
Events so any UI (TUI, web, mobile, tray app) can be a thin client.

This is **v0.1, a working scaffold** — hook ingestion, session state, and the
REST/SSE API are real; the PTY wrapper (`claudemon wrap`) and settings.json
auto-merge (`claudemon init`) are stubbed.

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

```
# In one shell:
claudemon serve

# In another:
curl -X POST http://127.0.0.1:7890/hook \
  -H 'content-type: application/json' \
  -d '{"event":"SessionStart","session_id":"demo","cwd":"/tmp"}'

curl http://127.0.0.1:7891/sessions
# [{"session_id":"demo","status":"active",...}]

# Live updates:
curl -N http://127.0.0.1:7891/events
```

## API surface (v0.1)

| Method | Path                | Purpose                              |
|--------|---------------------|--------------------------------------|
| POST   | `/hook` (7890)      | Hook ingress from Claude Code        |
| GET    | `/sessions`         | List all known sessions              |
| GET    | `/sessions/:id`     | Single session state                 |
| GET    | `/events`           | SSE stream of session updates        |
| GET    | `/health`           | Liveness                             |

Planned (require `claudemon wrap`):

| Method | Path                       | Purpose                       |
|--------|----------------------------|-------------------------------|
| POST   | `/sessions/:id/input`      | Write bytes to claude stdin   |
| POST   | `/sessions/:id/signal`     | Send a signal (SIGINT, etc.)  |
| POST   | `/sessions/:id/resize`     | Resize the PTY                |
| GET    | `/sessions/:id/stream`     | SSE byte stream from the PTY  |

## Layout

```
src/
  main.rs                 CLI entry
  cli.rs                  clap subcommands
  daemon/
    mod.rs                axum servers, lifecycle
    hook.rs               POST /hook
    api.rs                /sessions, /events
    init.rs               `claudemon init` (stub)
  session/
    state.rs              SessionState, HookEvent, status machine
    store.rs              In-memory store + broadcast channel
    transcript.rs         JSONL transcript reader (stub)
  wrapper/
    mod.rs                PTY wrapper (stub)
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
