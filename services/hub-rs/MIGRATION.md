# Shared Rust backend and migration record

Workspacer uses one shared Rust service library in the standalone executable and
native GPUI host. The Go hub, brain, MCP facade and launcher, and the private
Node desktop-services companion, are retired. Public Electron JavaScript
services, external provider CLIs and enabled plugin sidecars retain their own
roles; retiring the private companion does not remove those owners.

## Completed ownership and supported contracts

The source review inventory records 505 ported entries and 29 explicit
architectural retirements. [migration.json](migration.json) and
[the source reviews](reviews) retain original source hashes, behavior mappings,
intentional differences and scoped test evidence. Completion covers the reviewed
supported contracts, not every undocumented legacy input or a promise that all
old implementations were byte-identical.

Standalone `workspacer serve` owns the session engine, bus, services and MCP
facade in one process. Native local mode embeds that backend and uses library
calls/channels; network transports adapt the same services for external clients.
Electron keeps its public desktop services and owns or adopts the Rust control
plane. TUI startup can bootstrap a missing local Rust backend while preserving
pre-existing process ownership. No Go runtime fallback or private Node backend
is part of these compositions.

The shared owners implement configuration/profiles, saved sessions/layouts,
jobs/history, filesystem and Git operations, library/briefs, routing/usage,
workflow/task admission, durable replacement and remote-dispatch ownership.
MCP serves the reviewed builtin catalog plus enabled plugin tools through its
HTTP/SSE adapters. Federation preserves peer provenance and both local and
destination admission. Explicitly retired surfaces remain documented rather
than being reintroduced as placeholder handlers.

Persistence keeps supported stored representations and identity-loss behavior,
with the family-specific limits in [PERSISTED_STATE_REVIEW.md](PERSISTED_STATE_REVIEW.md).
The standard standalone selection retains the historical `workspacer-hub` state
directory for layout, jobs/history, pacing and push identity. Explicit
`--data-dir` selects shared hub state; custom configuration remains isolated.
A failed or uncertain operation does not become authorization to replay it.

`migrationComplete` is a build milestone. It is distinct from current runtime
readiness, provider availability and `launchReady`; a completed build can still
report an unavailable engine, disconnected provider or refused operation.
The library does not install process-wide signal handlers or exit its host;
the standalone launcher and embedding GUI own their respective lifetimes.

## Release and platform evidence

[CUTOVER_STATUS.md](CUTOVER_STATUS.md) is the index of reviewed gate receipts and
artifact/revision boundaries. Use those exact CI/package receipts rather than
inferring a published version from source completion or this document. Source
hash inventories and a successful completion checker do not execute tests.

The native Windows artifact retains its isolated Rust Preview identity and data
folder. Its installer/backend/upgrade/shutdown/uninstall scope is separate from
interactive GUI checks. Native macOS/Linux installers are not implied by the
existing Windows preview workflow. A fresh-volume container boot does not claim
a cloud rollout or upgrade of every pre-existing volume.

Native local integration uses the default `rust-hub` feature. The GUI defaults
to attaching to an existing service; `--local` selects local ownership, and
`--rust-local-dir <isolated-directory>` selects explicitly isolated ownership. `make test-native-rust` exercises the
controller/protocol and in-process adapter contracts, not a visual UI test on
every operating system.

## Repeatable commands

```sh
make test-hub-rust       # Rust runtime, transports, lifecycle and contract tests
make test-hub-parity     # explicit pinned-checkout Go oracle; setup below
make hub-migration      # inventory status and source-drift checks
python3 scripts/hub-migration.py backlog  # source-review inventory summary
python3 scripts/hub-migration.py backlog --json --prefix services/hub/cmd/brain/
make test-native-rust   # native protocol/controller + in-memory hub adapter
python3 scripts/hub-migration.py ready  # validate recorded completion evidence
```

Historical Go commands require a separate clean checkout at the exact captured
reference revision. Set `WKS_HUB_REFERENCE_ROOT` and run
`python3 scripts/hub-reference.py verify` before `make test-hub-reference`,
`make test-hub-parity` or `make test-routing-harness`. There is no fallback to the
current working tree. See [the pinned-reference instructions](../../scripts/reference/README.md).

`make hub-vocabulary` now checks historical output against the retained asset; it
does not overwrite it. `python3 scripts/hub-reference.py vocabulary-export` emits
an explicitly requested historical export to stdout for review. Routine Rust
assets are generated from portable contracts/current public TypeScript owners;
`make check-hub-rust-assets` needs no historical Go execution. Vocabulary coverage
is not behavioral parity; preserve independent fixture loaders and actual
installed-capability checks.

For local core experiments, `wks-hub --listen 127.0.0.1:0 --token-file <file>`
uses an existing credential and prints its assigned address. Add `--database`,
`--config-dir`, `--data-dir`, and `--home-dir` to embed claudemon and the migrated
local services. No legacy process is launched. `Hub::start(Options::default())`
starts without sockets. `backend::Backend` owns both Rust engines.

## Historical integration checkpoint (2026-09-28)

The following records an earlier, incomplete checkpoint. Its pending-review and
platform limitations describe that revision, not the current ownership above.

The combined crate passes `cargo check --all-targets` after wiring provider caller
proofs, the upstream MCP bridge, and the embedded conversation/statusline producer.
This is a compile checkpoint, not a complete suite or cutover claim. The provider
relay preserves the central hub / outbound worker topology; delegated calls carry
verified scope and a token fingerprint, never a bearer or synthesized host token.

Further implemented slices include threshold notifications, file-watch leases,
resumable-session discovery, review evidence, brief boards, analytics, terminal
leases, quiescence and machine-power policy, persistent push subscriptions and
HTTP/PWA assets. Native controller/protocol/embedding tests and a full GUI
feature compile have passed at earlier checkpoints; Windows process ownership
had compile-only coverage locally at that checkpoint. Subsequent Windows CI
and installer smoke passed at later checkpoints; see the receipt index in
[CUTOVER_STATUS.md](CUTOVER_STATUS.md) for their exact revisions.

The inventory remains deliberately conservative: unreviewed source rows and
cutover gates are pending even when a Rust module implements some behavior.
Packaging now selects the Rust backend. Do not delete the reference
implementation or announce migration completion until the compatibility and
packaging gates have been reviewed and run against a fixed source revision.

`make hub-capability-inventory` starts an isolated complete backend and compares
its actual installed methods with the reference brain/catalog and hub registries.
A mapped MCP name alone is not an implementation. The report explicitly does not
claim behavioral parity. `make build-rust-backend` builds the service used by the
normal Electron/TUI startup paths. Electron keeps its
existing provider services inside Electron and adopts the Rust hub's embedded MCP
listener; it does not start a separate Go facade or private Node companion.

The standard standalone configuration retains the old `workspacer-hub` directory
for layout, scheduled jobs/history, pacing preferences and push identity. Explicit
`--data-dir` overrides shared hub state; a custom `--config-dir` remains isolated.
The native preview continues to use its separate data directory.


## Retained assets and provenance

Shipped trusted plugin examples now live under `plugins/examples`; installed
bundle paths remain unchanged. Original Go paths in provenance identify historical
bytes, not packaging inputs or required current-checkout files.
`make check-retained-plugin-assets` checks exact hashes
and the documented test-import/README relocation exceptions. Public Electron
JavaScript services and optional Node plugin sidecars remain live product owners.

The TS-only routing-preferences sample lives under
`apps/desktop/tests/fixtures/routing-preferences-view.json`; its original source
path/hash remain recorded as provenance. See [the deletion preparation map](LEGACY_DELETION_PREPARATION.md)
and [persisted-state evidence](PERSISTED_STATE_REVIEW.md). The preparation map
preserves the reviewed removal sequence. Portable capture
validation and optional historical oracle execution have narrower scopes than
the completed runtime/platform receipts.
