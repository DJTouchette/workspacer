# Contributing to Workspacer

Thanks for wanting to help. Workspacer is a monorepo with four moving parts —
an Electron/React desktop app, a Rust session daemon, a Go control plane, and a
Rust terminal client — so this guide gets you from a clean checkout to a green
build and a mergeable pull request.

By contributing, you agree to the terms in [Contributor terms](#contributor-terms)
at the bottom of this file.

## Ground rules

- **Be respectful.** Assume good faith, keep discussion technical, and
  remember there's a person on the other end of the review.
- **Small, focused PRs** get reviewed faster than large ones. If a change is
  big or architectural, open an issue first so we can agree on the shape before
  you write it.
- **Don't file security issues as public issues.** See
  [`SECURITY.md`](SECURITY.md) for how those are handled.

## Prerequisites

Toolchains are pinned with [`mise`](https://mise.jdx.dev) via `mise.toml`:

- **Go 1.25** — the `hub` control plane
- **Node 22** — the desktop app (Electron + React + Vite)
- **Rust** (stable, via `rustup`) — `claudemon` and `wks-tui`

If you use `mise`, run `mise install` to get Go and Node at the pinned
versions. Install Rust separately with `rustup`.

## First build

From the repo root:

```bash
make install          # desktop JS deps (root + renderer workspaces)
make build            # build all four components (desktop, hub, claudemon, tui)
```

Then run the app in dev mode:

```bash
make build-claudemon  # required once before agents can spawn
make dev              # Vite renderer + Electron with hot reload
```

The desktop app spawns and supervises `claudemon` and `hub` for you — you don't
start them by hand. See the [README](README.md#common-tasks-from-the-repo-root)
for the full `make` target list, and the per-component READMEs
(`apps/desktop/`, `apps/tui/`, `services/claudemon/`, `services/hub/`) for
building each piece on its own.

## Where things live

| Path                 | What it is                                                        |
| -------------------- | ---------------------------------------------------------------- |
| `apps/desktop/`      | Electron + React desktop client (the primary GUI)                |
| `apps/tui/`          | `wks-tui`, the Rust terminal client                              |
| `services/claudemon/`| Rust session daemon: sessions, PTYs, provider adapters, git      |
| `services/hub/`      | Go control plane: event bus, supervisor, plugins, MCP facade     |
| `docs/`              | design notes, specs, and the feature catalog (`features.md`)     |
| `landing/`           | the static marketing site + user docs                            |

New to the codebase? Good reading order: [`docs/features.md`](docs/features.md)
(what exists and how mature it is), then `landing/docs.html` (the user guide),
then `landing/build.html` (the architecture and hub-bus protocol).

## Making a change

1. **Branch** off `master` (or fork, if you're external):
   `git checkout -b my-change`.
2. **Write the change with tests.** Match the style and structure of the code
   around it. Add or update tests for anything with runtime behavior.
3. **Format and lint** the components you touched (see below).
4. **Run the tests:** `make test` runs the desktop + hub + tui suites.
5. **Commit** in logical chunks with clear messages (see below).
6. **Open a PR** against `master` describing what changed and why, with a note
   on how you verified it.

### Formatting & linting

Run these for whatever you changed before pushing:

- **Desktop (TS/React):** Prettier is configured (`apps/desktop/.prettierrc`).
  Formatting and type-checks run as part of the desktop workflow.
- **Rust (`claudemon`, `wks-tui`):** `cargo fmt` and `cargo clippy` before you
  commit; keep clippy clean.
- **Go (`hub`):** `gofmt`/`go vet`; keep the build warning-free.

There's a `.git-blame-ignore-revs` at the root — bulk-formatting commits are
listed there so `git blame` stays useful. If you do a repo-wide reformat, add
the commit hash to that file.

### Tests

- `make test` runs desktop + hub + tui.
- Prefer testing behavior end-to-end where a change has a runtime surface, not
  just the happy path.
- If you're changing a daemon or the hub bus, exercise the affected flow, not
  only the unit under it.

### Commit messages

- Keep the subject line short and imperative ("Add review-pane push button",
  not "Added" / "Adds").
- Explain the *why* in the body when it isn't obvious from the diff.
- Reference an issue number when there is one.

### Pull requests

- Target `master`.
- Describe the change, the reasoning, and how you verified it.
- Keep the diff scoped to one concern; split unrelated changes into separate PRs.
- Expect review comments — that's the normal path to merge, not a rejection.

## Reporting bugs & requesting features

- **Bugs:** open an issue with what you did, what you expected, what happened,
  and your OS + which agent backend (Claude Code / Codex / OpenCode) you were
  running. Logs or a minimal repro help a lot.
- **Features:** open an issue describing the use case before writing code, so we
  can talk through the design. `docs/features.md` shows what's already built.

## Contributor terms

Workspacer is distributed under the [MIT License](LICENSE).

By submitting a contribution (a pull request, patch, or any other work) you agree
that:

1. You are legally able to make the contribution, and it is your original work
   (or you have the right to submit it).
2. You license your contribution to the project under the MIT License, the same
   terms as the project as a whole.

If your employer has rights to work you create, make sure you have permission to
contribute before you do. If we ever adopt a formal Contributor License Agreement
(CLA), we'll ask you to sign it; until then, this section is the agreement.

This is not legal advice. If any of the above is a problem for your situation,
open an issue or email **djtouchette1993@gmail.com** before contributing.
