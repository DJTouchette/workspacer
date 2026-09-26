---
title: workspacer serve CLI (headless server launcher)
tags: [hub, cli, headless-server, process-supervision, auth-token, remote, ports]
related_paths:
  - "services/hub/cmd/workspacer/main.go"
  - "services/hub/cmd/workspacer/serve.go"
  - "services/hub/cmd/workspacer/plan.go"
  - "services/hub/cmd/workspacer/child.go"
  - "services/hub/cmd/workspacer/backoff.go"
  - "services/hub/cmd/workspacer/token.go"
  - "services/hub/cmd/workspacer/tokencmd.go"
  - "services/hub/cmd/workspacer/status.go"
  - "services/hub/cmd/workspacer/install.go"
  - "services/hub/cmd/workspacer/resolve.go"
  - "services/hub/internal/authtoken/authtoken.go"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Workspacer headless launcher CLI

## Commands and ownership

`services/hub/cmd/workspacer/main.go` dispatches serve, plugin dev, status,
token, jobs, fleet, install-cli, and help. It is the process launcher and host
administration entry point, not another implementation of every agent capability.

`serve` owns claudemon, hub, optional MCP facade, and full-scope brain as sibling
children. The current plan starts brain separately with `--scope full`; it does
not ask the hub to supervise brain via `--brain-scope full`. The latter flag is
still used by other launch modes, such as desktop catalog delegation.

## Resolution and startup

`resolveBin` prefers an explicit override, then a sibling of the real executable,
then PATH. `selfDir` resolves the launcher symlink so install-cli does not change
which shipped siblings are found. Missing claudemon/hub is fatal; missing brain
or facade produces a warning and the corresponding reduced stack. An explicit
path is not proof that the file is executable; child startup can still fail.

`bootStack` checks the hook/API/hub ports, plus the MCP port when a facade is
configured, by briefly binding them. It refuses collisions instead of killing an
unknown incumbent. A port can still be claimed between that check and actual bind.

The launcher resolves a DB path before starting daemons. Default daemon ports
can use the derived XDG/home path; changing either claudemon port requires an
explicit `--claudemon-db-path`. Merely choosing another XDG directory does not
bypass the alternate-port guard. The explicit path is the caller’s choice, including
intentional sharing; it is not an automatic unique-store allocation.

Unless `--no-claudemon-init` is set, the plan runs `claudemon init --hook-port N`
before spawning the stack. That step is bounded and idempotent. Failure logs a
warning and startup continues; a healthy API afterwards does not prove hooks
were installed. Claude PTY state and first-message readiness depend on those hooks,
whereas managed adapter state comes through its driver.

Claudemon always binds loopback in this plan; `--host` controls the hub. Health
probing respects a concrete hub bind rather than always dialing loopback.
Daemon health waits have a bounded startup window. The facade’s gate additionally
checks service identity, listen address, expected hub connection and initial plugin
catalog readiness before launching brain with the endpoint. Without a facade binary,
brain starts without an invented MCP URL. Later spawns revalidate facade health.

The ready banner reports endpoints and the pairing credential, in human form or
JSON on stdout; logs stay on stderr in JSON mode. Treat banner output as containing
a credential. Launching brain is not the same as proving its provider registration;
use the status probe for that separate observation.

## Shutdown and restart budget

SIGINT/SIGTERM initiates brain → facade → hub → claudemon shutdown; a second
signal can force exit. The CLI child loop forwards prefixed logs and flushes
partial lines after exit. It supplies the parent-death pipe/PID protocol and
requests graceful termination, with platform-specific force behavior.

Restart delay begins at one second, doubles to 30 seconds, and gives up after ten
attempts unless a sufficiently long run resets the count (one minute). Exhausting
a child’s budget leaves that loop stopped; the foreground launcher may still be
waiting for a signal. Check actual component state instead of trusting an earlier
ready banner. This finite-budget loop differs from the hub’s internal sidecar
supervisor; see [process supervision](hub-process-supervision.md).

## Status and credentials

`status` probes claudemon and hub health, then uses **brain.info** to distinguish a
registered brain from a bus with no provider. The retired app.getCwd probe could
be answered by desktop and is not the current implementation. The brain line is
informative; the command’s success criteria for daemon/hub health must not be
mistaken for proof that every provider capability is present.

The host token is stored under the shared Workspacer config directory as
`remote-token`. A missing token amid other state is suspected identity loss:
serve refuses unless the credential is supplied or `--allow-new-token` explicitly
accepts a new identity. Deleting the token is not ordinary recovery preserving
existing pairings. Scoped records live separately in tokens.json.

`token create/list/revoke` operate on those scoped records. Revocation accepts a
unique prefix of at least eight characters or the full token and refuses ambiguity.
The running bus periodically revalidates existing scoped connections; the CLI’s
older receipt text about already-open connections is not the current bus behavior.
Host-token rotation and plugin-token revocation are separate mechanisms.

`token facade-authority` changes identity-delegation authority on an exact unique
infrastructure label. It accepts a dedicated operator service token, not a session
or Remote Control pairing, and does not expose a self-granting bus method. This
is separate from ordinary method scope and from authenticated-host-only admin
rights. Inspect its validation before provisioning a facade’s outbound credential.

## Jobs, fleet, and installation

`jobs` is the owner-credential administration path for scheduled specs; see
[hub jobs](hub-jobs.md). `fleet quiescence` reports the hub’s fleet-rest verdict
with an exit-code contract for scripts. Plugin dev builds on the common launch
flags and adds its reload workflow; it is not a separate set of daemon defaults.

`install-cli` symlinks on Unix or copies on Windows and avoids copying over itself.
Binary discovery follows the actual target after symlink resolution. It does not
install every external provider CLI, sign in accounts, or provision a cloud node.

The brain can start a private Node companion from desktop-host.cjs beside it, or
WKS_DESKTOP_HOST. `make build-cli` builds the Go/Rust binaries but not that bundle.
From `apps/desktop`, `npm run build:desktop-host` builds/copies it; packaging must
carry it and Node for the shared service surface. See
[headless services](headless-desktop-services.md).

## Verification

From `services/hub`, `go test ./cmd/workspacer` covers planning, resolution,
status, token/CLI handling, DB guards, init and child fixtures. The documented
checks use controlled files/processes; they do not start a production server.
Pair plan tests with actual health/registration observations for operational
changes, and preserve explicit skips rather than treating them as executed
cross-platform checks. Fly image/stamp concerns live in
[deployment](fly-node-deploy.md).
