# Workspacer

**A control plane and IDE for coding agents.** Run a whole fleet of
long-lived coding agents — Claude Code, Codex, and OpenCode — side by side in
one local-first desktop app. Watch every agent's state at a glance, step in
only when one actually needs you, and review and ship the code they write
without leaving the window.

It's not a chat box bolted onto an editor. Workspacer treats agents as
first-class, long-running processes: each one gets its own workspace (tabs,
terminals, a browser, a git review pane, notes), lives in a background daemon
so closing the window never kills it, and reports up into a single control
plane you can also drive from a terminal or your phone.

> Website & docs: open `landing/index.html` for the overview and
> `landing/docs.html` for the full user guide (both are static, no build step).

---

## Why it exists

One agent in a terminal is easy — you sit and watch it. That falls apart the
moment you have three, five, ten going at once. You lose track of which agent
is working, which is blocked on an approval, and which finished ten minutes
ago and is sitting idle burning nothing. Tabbing between terminals, most of the
switching you do doesn't pay off.

Workspacer keeps the ambient state of every agent in front of you so switching
becomes a decision you make, not a reflex you act on out of fear of missing
something:

- **See who needs you.** A per-agent status dot (idle / working /
  needs-approval / done), a live "N need you / N working" header, and OS
  notifications that stay quiet for the agent you're already watching.
- **Act without hunting.** A Triage Inbox collects every approval and question
  across the fleet like an email inbox; jump straight to the next agent waiting
  on you.
- **Close the loop.** When an agent finishes, review its diff, stage, commit,
  and push from a built-in review pane instead of dropping to a shell.

Typical uses: run several agents across different repos at once; babysit a long
refactor while doing other work; spawn a supervisor agent that coordinates the
rest of the fleet; check on a running job from your phone.

## What you get

- **Multi-agent workspaces** — one agent = one long-lived session = one
  workspace, keyed by working directory, with its own tabs and panes.
- **Three agent backends** — Claude Code, Codex, and OpenCode, all landing in
  the same fleet with the same state, telemetry, and approval prompts.
- **A real GUI for each agent** — approve/deny in one key, answer questions
  inline, read diffs as they land, follow a clean work log, attach files by
  drag/paste/picker. (Plus a raw terminal view for Claude.)
- **Pane types** — terminal, browser, git review/diff, per-agent markdown
  notes, a prompt/skill/agent library, analytics, an overview, and plugin panes.
- **Layout your way** — tabs, spatial (pan-zoom canvas), or stacked feed;
  switch without remounting live panes. A Fleet Deck radar sits over all of it.
- **Attention & notifications** — status dots, aggregate counts, jump-to-next,
  and configurable OS notifications.
- **Remote & multi-client** — drive the same fleet from a terminal client
  (`wks-tui`) or a phone/laptop over the network; opt-in, token-authed sharing.
- **Extensible** — a plugin system (drop in a manifest, get a supervised
  sidecar with its own panes) and an MCP facade that exposes the fleet as tools
  a supervisor agent can drive.

For a full, honest feature-by-feature catalog with maturity levels, see
[`docs/features.md`](docs/features.md).

---

## Quick start

```bash
make install          # install desktop JS deps (root + renderer workspaces)
make build-claudemon  # build the session daemon (needed before spawning agents)
make dev              # run the desktop app (Vite renderer + Electron, hot reload)
```

`make dev` starts the Vite renderer and launches Electron with hot reload; the
app spawns and supervises the `claudemon` and `hub` daemons as child processes.
`./dev` is a thin wrapper around the same npm script.

To let Workspacer observe Claude Code agents, wire Claude's hooks once:

```bash
claudemon init                  # merge hooks into ~/.claude/settings.json
claudemon init --dry-run        # preview the merged file, write nothing
```

Codex and OpenCode agents are driven directly and need no hook wiring.

Remote sharing is off by default and is a **runtime toggle** in the app (Remote
control → Start sharing). To force it on at launch for testing, use
`make dev-share` (sets `WORKSPACER_REMOTE_SHARE=1`).

## Architecture at a glance

You launch one thing — the desktop app — but it supervises a small set of
processes so your sessions outlive any window:

```
apps/
  desktop/     Electron + React desktop app — the primary GUI client
  tui/         wks-tui — Rust terminal client over the hub bus
services/
  claudemon/   Rust session daemon: owns sessions/PTYs, runs per-provider
               adapters, streams conversation, usage, and git
  hub/         Go control plane: event bus, process supervisor, capability
               router, plugin system, and an MCP facade (cmd/mcp)
docs/          specs, design notes, and the feature catalog
landing/       the marketing site + user docs (static HTML)
```

The desktop app spawns `claudemon` and `hub` and restarts them if they crash.
`wks-tui`, the web client, and phone/PC clients all connect to the same `hub`
bus, so a session running on one client can be observed and driven from another.

## Common tasks (from the repo root)

| Command                | What it does                                          |
| ---------------------- | ----------------------------------------------------- |
| `make dev`             | Desktop app in dev mode (Vite + Electron)             |
| `./dev`                | Same as `make dev` (wrapper around `npm run dev`)     |
| `make dev-share`       | Desktop app in dev mode with remote sharing forced on |
| `make dev-tui`         | Run wks-tui (debug); builds claudemon first           |
| `make run-tui`         | Run wks-tui (release); builds claudemon + tui first   |
| `make build`           | Build all four components                             |
| `make build-hub`       | Build the Go `hub` + `mcp` binaries                   |
| `make build-claudemon` | `cargo build --release` for claudemon                 |
| `make build-tui`       | `cargo build --release` for wks-tui                   |
| `make test`            | Desktop + hub + tui test suites                       |
| `make package`         | Build daemons + produce desktop installers            |
| `make clean`           | Remove build artifacts across all components          |

Each component also builds independently from its own directory — see the
per-component READMEs (`apps/desktop/README.md`, `apps/tui/README.md`,
`services/claudemon/README.md`, `services/hub/README.md`).

## Toolchains

Pinned via [`mise`](https://mise.jdx.dev) (`mise.toml`): Go 1.25, Node 22. Rust
is via the standard `cargo`/`rustup` toolchain.

---

## Contributing

Contributions are welcome. Start with **[CONTRIBUTING.md](CONTRIBUTING.md)** for
the dev setup, how the codebase is laid out, the test/lint expectations, and the
pull-request flow. TL;DR: fork or branch, `make install && make build`, make your
change with tests, run `make test`, and open a PR against `master`.

Found a security-sensitive issue? See [`SECURITY.md`](SECURITY.md) — please
don't file those as public issues.

## License

Workspacer is **source-available** under the
[Business Source License 1.1](LICENSE).

In plain terms:

- The full source is public — read it, build it, modify it, share your changes.
- **Free to use, including at work**, as long as your organization has **at most
  five (5) total users** of Workspacer.
- **Six (6)+ users needs a commercial license.** So do hosted/managed-service
  resale offerings. Reach out at **djtouchette1993@gmail.com**.
- Each version becomes MIT-licensed four years after it's published.

This is not an OSI "open source" license today, but every version converts to
the fully-open MIT License on its Change Date. The summary above is for
convenience only and is not legal advice — the [LICENSE](LICENSE) file controls.
