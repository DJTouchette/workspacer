# Always-on Fly hub

The default image runs `workspacer-rust serve --hub-only`. It owns the node
registry, pairing identity, plugins, `/m` and `/app`; execution comes from the
separately authenticated worker provider. Scheduled jobs are disabled here and
uploads go to the worker. The local service cannot masquerade as worker liveness.

Start with [RUNBOOK.md](RUNBOOK.md) for the ordered provisioning and reachability
steps. Read the [current Rust image contract](../rust/README.md) for upgrades,
separate worker credentials and the persisted identity rules.

Build independently from the repository root:

```sh
docker build -f deploy/fly/hub/Dockerfile \
  --build-arg WKS_SOURCE_SHA=REVIEWED_COMMIT -t workspacer-hub:dev .
```

`WKS_INSTALL=artifact` consumes `workspacer-rust`, `claudemon`, web assets and
the build stamp from one Rust-only release archive. Set `WKS_RELEASE_TAG` and
preferably `WKS_RELEASE_SHA`. `WKS_WITH_WEBAPP=0` omits `/app`, retaining the
compiled mobile client. The generated Dockerfile's default role is `hub`.

The mounted HOME, pairing and VAPID keys, node registry, UID/GID 10001,
Tailscale TLS, optional second plugin origin and watchdog remain intact.
No worker account or live volume is read while building. Existing custom images
must use the audited upgrade path; legacy `WKS_BASE` layering is refused rather
than silently replacing its users, tools or supervisor policy.

Run `deploy/fly/preflight.sh` for static contracts and isolated image boot checks.
These never deploy to Fly; the deployment gate remains pending until actual
image and platform checks pass.
