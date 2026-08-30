---
title: "Fly box images (deploy/fly): node + hub, build stamps, boot state and the credential surface"
tags: [deploy, fly, node, hub, docker, image, bootstrap, runbook, build-stamp, release-artifact, token-leak, jobs, boot, entrypoint, install]
related_paths:
  - "deploy/fly/node/Dockerfile"
  - "deploy/fly/node/bootstrap.sh"
  - "deploy/fly/node/entrypoint.sh"
  - "deploy/fly/node/verify-image.sh"
  - "deploy/fly/node/RUNBOOK.md"
  - "deploy/fly/hub/entrypoint.sh"
  - "deploy/fly/hub/bootstrap.sh"
  - "deploy/fly/fetch-release.sh"
  - "deploy/fly/write-build-stamp.sh"
  - "deploy/fly/test-fetch-release.sh"
  - "deploy/fly/preflight.sh"
  - "services/hub/cmd/brain/lastexit.go"
  - "services/hub/cmd/brain/main.go"
  - "services/hub/internal/redact"
  - "services/hub/internal/jobs/scheduler.go"
  - "services/hub/cmd/hub/main.go"
owner: Damien Touchette
last_reviewed: 2026-08-29
---

# Fly box images (deploy/fly): node + hub, build stamps, boot state and the credential surface

## Overview

`deploy/` had no context doc; this one collects what was learned running the two
Fly images in anger during 2026-08. A **worker node** (`deploy/fly/node`) runs
claudemon + `brain --hub <remote>` in provider-attach topology and starts **no
local hub**; the **hub box** (`deploy/fly/hub`) runs the bus. Both are built from
a shared preflight/stamp/fetch toolchain under `deploy/fly/`. Everything below
was observed on real machines, not inferred — where a specific machine is named,
that is the evidence.

## The base node image installs ONLY Claude Code

`deploy/fly/node/Dockerfile` has exactly one agent-CLI install
(`npm install -g @anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}`). There is no
codex, opencode or pi install anywhere in `deploy/`. Verified three ways: a
repo-wide grep for install verbs returns only that line; `verify-image.sh`
asserts nothing about other agent binaries; and `bootstrap.sh` merely
PRE-CREATES `$WKS_HOME/.codex` ("if the codex provider is used") — a
volume-persistence provision, not evidence of an install.

**This contradicts a widely-held belief** that "codex-cli was added to wks-node".
It was not. A plan that says "add provider X the way codex was added" is planning
against a change that does not exist and will size the work wrongly.

To add a provider to a node: (1) add its persistent dotfile dir to
`bootstrap.sh`'s dir list next to `.claude`/`.codex`; (2) install the CLI in a
**downstream** Dockerfile layer using the base's `/usr/local` npm prefix, never
under `$HOME` (anything under `$HOME` is shadowed the instant the Fly volume
mounts at boot); (3) re-run `verify-image.sh`, which is mandatory and must stay
last. `example.Dockerfile` + `BASE_IMAGE.md` establish the downstream-layer rule.

## Version identity: the build stamp is the only honest answer

`workspacer`, `hub` and `brain` have **no `--version` flag**, and
`claudemon --version` prints the Cargo version `0.1.0`, which has not moved in
the life of the project. So nothing on a deployed box could say which commit it
ran until `build-stamp` (key=value, installed at
`/usr/local/share/workspacer/build-stamp`, plus `build-stamp.hub` on the hub
layer) was added. Both entrypoints print it on every boot, so `fly logs` answers
it without a shell; `verify-image.sh` asserts it; `install=source|release`
records provenance.

`--build-arg WKS_INSTALL=artifact` (default stays `source`) downloads
`workspacer-server-<platform>.tar.gz` from a GitHub release instead of compiling
— BuildKit skips the gobuild/rustbuild/webbuild stages entirely because the
artifact arm never references them (measured 11s vs ~10min).

Rules that fall out of this:
- **`deploy/fly/write-build-stamp.sh` is the single writer of the stamp format**
  for CI and both Dockerfiles. Never add a fourth writer.
  `fetch-release.sh` copies a bundle's stamp VERBATIM rather than rewriting it.
- **Anything added to the images that must come out of the release bundle also
  has to be added to the release packaging step AND to the Dockerfile's
  `WKS_RELEASE_REQUIRE` list**, or artifact mode ships an image that cannot boot.
  This is not hypothetical: the bundle was missing `mcp`, and the node entrypoint
  dies ~10s into boot without it.
- Three drift guards fail the build loudly — RELEASE DRIFT (bundle downloads but
  claims another tag, the real rolled-`nightly` case), COMMIT DRIFT (right tag,
  wrong sha), STAMP DRIFT (artifact hub on an artifact base from a different
  commit, caught by `verify-image.sh` comparing the two stamps).
- Offline coverage is `deploy/fly/test-fetch-release.sh` (curl over `file://`);
  `./deploy/fly/preflight.sh artifact` is the docker-level proof.

## Boot-state reporting: `last-exit.json` was written but never trusted

`deploy/fly/{node,hub}/entrypoint.sh` write `/data/state/last-exit.json` from a
**trap on INT/TERM**. Exits that run no trap — host eviction, OOM kill of PID 1,
SIGKILL — write nothing, leaving an EARLIER run's record in place, and nothing
could date it: the entrypoint writes a `bootId` field but
`services/hub/cmd/brain/lastexit.go`'s `exitRecord` has fields only for
reason/exitCode/at, so `bootId` is dropped at parse. A node that slept cleanly,
woke, ran a day and was then hard-killed reported the stale `signal-TERM` and
every client showed a node that went to sleep on purpose.

Closed entrypoint-side by renaming the file to `last-exit.consumed.json` once
logged; `readExitRecord` already returns nil for a missing file, which reads as
"no record" rather than "ended cleanly". **Consequence to know:** the rename
happens before the brain starts, so `brain.info` no longer carries an exit reason
at all — the record's home is now the boot log (which reaches `fly logs` via the
previous-boot replay). Putting the `brain.info` half back means moving the
consumption into `lastexit.go`: read, report, THEN rename. `RUNBOOK.md` §8 had to
change from `cat /data/state/last-exit.json` to grepping the boot log, because
the file is gone by the time you look.

See also `domains/config.md` on `services/hub/internal/statelost`, whose Go/shell halves
disagreed and printed a false `STATE LOSS` on every genuinely-first node boot.

## Credential surface: three separate leaks, two closed

1. **Bus dial errors quote the tokened URL.** Closed by
   `services/hub/internal/redact` — see `modules/hub-bus-control-plane.md`.
2. **`brain --help` prints the node's `HUB_TOKEN` as a flag default.** Go's
   `flag` package prints whatever default the flag was constructed with, and
   `services/hub/cmd/brain` seeds `-token` from `$HUB_TOKEN`, so usage output renders
   `-token string … (default "<the node's real 32-char token, plaintext>")`.
   Observed directly on machine `1857645df24448`, 2026-08-26. **Still open.**
   Lower severity than the log leak (reading it needs a shell that already has
   `$HUB_TOKEN` in its environment) but any `brain --help` captured into a
   transcript, an agent's tool output or a CI log publishes the credential. The
   fix is to seed the flag with a placeholder and read the env var after
   `flag.Parse`, or override the usage string. **Audit any other command that
   defaults a flag from a credential env var.** `-hub` has the same shape but its
   value carries no secret.
3. **The node's operator token reaches `/bin/sh` inside the hub process.** The
   node attaches with an operator-tier `HUB_TOKEN`; operator is `trusted`; and
   `jobsTrusted` (`services/hub/cmd/hub/main.go`) is a bare `c.IsTrusted()` with
   nothing narrower. So a node may call `jobs.upsert` then `jobs.run`, and a job
   of kind `shell` reaches `jobs.BusRunner.Shell`
   (`services/hub/internal/jobs/scheduler.go`) — `exec.CommandContext("/bin/sh",
   "-c", command)`, unconfined, in the HUB process's environment, i.e. the one
   holding `$FLY_API_TOKEN`, on the volume holding
   `nodes.json`/`tokens.json`/`remote-token`. The node is BY DESIGN the machine
   that runs agent-written code, so this is a real path from a prompt-injected
   agent to the credential that creates and destroys machines.
   **An empty `--jobs-file` is the off switch** and `deploy/fly/hub/entrypoint.sh`
   now passes `--jobs-file ""`; `if *jobsFile != ""` wraps every `jobs.*`
   `RegisterLocalIdent`. Verified executably both ways inside the image: with the
   empty flag `workspacer jobs list` answers "no provider for jobs.list" (rc 1);
   the same binary with a non-empty `--jobs-file` on another port answers
   normally (rc 0). **Do NOT delete that flag to enable jobs — fix the tier or the
   gate instead.** The durable shape of the gap is in
   `modules/hub-bus-control-plane.md`: the hub has no provider/node identity at
   the HTTP layer.

## MCP on a node needs two tokens, and they are not the same token

The node's MCP facade is real (image builds/copies `mcp`, the entrypoint
supervises it, and the brain mints per-session scoped facade tokens for
`mcpFacade`/`toolScope`/`supervisor` spawns — implemented 2026-08-27). The
remaining operational wrinkle is **token topology**: a provider-scoped
`HUB_TOKEN` is right for `brain`, while the facade's own OUTBOUND bus connection
may need `WKS_MCP_HUB_TOKEN`. Get this wrong and agents authenticate to the local
facade fine and still have their hub calls denied — see the "two distinct
tokens" gotcha in `modules/mcp-tool-facade.md`, which is the same confusion one
layer down.

## `su - wks` (the shell the runbook tells you to use) loses `RUSTUP_HOME`

    su - wks -c 'rustc --version'   -> rustup could not choose a version of rustc
    su   wks -c 'rustc --version'   -> rustc 1.98.0 / cargo 1.98.0

Rust is installed correctly (`/usr/local/rustup/toolchains/...` exists,
`settings.toml` names a default, the image sets
`ENV RUSTUP_HOME=/usr/local/rustup`). A LOGIN shell sources
`~/.profile` → `~/.wks-env`, which rebuilds the environment from the node
`fly.toml [env]` list — it exports `HOME`, `XDG_*`, `GOPATH`, `GOMODCACHE`,
`GOCACHE`, `BUNDLE_PATH`, `npm_config_cache`, `HISTFILE`, `PATH`, `WKS_STATE_DB`
and **no `RUSTUP_HOME`** (nor `CARGO_HOME`). rustup then falls back to
`$HOME/.rustup` = `/data/home/.rustup`, on the volume and empty.

**Agents are unaffected** — the `claudemon` process carries
`RUSTUP_HOME=/usr/local/rustup`, so a dispatched worker can `cargo build`. Go is
unaffected (needs no env var, and `.wks-env` puts it on `PATH`). But every
interactive check in `deploy/fly/*/RUNBOOK.md` uses `su - wks` deliberately (so
`~/.wks-env` is sourced), so **a human verifying the node concludes Rust is
missing when it is present and working for the workloads that matter.** Fix:
add `RUSTUP_HOME` and `CARGO_HOME` to whatever generates `~/.wks-env`, alongside
`GOPATH`. Until then use `su wks -c` (non-login) for Rust checks. Observed on
machine `1857645df24448`, 2026-08-26.
