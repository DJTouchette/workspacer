---
title: workspacer serve CLI (headless server launcher)
tags: [hub, cli, headless-server, process-supervision, auth-token, remote, ports]
related_paths:
  - "services/hub-rs/src/cli/mod.rs"
  - "services/hub-rs/src/cli/serve.rs"
  - "services/hub-rs/src/cli/parent.rs"
  - "services/hub-rs/src/cli/identity.rs"
  - "services/hub-rs/src/cli/admin.rs"
  - "services/hub-rs/src/cli/install.rs"
  - "services/hub-rs/src/backend.rs"
  - "services/hub-rs/CLI_MIGRATION.md"
owner: Damien Touchette
last_reviewed: 2026-09-30
---

# Workspacer headless launcher CLI

## Commands and ownership

`services/hub-rs/src/cli/mod.rs` dispatches serve, plugin dev, status,
token, jobs, fleet, install-cli and help. The installed command is `workspacer`;
its shipped executable is `workspacer-rust`.

Full `serve` owns the session engine, bus, shared desktop services and optional MCP
facade in one Rust process. It does not launch separate hub, brain, MCP or private
Node companion executables. Legacy child-binary override flags fail explicitly.
Electron has a separate topology: it owns its daemon and registers its desktop
capabilities with a control plane that it either owns or adopts.

## Planning and startup

Put serve-specific flags after the subcommand, for example
`workspacer serve --data-dir /absolute/state`. The planner validates roles, ports,
paths and identity before startup. Changing either daemon port in full mode
requires an explicit `--claudemon-db-path`. Explicit database paths win; otherwise
absolute XDG data or the selected home supplies the default. An alternate port
never silently selects a shared default database.

`--hub-only` starts the control plane without owning an engine. Optional
`--external-claudemon[=URL]` requires explicit hub-only mode and validates the
borrowed daemon's maintenance health identity before starting. A bare flag uses
the configured loopback API port. This integration does not own the borrowed
process, hooks, database or lifetime. A control plane can also start without a
borrowed daemon; external capability providers may register later.

Full mode initializes hooks through the claudemon library, bounded to 15 seconds,
unless `--no-claudemon-init` is selected. Failure warns and continues. It does not
spawn `claudemon init`, and healthy APIs alone do not establish hook installation.
The owned engine binds loopback; `--host` selects the hub listener. Startup errors
clean up the owned graph. The readiness check verifies MCP service identity,
Rust implementation, listener, expected hub and initial catalog readiness.

Ready output includes endpoints and a pairing credential. JSON mode keeps the
banner structured; `--quiet` suppresses both banner formats. Avoid treating a
ready banner as proof that every later external provider has registered.

## Shutdown and ownership

Interrupt or Unix SIGTERM initiates owned backend shutdown. A second signal may
force exit. With `WORKSPACER_PARENT_PID` set, the launcher also watches the declared
parent and stdin EOF; ordinary foreground stdin EOF without that opt-in is not a
shutdown request. Unexpected backend failure/stoppage produces an error.

There is no legacy sibling-process restart loop in this launcher. Plugin sidecar
supervision has a separate owner; see [process supervision](hub-process-supervision.md).
A borrowed daemon remains alive when the CLI exits. Similarly, quitting Electron
must preserve an adopted hub while removing Electron's registered capabilities;
the surviving hub's tool catalog should reflect its remaining providers.

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

The core standalone runtime does not require Node or `desktop-host.cjs`.
Optional plugin sidecars and external provider CLIs can have independent runtime
requirements. Electron still supplies its public JavaScript services; those are
not the retired private companion.

## Verification

`services/hub-rs/CLI_MIGRATION.md` maps retained launcher behavior and intentional
ownership changes to production files and tests. Rust `tests/cli.rs`,
`tests/backend_owner.rs`, `tests/backend_hooks.rs` and `tests/shutdown.rs` exercise
planning, real temporary-state startup, identity, borrowed ownership and shutdown.

Packaged service and Electron ownership smokes supply separate artifact evidence.
Do not infer cross-platform execution from Linux results, or installed GUI
behavior from a backend harness. The cutover ledger and exact CI/release receipts
remain the authority for completed gates; this source review does not certify them.
See [deployment](fly-node-deploy.md) for image and artifact contracts.
