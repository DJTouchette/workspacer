# Web parity implementation — completion checkpoint

The requested server-backed parity delta is implemented and **deployed**. Do not
restart this work from the earlier TODO lists. The authoritative completion report
is `docs/web-parity-audit-2026-09-12.md`; live results are in
`docs/web-capability-live-2026-09-12.json` and the generated API inventory is current.

Live app/machine: workspacer-node / 1857645df24448.
Image: registry.fly.io/workspacer-node:wake-fix-20260912.
Digest: sha256:c660ea2d2c49ed87a0ca44497bcf0377421662a6c83a480291b2a2a43086d7b4.
Live validation passed: 165 capabilities; hub/daemon/facade ready; HTTPS sharing
available/active/controllable; idle STOP still 900 seconds. The update's live guard
reported no work blockers. A client-active blocker immediately after boot/verification
is expected and is not an active agent.

Implementation was developed on base 810a392e; the parity work and Wake fix are
packaged together for version control.
Unrelated .agents/, .claude/skills/ and pre-existing Rivet notes must not be swept
into any later commit. No subagents were used or authorized.

## Architecture and important seams

- Shared native TS cores execute in a brain-owned Node stdio companion. Node >=22.13
  is required; desktop builds, standalone release bundles and the Fly image include
  desktop-host.cjs beside brain. Native browser hosting reuses its native controllers.
- contracts/desktop-service-methods.json drives Go/TS registration and owner gates;
  both languages test the manifest and its authority cases. Regenerate with
  apps/desktop/scripts/gen-desktop-services.mjs after method changes.
- Headless manager replacement uses the existing durable transaction and checkpoint
  validator with fixed private Go lifecycle callbacks. No callback is a public RPC.
  Messages/fleet wakes use a held/in-flight outbox; request-tagged sends retain
  replay-fenced receipts. Worker result capture commits only after fresh validation.
- The MCP service's explicit facadeAuthority grant preserves local session provenance;
  it does not make the service the hub owner or widen profile allowlists.
- Launch integrations use shared validation/Codex routing. The hub validates the
  pending owner spawn, provider connection and selected plugin before preparing.
- Headless analytics shares native SQL/schema and transcript accounting, using
  Node SQLite in headless-analytics.sqlite. Saved usage survives transcript cleanup.
- Peer changes hot reload while retaining unchanged links. Owner-only network control
  uses a root helper with a protected credential and socket. The parent creates the
  credential and sets socket ownership; the capability-dropped child must not chown
  or chmod the socket after the parent can take ownership. Stale sockets are removed
  before launching the helper.
- uploads.Store is shared, but isolated uploads run in the worker. files.receiveUpload
  is owner-gated and is called through the hub's self client for an authorized upload.
  Files remain UID 10001 / 0600 in a per-user temp directory. Preview/download reads
  can read that fixed spill directory without granting fs.write another root.
- Idle self-stop protects all future schedules and running shell jobs. Ordinary
  diagnostic quiescence keeps its prior defaults. The empty scheduler was enabled
  with ownerOnlyJobsEnabled; legacy jobsEnabled remains false, so older images
  cannot accidentally enable their earlier, weaker job authorization.
- The brain now subscribes before seeding, and reseeds on every successful SSE
  connection. Missing the initial daemon startup must not permanently lose roots.
  In-memory live roots bypass the old HTTP-only cwd cache.

## Verification and useful commands

- npm --prefix apps/desktop run test:desktop-host
- node apps/desktop/scripts/verify-parity-image.mjs IMAGE
- node apps/desktop/scripts/verify-upload-identity.mjs IMAGE
- npm --prefix apps/desktop run audit:web -- --check
- python3 deploy/fly/combined/parity-preflight.py APP MACHINE
- python3 deploy/fly/combined/verify-web-capabilities.py APP MACHINE --expect-plugin-admin --expect-parity

The two image checks passed; 14 real-browser interaction tests passed; Go race checks
passed; main/renderer failures were resolved through focused reruns. Broad simultaneous
suites caused 5s fixture timeouts; bounded concurrency fixed those. On this Node 25
host, run DOM tests with NODE_OPTIONS=--no-experimental-webstorage to avoid its
unconfigured native Storage masking the test browser's implementation.

Latest logs are /tmp/workspacer-live-parity-verification.log,
/tmp/workspacer-parity-deploy.log, /tmp/workspacer-image-smoke.log,
/tmp/workspacer-upload-identity-smoke.log, /tmp/workspacer-cross-stack-tests.log,
/tmp/workspacer-parity-race-tests.log, /tmp/workspacer-renderer-final-focus.log,
/tmp/workspacer-main-contention-retry.log and /tmp/workspacer-audit-run.log.

Fly registry auth needed refreshing immediately before docker push. The machine API
rejected a digest-only image identifier; deployment by the published tag succeeded
and resolved to the exact expected digest above. The private rollback config is at
~/.workspacer/backups/web-parity-20260912-machine.json. Preserve idle guards, identity,
volume and UID separation on any subsequent deployment.

## Wake follow-up (2026-09-12)

Fixed both clients using incompatible fetch options (`no-cors` with `redirect:error`),
which browsers reject before contacting Fly. Both now use `redirect:follow` with
credentials omitted, and mobile shell cache is v19. Real Chromium regression tests
pass for /m and /app. The guarded live verification stopped the idle existing Fly
machine and successfully started it using the deployed mobile Wake button.
Repeat via `apps/desktop/scripts/verify-machine-wake.mjs` (requires explicit
`--stop-idle`; refuses work blockers). The client-only image builder is
`deploy/fly/combined/build-client-upgrade.sh`; it preserves the parity supervisor.
