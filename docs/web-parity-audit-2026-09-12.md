# `/app` desktop parity — implemented and deployed

The identified server-backed gaps are implemented and deployed on the combined
Fly node. Server-owner browser access now uses the same operational services as
native desktop, with browser equivalents for file selection, downloads and reveal.
Native OS integration remains a separate boundary.

## Deployment

- App/machine: `workspacer-node / 1857645df24448`
- Image: `registry.fly.io/workspacer-node:desktop-parity-20260912`
- Verified digest: `sha256:9605d8d2bfb441f1ceca9b4b1bb2b9dc1dc6c8d27655012fb801e1e1447d3b71`
- Authenticated `/app/`, health, plugins and examples returned HTTP 200.
  The external HTTPS route still refuses unauthenticated access.
- Live verification found **165 registered capabilities**; all selected probes passed.
- Hub, claudemon and MCP facade reported ready. HTTPS sharing reported available,
  active and controllable.
- Idle mode remains **stop**, with a **900-second quiet period** and the existing
  sampling interval. Scheduled jobs and running shell jobs prevent automatic stop.
- The update passed the live work guard. Machine identity, volume, supervisor and
  hub UID 10002 / worker UID 10001 separation were retained.

## Resolved capabilities

- Git history diff/statistics, stage, unstage, commit, push and worktree operations.
- Shared workflow definitions, pinned dispatch templates, result contracts, task
  history, request capture, review evidence, HTML review diffs and brief boards.
- Durable manager replacement: checkpoint/hash verification, fresh successor,
  worker/task transfer, viewer binding, retained message receipts and recovery.
- Profile/account operations, title generation, readiness, pricing and diagnostics.
- Persistent analytics using the native schema, queries and transcript accounting,
  including subagent/model attribution and explicit unrecorded-usage counts.
- Custom fonts/icons, server file selection, downloads and editor reveal. The
  bundled sandboxed editor and timeline are seeded on an empty plugin install.
- File watching across atomic replacement/reconnect, config refresh and library
  changes, plus workflow transition events.
- Peer configuration with redacted credentials and hot reload of changed links;
  peer-targeted spawn and immediate follow-up routing; server sharing controls;
  node/job registries and launch integrations.
- Worker-owned uploads: the same image/PDF and 24 MiB limits apply, while files are
  created privately under the identity that must read them. No hub-disk fallback.

## Authority and browser boundaries

Native IPC is an owner interface. Its new server counterparts therefore require
actual server-owner authentication; a scoped operator, provider, plugin or peer
link does not acquire administrative authority by using the browser. The approved
MCP service receives explicit session-provenance delegation; profile/account
allowlists remain independent.

Launch preparation callbacks are bound to a live owner-authorized spawn, the
provider connection and the exact selected plugin. The root sharing helper requires
both its private socket and a separate credential under the protected hub config
root. Job administration is owner-only; the new scheduler marker is ignored by older
rollback images, preserving their disabled legacy scheduler.

Browser security still prevents native cookie import, OS window/updater control,
local PATH installation and native file-manager launch. Server file reveal opens
the editor; external file opening downloads the bytes. Those are browser equivalents,
not claims that a remote path opened on the viewing device.

## Verification

- Go hub/brain/bus/capspec/token/federation checks, plus race checks.
- Main-process and renderer suites, with focused reruns resolving their failures.
  Node 25's native Web Storage was disabled for DOM tests so the test browser's
  actual Storage implementation was used; CPU-heavy fixtures passed with bounded
  test concurrency.
- **14 browser interaction tests** against a real scratch hub.
- Real brain + Node + fake-daemon tests for dispatch/worktree/schema/review flow,
  plugin launch patches and complete manager succession with a queued message.
- Real SQLite/process-restart analytics test and font/file confinement checks.
- Production-image browser smoke: Git edit/stage/commit, rendered sandboxed editor,
  analytics, pricing, administration, uploads/preview, runtime and network controls.
- Separate-UID image test: triage upload works, direct receiver access is denied,
  UID 10001 can read its mode-0600 file, and hub UID 10002 cannot.
- Live read-only validation: [results](web-capability-live-2026-09-12.json).

The generated [capability inventory](web-capability-inventory.md) includes inherited
and optional API members. A transport entry alone is not a runtime claim; the checks
above exercise the implementation and authority boundaries.

## Rollback and source state

Previous image:
`registry.fly.io/workspacer-node:web-parity-20260912@sha256:84619a2c8c5c86f0036438ba5d48c53b0925e0c51d15f9d7ca79d44f8381aa9c`.
The previous machine configuration is retained privately at
`~/.workspacer/backups/web-parity-20260912-machine.json`. Use the existing idle guard
for any rollback. Root-side provisioning backups remain in `/data/combined`.

These implementation changes are in the working tree and have not been pushed to
Git. The deployed image was built from that tested tree on base revision `810a392e`.
