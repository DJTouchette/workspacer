# Rust Fly image preview

This is an explicit build-time preview. Existing Fly Dockerfiles and app-config
default builds still select the legacy backend. Explicit Rust source/artifact
builds and the isolated upgrade path are available below. Nothing here deploys or changes a Fly machine.

The new Dockerfile has `WKS_ROLE=node`, `hub` and `combined` runtime stages. They run
`workspacer-rust`; the node also carries the Rust `claudemon` executable for hook
forwarding and diagnostics. The engine, MCP server and outbound provider relay
run inside the one worker backend process. The hub uses `serve --hub-only`, so
its local services cannot masquerade as external node liveness. It disables
jobs and forwards uploads to a registered worker. There are no Go backend
binaries or private `desktop-host.cjs` companion. The source Node 22/npm/tooling
payload is retained for third-party provider CLIs and installed plugin runtimes;
it does not implement the backend. The worker preserves the existing
`@anthropic-ai/claude-code` installation and version check, with the same
`CLAUDE_CODE_VERSION` build argument. Existing downstream provider/toolchain
installations are retained by the upgrade layer. Within these explicitly selected
images, `workspacer` is a verified symlink to `workspacer-rust`; no Go executable
backs the compatibility name.

## Build and validation

Use a reviewed, committed source revision and pass its exact SHA as the stamp.
The build requires a Rust image compatible with the checked-in lockfiles;
`RUST_IMAGE` can select a pinned builder. Architecture comes from BuildKit's
`TARGETARCH`, not a hard-coded stamp.

```sh
docker build -f deploy/fly/rust/Dockerfile \
  --build-arg WKS_ROLE=node --build-arg WKS_SOURCE_SHA=REVIEWED_COMMIT \
  -t workspacer-rust-node:preview .
docker build -f deploy/fly/rust/Dockerfile \
  --build-arg WKS_ROLE=hub --build-arg WKS_SOURCE_SHA=REVIEWED_COMMIT \
  -t workspacer-rust-hub:preview .
docker build -f deploy/fly/rust/Dockerfile \
  --build-arg WKS_ROLE=combined --build-arg WKS_SOURCE_SHA=REVIEWED_COMMIT \
  -t workspacer-rust-combined:preview .
```

`WKS_WITH_WEBAPP=0` skips the web renderer build; compiled `/m` assets remain.
The final image checks reject Go backend executables, the private Node companion
and baked account/volume state.
Builds use the repository root as context. The Dockerfile-specific ignore file
excludes target directories, node_modules, logs and existing dist output.

Run the local contracts without cloud credentials:

```sh
bash deploy/fly/rust/test-launch.sh
python3 deploy/fly/rust/test-supervisor.py
bash deploy/fly/rust/test-artifact.sh
bash deploy/fly/rust/test-provision.sh /path/to/workspacer-rust
WORKSPACER_RUST_BIN=/path/to/workspacer-rust bash deploy/fly/rust/test-launch.sh
bash deploy/fly/node/test-bootstrap.sh
bash deploy/fly/hub/test-bootstrap.sh
```

The optional binary check parses the exact container argument arrays with
`--help`; it starts no backend. Docker is not available in the implementation
workspace, so no image build or container boot has been certified here.
`deploy/fly/rust/preflight.sh` supplies static, build, artifact and boot stages;
`WKS_RUST_BACKEND=1 deploy/fly/preflight.sh` routes to them. The boot rehearsal
uses actual images and fresh local volumes, simulates only Tailscale, provisions
separate temporary provider/facade identities, verifies Rust relay registration
and stops both roles gracefully. It also boots/stops the generic combined image. It does not run a model or contact Fly.

## Existing volumes and identity

The entrypoint opt-in is `WKS_RUST_BACKEND=1`, baked into these images. The old
entrypoints keep their network, boot logs, UID/GID 10001, volume checks,
Tailscale identity, TLS setup, doorbell and shutdown traps. Their default branch
is unchanged. Both roles still use:

- `/data/home` for HOME and provider accounts.
- `/data/home/.config/workspacer` for config, remote-token, scoped tokens,
  plugins, peers and nodes.
- `/data/home/.config/workspacer-hub` for historical hub state and VAPID identity.
- `/data/home/.local/share/claudemon/state.db` for the worker database.
- `/data/tailscale` for the persisted tailnet identity.

No file symlink replaces persistent directories. A missing established pairing
identity still refuses startup. The old Go worker could have configuration and
session tokens but no **local** pairing identity, because it only attached to the
remote hub. For that reviewed first migration, run this explicitly as the volume
owner; it preserves an existing token and prints only its prefix:

```sh
workspacer-rust --config-dir /data/home/.config/workspacer \
  token init-host --allow-new-token
```

Do not put `WORKSPACER_ALLOW_NEW_TOKEN=1` or `--allow-new-token` in the entrypoint.
Restore an accidentally lost established token instead. The worker's local
identity is distinct from both upstream credentials below.

The Rust node passes only this boot's consumed previous-exit record through
`WKS_LAST_EXIT_FILE`. An old `last-exit.consumed.json` is not reused on a later
unclean boot. `brain.info` returns only the public reason/code/time fields.

## Separate worker credentials

A full worker uses two credentials minted on the always-on hub:

1. `HUB_TOKEN` (or `WKS_PROVIDER_TOKEN_FILE`) authenticates provider registration.
   Use a dedicated provider-scoped token. Do not rotate an existing credential
   implicitly during image migration.
2. `WKS_MCP_HUB_TOKEN` (or `WKS_MCP_HUB_TOKEN_FILE`) is a **different**, dedicated
   operator service token with `facadeAuthority: true`. The worker verifies its
   negotiated scope, grant and fingerprint. A host token, provider token,
   ordinary operator token or old hub without identity negotiation is refused.

Run the explicit migration helper on the hub as the scoped token-store owner
(`wks` in this image), with an output directory owned by that user:

```sh
/usr/local/lib/wks-rust/provision-worker-caller.sh \
  /data/home/.config/workspacer/tokens.json NODE_ID /private/worker-mcp-token
```

This only updates the local scoped token store and creates an owner-only output
file. It never contacts Fly, reuses an owner/provider token, prints the credential,
replaces an existing output or silently modifies a duplicate service label.
Transfer the output using the deployment secret store, then remove the transfer
copy. Secret values should enter the secret CLI through stdin or a protected
file, not shell history or backend process arguments. Restart the worker after
provisioning. The relay waits for its authenticated MCP caller/catalog before it
advertises full readiness.

Full workers require `WKS_MCP_FACADE_ENABLED=1`; the current container policy
defaults to `WKS_MCP_UNTOKENED=deny`; an explicit `view` or `operator` override
is preserved. A separate `WKS_MCP_TOKEN` static facade credential disables guest
access and does not become the hub owner or a session identity.
Non-loopback custom facade addresses are rejected; configure
`WKS_MCP_FACADE_PORT` instead. `WKS_BRAIN_SCOPE=catalog` remains available and
can disable MCP explicitly. `HUB_BUS_URL`, `WKS_NODE_ID`, database/hook/API ports
and existing Tailscale settings retain their meanings. The worker disables
central jobs, plugin autoload, push, node management and ordinary peer owners.

## Cutover gates and legacy paths

Before changing a deployed machine, build and boot the selected image, provision
the separate facade identity, verify provider CLI/account readiness, test an
actual two-host spawn/MCP round trip, and verify graceful stop/wake against the
existing volume. Image building and fake cloud tests are not cloud deployment
validation.

The legacy `deploy/fly/{hub,node}/Dockerfile` default paths are unchanged until
cutover. The Rust Dockerfile also supports `WKS_INSTALL=artifact`, using the
existing stamp-checked fetcher with required `workspacer-rust`,
`claudemon`, `web/index.html` and `build-stamp`. An older Go-only nightly refuses
rather than falling back to Go. `WKS_RELEASE_TAG`, `WKS_RELEASE_SHA`, repository,
asset and base URL overrides retain the fetcher's existing meanings. The fetcher
logs the archive digest but does not compare it with a published checksum. Asset
architecture follows BuildKit unless explicitly overridden.

## Existing isolated combined deployment

The deployed protected supervisor is different from the generic combined
entrypoint. It holds hub state and Fly/network credentials under UID10002 while
workers and provider accounts use UID10001. Do not replace it with a generic
single-user entrypoint.

`upgrade-supervisor.py` transforms only the audited hub and worker spawn blocks.
Its source fixture is the read-only program at `/opt/combined/supervisor.py`, not
live state or credentials. Unknown spawn policy, owner IDs, missing manifest or
network/power guards refuse transformation. Root isolation, Tailscale identity,
volume attestation, private logs and process-group draining remain unchanged.
The transformed worker requires an already provisioned local identity and an
approved operator facade token; it never mints or widens those at startup.

Build an image without deploying it:

```sh
bash deploy/fly/rust/build-upgrade.sh CURRENT_IMAGE NEW_IMAGE --isolated
```

The current source tree must be clean and committed for an honest build stamp.
This layer preserves the image's external provider/toolchain payload, replaces
the Rust binaries and web assets, and removes Go backend names and the private
Node companion. Optional `--power APP MACHINE observe|stop|off` preserves the
root-installed fixed-target power configuration interface. No Fly command runs.

The existing combined `build-upgrade.sh`, `build-parity-upgrade.sh`,
`build-client-upgrade.sh` and `build-daemon-upgrade.sh` route to this whole-backend
Rust build when `WKS_RUST_BACKEND=1`. Updating the `claudemon` leaf alone would no
longer update the embedded engine. Existing deploy/preflight/verification scripts
select `workspacer-rust` for their read-only idle query when present. They retain
their original explicit deployment/provisioning authorization behavior; none was
run against a live machine during this migration.

For future Rust split-role images, omit `--isolated`; the upgrade builder requires
a `rust-hub`, `rust-node` or `rust-combined` label and retains that role's entrypoint. The generic
combined Dockerfile remains a separate legacy template; the new `WKS_ROLE=combined`
image provides its owned single-user Rust equivalent. It must not replace the
existing UID-isolated deployment. No legacy source is
safe to delete solely because these opt-in paths exist: the parent release and
default packaging cutover must update its remaining consumers too.
