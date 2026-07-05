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
               state, and streams conversation transcripts
  hub/         Go control-plane: event bus, process supervisor, capability
               router, plugin system, and an MCP facade (cmd/mcp)
docs/          specs and design notes
```

The desktop app spawns and supervises `claudemon` and `hub` as child
processes. `wks-tui` and remote/web clients connect to the same `hub` bus, so a
session running on one client can be observed and driven from another.

## Quick start

```bash
make install      # install desktop JS deps (root + renderer workspaces)
make dev          # run the desktop app (Vite renderer + Electron, hot reload)
```

`make dev` runs `npm run dev`, which starts the Vite renderer and launches
Electron with hot reload; the app spawns `claudemon` and `hub` as child
processes. `./dev` is a thin wrapper around the same npm script.

Remote sharing is off by default and is a **runtime toggle** in the app (Remote
control → Start sharing) — the hub then binds a token-authed web endpoint whose
URL + token show up in the app's Hub status. To force it on at launch instead
(for testing the web/bridged client), use `make dev-share`, which runs
`npm run dev:share` (sets `WORKSPACER_REMOTE_SHARE=1`).

`claudemon` is built separately — run `make build-claudemon` once (or
`make build`) before the app can spawn Claude sessions.

## Common tasks (from the repo root)

| Command                  | What it does                                        |
| ------------------------ | --------------------------------------------------- |
| `make dev`               | Desktop app in dev mode (Vite + Electron)           |
| `./dev`                  | Same as `make dev` (thin wrapper around `npm run dev`) |
| `make dev-share`         | Desktop app in dev mode with remote sharing forced on |
| `make dev-tui`           | Run wks-tui (debug); builds claudemon first         |
| `make run-tui`           | Run wks-tui (release); builds claudemon + tui first |
| `make build`             | Build all four components                           |
| `make build-hub`         | Build the Go `hub` + `mcp` binaries                 |
| `make build-claudemon`   | `cargo build --release` for claudemon               |
| `make build-tui`         | `cargo build --release` for wks-tui                 |
| `make test`              | Desktop + hub + tui test suites                     |
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
