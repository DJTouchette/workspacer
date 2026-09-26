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
  - "deploy/fly/combined/Dockerfile"
  - "deploy/fly/combined/entrypoint.sh"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Fly images: topology, artifacts, and boot evidence

## Choose the topology first

| Image | Runtime role |
| --- | --- |
| `deploy/fly/node` | Claudemon and a brain attached to an external hub; local MCP facade when enabled |
| `deploy/fly/hub` | Hub/control plane over the shared base |
| `deploy/fly/combined` | Hub-layer derivative whose entrypoint runs a local `workspacer serve` stack |

Do not apply a worker-node’s remote-provider instructions to the combined stack.
The combined entrypoint bootstraps the mounted home, drops to the wks user, and
runs serve on port 7895 with the packaged web application and trusted front-end
hostname. Its JSON ready banner contains the pairing credential and is redirected
to `/data/logs/serve-ready.json` under a restrictive umask rather than Fly logs.

These are source contracts. A historical machine observation is not evidence
that a currently deployed image contains this checkout.

## Base versus downstream tools

The node runtime starts from a Node 22 image and installs Workspacer runtime and
operational utilities plus Claude Code. Its built-in agent-CLI install is Claude;
creating a .codex directory is not installing Codex. Project compilers/toolchains
and other provider CLIs belong in downstream images. The current runtime base
does not install the Rust toolchain merely because Rust was used in a build stage.

Install executable tooling outside the mounted home so the volume does not hide
it. Preserve required per-user state/cache directories through bootstrap and run
`verify-image.sh` after downstream additions. Inspect the actual image/toolchain
rather than repeating a former custom machine’s rustup version.

The generated login `.wks-env` exports a selected environment, not every variable
from the container. A downstream rustup install that depends on RUSTUP_HOME or
CARGO_HOME must also preserve those in its login-shell path; their absence from
the base’s generated list is not proof Rust is installed but broken everywhere.

The runtime Dockerfiles copy named binaries/artifacts. When adding a companion
such as `desktop-host.cjs`, check both source-copy and artifact-copy paths and
release packaging; a node having `node` on PATH is not proof the companion bundle
was installed. See [headless desktop services](headless-desktop-services.md).

## Source and artifact installations

`WKS_INSTALL` selects source or artifact build stages. Artifact mode downloads a
release server bundle through `deploy/fly/fetch-release.sh`; it must satisfy the
requested release tag, optional expected commit, required file list and executable
file list. Branch/tag names can move, especially nightly; recording the requested
tag alone does not identify the downloaded bytes.

`deploy/fly/write-build-stamp.sh` defines the stamp format used by image/CI paths:
component, install provenance, version/tag/commit, build time, platform and run.
The fetcher copies the bundle stamp rather than pretending it built those bytes.
The runtime base stamp and hub-layer stamp live under
`/usr/local/share/workspacer`; image verification checks incompatible provenance.

A printed archive SHA-256 is identification evidence, not authenticity verification
against an independently published checksum. Required-file and stamp checks do
not prove arbitrary archive content is trustworthy or that the image boots.

Changing bundle contents requires aligned release packaging, required-file lists,
Docker COPY paths and verification. Artifact/source parity must be tested, not
inferred from a successful source build. Historical elapsed build times are not
current performance guarantees.

## Boot records and prior exits

Node/hub entrypoints record exits through their trap paths. SIGKILL, host eviction
or a killed PID 1 can bypass those paths, so a missing record means unknown.
At boot they log the prior `last-exit.json` and rename it to a consumed file.
That prevents an earlier clean exit from being presented as the latest run’s
outcome after a later unrecorded crash.

`services/hub/cmd/brain/lastexit.go` reads the remaining record once and does not
carry the entrypoint bootId in its parsed struct. Since normal entrypoint
consumption happens before brain starts, operational investigation uses the boot
logs rather than assuming brain.info will still expose that prior record. Keep
write/consume/read ordering together if moving this responsibility.

A boot stamp proves which artifact claims to be running; a completed health probe
proves its particular listener answered. Neither proves all provider credentials,
remote mounts, or wake/sleep behavior. Consult the deployment runbook’s live
checks separately from local assembly tests.

## Credentials and capability authority

The brain’s outbound hub credential, the facade’s outbound hub credential, and
an agent’s local facade bearer are distinct. Node entrypoint supports
`WKS_MCP_HUB_TOKEN`, falling back to HUB_TOKEN with a warning; a provider credential
appropriate for brain registration may be too narrow for the facade’s forwarded
agent calls. Per-session facade authentication does not broaden that outbound
connection. Preserve provider registration boundaries and host-owned admin gates.

The hub image passes an empty jobs-file flag to disable scheduling/admin methods.
The old explanation that any scoped operator token passes a bare IsTrusted jobs
gate is obsolete: current jobsTrusted also requires authenticated-host provenance.
Do not remove the image’s off switch based on an old token assumption; choose
job enablement according to the intended deployment and actual owner credential.
See [jobs](hub-jobs.md).

Dial-error redaction protects tokened URLs. A separate current limitation remains:
brain’s token flag is constructed from HUB_TOKEN as its default, so ordinary Go
flag help can expose that value. Do not capture help from a credential-bearing
shell into shared logs as a diagnostic. Source review of this behavior is not a
claim that every other launcher/help/log path is redacted.

## Validation and limits

From the repository root:

```bash
bash deploy/fly/test-fetch-release.sh
```

This uses local fixture archives and checks release/commit/file requirements.
Shell syntax checks validate parsing, not runtime commands or cloud boot.
`deploy/fly/preflight.sh` has image-build, boot-rehearsal and artifact stages and
requires Docker plus its other declared prerequisites. Its controlled rehearsal
replaces tailnet daemons; it is not a live Fly/Tailscale deployment test.

Use `deploy/fly/node/RUNBOOK.md`, the hub guide, or
`deploy/fly/combined/README.md` for the selected topology. No deployment,
credential rotation, cloud-machine start/stop, or production restart is implied
by reviewing these docs. Record the actual image/stamp and environment when
performing those operational checks.
