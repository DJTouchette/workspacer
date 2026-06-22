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
make dev          # run the desktop app with remote sharing on
```

`make dev` runs `npm run dev:share`, which builds the `hub` binary, starts the
Vite renderer, launches Electron with hot reload, and enables remote sharing
(the hub binds a token-authed web endpoint — the URL + token show up in the
app's Hub status). Use `./dev` for the same dev loop *without* remote sharing.

`claudemon` is built separately — run `make build-claudemon` once (or
`make build`) before the app can spawn Claude sessions.

## Common tasks (from the repo root)

| Command                  | What it does                                        |
| ------------------------ | --------------------------------------------------- |
| `make dev`               | Desktop app in dev mode + remote sharing            |
| `./dev`                  | Desktop app in dev mode (no remote sharing)         |
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
