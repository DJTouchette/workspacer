# Fly worker node

The default image runs `workspacer-rust serve --upstream`: one owned Rust engine,
loopback MCP facade and outbound provider relay. `claudemon` remains a Rust hook
and diagnostic CLI. Node/npm and Claude Code remain third-party provider/plugin
tooling; no Go backend or private Node desktop-host process is installed.

Start with [RUNBOOK.md](RUNBOOK.md) for provisioning, persistence and Tailscale,
and read the [current image and credential migration contract](../rust/README.md)
before upgrading an existing volume. Full workers require distinct upstream
provider and scoped facade-caller credentials, plus a local pairing identity.
Existing credentials are never silently rotated or reused for another role.

Build from the repository root:

```sh
docker build -f deploy/fly/node/Dockerfile \
  --build-arg WKS_SOURCE_SHA=REVIEWED_COMMIT -t workspacer-node-base:dev .
```

`WKS_INSTALL=artifact` instead consumes the Rust-only server release archive;
set `WKS_RELEASE_TAG` and preferably `WKS_RELEASE_SHA`. The Dockerfile is generated
from `../rust/Dockerfile` with role `node`. Image build/boot checks are run by
`deploy/fly/preflight.sh` and the Rust container contract CI workflow.

UID/GID 10001, `/data/home`, the state directories, Tailscale identity, wake
doorbell, volume-loss guards, signal drain and public image-verifier path are
preserved. The base still reserves `/usr/local/go/bin` for optional downstream
project toolchains. [example.Dockerfile](example.Dockerfile) illustrates a project
layer. Use the audited [upgrade builder](../rust/build-upgrade.sh) for existing
custom images rather than silently dropping their installed tooling.

No Docker build or real Fly deployment is certified merely by a source check.
The CI boot rehearsal uses fresh volumes and fake Tailscale; real networking,
OAuth and cloud operations require the explicit operator checks in the runbook.
