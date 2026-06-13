# Workspacer

A horizontal-scroll workspace for agent-driven development. Workspacer is a
monorepo: two client apps over a shared Go control-plane, plus a Rust daemon
that supervises Claude Code sessions.

## Layout

```
apps/
  desktop/     Electron + React desktop app (the GUI client)
  tui/         wks-tui — Rust terminal client over the hub bus
services/
  claudemon/   Rust daemon: ingests Claude Code hook events, owns session
               state, parses transcripts
  hub/         Go control-plane / event bus + MCP facade — plugins and remote
               clients broker events here
docs/          specs and design notes
```

The desktop app spawns and supervises `claudemon` and `hub` as child
processes; `wks-tui` and remote/web clients connect to the same `hub` bus.

## Quick start

```bash
make install      # install desktop JS deps
make dev          # run the desktop app (Vite + Electron)   — or: ./dev
```

`make dev` (and `./dev`) auto-build the `hub` binary first; the running app
builds/locates `claudemon` and `hub` from `services/` automatically.

## Common tasks (from the repo root)

| Command                  | What it does                                        |
| ------------------------ | --------------------------------------------------- |
| `make dev` / `./dev`     | Desktop app in dev mode (hot reload)                |
| `make build`             | Build all four components                           |
| `make build-hub`         | Build the Go hub + mcp binaries                     |
| `make build-claudemon`   | `cargo build --release` for claudemon               |
| `make build-tui`         | `cargo build --release` for wks-tui                 |
| `make test`              | Desktop + hub test suites                           |
| `make package`           | Build daemons + produce desktop installers          |
| `make clean`             | Remove build artifacts across all components        |

Each component also builds independently from its own directory — see the
per-component READMEs (`apps/desktop/README.md`, `apps/tui/README.md`,
`services/claudemon/README.md`, `services/hub/README.md`).

## Toolchains

Pinned via [`mise`](https://mise.jdx.dev) (`mise.toml`): Go 1.25, Node 22.
Rust is via the standard `cargo`/`rustup` toolchain.

## License

MIT
