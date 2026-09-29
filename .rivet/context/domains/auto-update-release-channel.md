---
title: In-app auto-update (electron-updater, stable vs nightly channels)
tags: [auto-update, electron-updater, release, nightly, distribution, electron-builder]
related_paths:
  - "apps/desktop/src/main/services/updateService.ts"
  - "apps/desktop/src/main/services/updateService.test.ts"
  - "apps/desktop/src/renderer/src/components/settings/UpdatesSection.tsx"
  - "apps/desktop/src/renderer/src/hooks/useAppVersion.ts"
  - "apps/desktop/src/main/ipc.ts"
  - "apps/desktop/src/main/shared/ipcChannels.ts"
  - "apps/desktop/src/main/preload.ts"
  - "apps/desktop/src/renderer/src/App.tsx"
  - "apps/desktop/electron-builder.yml"
  - ".github/workflows/release.yml"
  - ".github/workflows/rust-native-preview.yml"
  - "apps/desktop/scripts/build-rust-backend.mjs"
  - "apps/native/scripts/windows-payload.mjs"
  - "apps/native/scripts/package-windows.mjs"
  - "apps/native/scripts/test-windows-installer.ps1"
  - "apps/native/src/bin/native-harness.rs"
  - "scripts/hub-migration.py"
  - "scripts/check-nightly-preview.py"
  - "scripts/test_nightly_preview.py"
  - "scripts/smoke-server-bundle.py"
  - "scripts/test_smoke_server_bundle.py"
  - "services/hub-rs/migration.json"
owner: Damien Touchette
last_reviewed: 2026-09-29
---

# Desktop updates and release channels

## Runtime ownership

`apps/desktop/src/main/services/updateService.ts` owns the Electron updater.
The main process starts it with its window and stops its timer during shutdown.
Status is pulled through `updatesGetStatus` and pushed through `onUpdateStatus`;
check/install use native IPC. Plain `webBackend.ts` reports unsupported.
`bridgedBackend.ts` keeps all four operations in `HOST_ONLY`, so desktop bus
mode still updates the local Electron distribution. This does not update an
independently deployed headless server.

The status states are `unsupported`, `manual`, `disabled`, `idle`, `checking`,
`downloading`, `downloaded`, and `error`. Gates run in this order:

1. Unpackaged builds remain unsupported.
2. Packaged macOS reports manual; an explicit check opens the releases page.
3. On supported platforms, `updates.enabled=false` disables automatic startup.
4. Otherwise wire listeners, check immediately, and repeat every four hours.

An explicit `checkNow()` bypasses the config disable switch on supported
packaged builds. `wire()` is guarded to run once, while `stop()` only clears the
timer. Do not assume saving config dynamically rewires the service or changes
its existing feed/timer; inspect the caller lifecycle before promising that.

Downloads start automatically; `autoInstallOnAppQuit=false`. On download,
the dialog offers restart, release notes, or later. Reading notes returns to
the decision dialog. `promptOpen` prevents overlapping dialogs while one is
open; it is not a permanent version-based deduplication record. `installNow()`
only acts in downloaded state and then invokes `quitAndInstall()` directly.

Updater error events log and publish error state without an error dialog.
The separate check rejection handler prevents an unhandled rejection; it
relies on the updater's event for error-state publication. Status pushes can
be lost during window teardown; the renderer pulls current state on mount.

## Feed selection and input validation

Stable builds use the GitHub provider configured in
`apps/desktop/electron-builder.yml`. Versions containing `-nightly` explicitly
switch to the generic rolling-nightly asset URL, force channel `latest`, and
allow downgrades. Single-range download configuration is explicit through
`useMultipleRangeRequest=false`; retain it when modifying the generic feed.
These are the checked-in feed choices, not a live CDN availability test.

`sanitizeUpdateChannel` accepts a lowercase alphanumeric first character,
then lowercase alphanumeric, dot, dash or underscore; invalid values fall
back to `latest`. It validates spelling, not feed existence. Nightly ignores
the configured channel because its assets use `latest*.yml`.
`releaseNotesUrl` accepts the service's restricted version pattern, sends
nightly versions to the rolling release, and falls back to the releases index
for invalid values. Do not concatenate unchecked feed input into URLs.

`apps/desktop/src/renderer/src/components/settings/UpdatesSection.tsx` exposes
the enabled toggle, running version, nightly badge and bundled release notes;
there is no channel selector. Manual mode disables the toggle. Bundled notes
come from the running build, so the offered update's dialog opens its release
page rather than presenting the old bundled changelog as new release notes.

## Packaging and publication

`apps/desktop/electron-builder.yml` builds Windows NSIS/portable, Linux
AppImage and macOS DMG packages. The workflow uses Windows x64, Linux x64 and
macOS arm64 legs. macOS has no ZIP target, signing discovery is disabled, and
the service explicitly gates automatic updates off. Changing packaging alone
does not remove that runtime gate.

`.github/workflows/release.yml` runs for version tags, PRs, manual dispatches
and a daily 08:00 UTC schedule. Its gate chooses nightly mode for schedule or
nightly input, skips an unchanged nightly SHA, and emits one numeric timestamp
for all build legs. When `services/hub-rs/migration.json` exists, nightly mode
also runs `python3 scripts/hub-migration.py ready`. Failure normally sets
`build=false` and defers publication. That command checks source hashes,
replacement/test evidence and cutover gates; it does not execute tests.
Ordinary non-nightly manual/PR/tag builds can still produce validation artifacts.

An explicit manual dispatch with **both** `nightly=true` and
`migration_preview=true` may publish a testing nightly before the audit is
complete. `scripts/check-nightly-preview.py` requires the default branch and a
completed successful push run of `ci.yml` for the exact candidate SHA. A newer
failed or pending run cannot borrow an older success. The workflow checks this
before building and again before changing the live release. This mode labels
the release notes as a Rust migration testing nightly and does not change any
migration evidence or cutover status. Scheduled runs and default manual nightly
runs retain the completion gate. The testing nightly uses the existing rolling
nightly feed; it is not a separate release channel.

The normal release path does not enforce independent CI conclusions. The explicit
migration-preview path enforces `ci.yml`, and all publication still waits for the
release workflow's own build legs and installer smoke tests. Do not claim this
checks every other workflow automatically.

Builds use `--publish never`. `npm run package` builds the Electron frontend,
Rust backend and standalone claudemon before electron-builder. Electron retains
its own claudemon process and supplies UI capabilities to the Rust control plane;
its packaged resources no longer contain Go services or `desktop-host.cjs`.

Uploads include installers, update YAML, blockmaps, standalone server bundles
and standalone claudemon bundles. The server bundle contains `workspacer-rust`
(and the `workspacer` executable alias), standalone `claudemon`, `web/`, optional
plugin `examples/`, a README and build stamp. `workspacer serve` owns the engine,
bus, desktop services and MCP facade in one Rust process. No private Node
companion or Node runtime is required for this core; external provider CLIs and
optional plugin sidecars can have their own runtime requirements. Node remains
build tooling, and Electron itself still includes its JavaScript runtime.

Version-tag attachment uses a **draft** GitHub release, with notes cut from
`CHANGELOG.md`; a missing tagged section fails the notes step. Building a tag
is not equivalent to publishing its draft to clients.

Nightly packaging uses space-free Windows artifact names and bypasses the
conditional Azure signing path. Stable Windows signing is conditional on the
version-tag/profile gates; do not assert every built Windows artifact is signed.

## Nightly rollout failure boundary

The publish job waits for all matrix build legs and the separate native Windows
packaging/smoke job, serializes on a shared nightly lock,
and checks expected assets before changing the live release. It removes stray
drafts, uploads a new draft, deletes the old live release, deletes/polls the tag
up to six times, then publishes the new draft. A build or pre-deletion upload
failure leaves the old release in place.

This sequence is **not atomic**. After old-release deletion, failed tag cleanup
or failed draft publication can leave no live nightly. The tag check retries
propagation delays, but a failed lookup is not independently distinguished from
an absent tag by that shell condition. Avoid promising uninterrupted feed
availability or automatic rollback. The workflow's own comments use stronger
language than its executable sequence supports.

## Validation scope

The earlier updater audit ran 39 mocked lifecycle/input tests. The 2026-09-29
packaging audit checked workflow and script source without rerunning that suite.
Manual [release build 36566605909](https://github.com/DJTouchette/workspacer/actions/runs/36566605909)
on `04322791` completed all three build legs and artifact uploads successfully,
including the Windows native installer smoke; its nightly publication and tag
attachment steps were skipped. This was a release-workflow build preview, not a
nightly publication or proof of full service parity. The separately named
[Rust backend/native preview 36534488797](https://github.com/DJTouchette/workspacer/actions/runs/36534488797)
on the same revision failed its service-test legs. Build success therefore
must not be reported as green independent CI.

No release publication, platform signing or external updater CDN was verified
by this audit. See [deployment](../modules/fly-node-deploy.md) for artifact stamp validation and
[serve CLI](../modules/workspacer-serve-cli.md) for headless bundle ownership.

## Stable Windows metadata audit

The installed `electron-updater` GitHub provider replaces spaces with dashes in
resolved filenames; it does not reconcile arbitrary dotted versus dashed asset
names. The 2026-09-26 release learning records a stable metadata/asset mismatch.
For Electron Windows artifacts, the workflow gives explicit space-free names
only to nightlies. Verify
each stable update YAML path/URL against the actual uploaded draft asset names
before publication; do not assume the provider repairs them. This audit checked
the local resolver and workflow, not the historical live release URLs.

## Extracted standalone archive smoke

After creating each platform's standalone server archive, the release matrix runs
`scripts/smoke-server-bundle.py` against the extracted archive. It validates the
source/platform stamp and CLI alias, starts with isolated home/config/data/database,
empty PATH and hook initialization disabled, then checks authenticated hub and
brain service readiness, the engine/hook listeners, MCP identity/catalog health and
the bundled web entry. MCP coverage also makes actual `tools/list` requests for
legacy `2025-11-25` and modern `2026-07-28` protocol shapes, including complete-result
and private zero-TTL cache hints for modern discovery. Parent-pipe EOF must yield a
successful joined process exit and closure of all four listeners.

This guard exercises packaged binaries, including actual MCP catalog responses;
it does not establish GUI behavior or every provider/tool operation. No production
state, provider credentials or active daemon is used by the smoke.

## Native Windows installer

`native-windows-build` depends only on the release gate and compiles `wks-native`
and `native-harness` in release mode alongside the Electron/backend matrix, on
an independent Windows runner. It uploads the two binaries and the x64 CRT DLLs
from that compiler runner, with file SHA256s and source SHA, gate-derived version,
platform, workflow run, rustc, runner image and CRT-source provenance.

`native-windows-package` waits for the matrix and native compile jobs. It downloads
those inputs and the existing Rust backend executable uploaded by the Windows
Electron leg. It verifies both receipts against source SHA, version, platform,
run ID and every file hash before restoring the canonical package paths. It does
not compile Rust or rediscover CRT DLLs on the packaging runner. The captured CRT
also travels beside the harness. Locked `npm ci --ignore-scripts` supplies the
pinned NSIS toolchain without rebuilding Electron; the existing native payload
regressions and install/backend/uninstall smoke remain mandatory.

Each compilation job uses one `rust-cache` action, `cache-bin: false`, and a
separate release cache key. Cache cleanup cannot compete over Cargo-bin on the
same runner. Dependency caches do not replace explicit binary artifact transfers.
Final installer/archive uploads use compression level zero to avoid recompressing
existing compressed payloads; intermediate raw binaries retain normal compression.
This changes scheduling and compression work; measured wall-time improvement still
requires the nonpublishing validation run.

Internal transfer artifacts deliberately lack the `workspacer-` prefix used by
the nightly publisher. Only the final `workspacer-native-windows` artifact joins
the release downloads, and stable tags attach the unsigned native installer to
the existing draft. The nightly notes include its direct download link. The
current separate unsigned artifact is
`Workspacer-Native-Rust-Preview-Setup-<version>-x64.exe`; the nightly asset gate
now requires that exact naming family. Native updates remain manual and emit no
Electron update metadata. The Electron Windows download excludes native assets.

`windows-payload.mjs` stages `wks-native.exe`, standalone `workspacer-rust.exe`,
Visual C++ runtime DLLs, license/icon/README, optional examples and build stamp.
It includes neither the four Go services nor `desktop-host.cjs`, `node.exe` or
a Node license. The Windows packaging script requires Node 22.13+ within the
Node 22 series as build tooling; that is not a runtime payload dependency.

The per-user NSIS identity remains `Workspacer.Native.RustPreview`, separate
from standard native and Electron installs. Its Start menu and finish-page
launches pass `--local`, owning the embedded Rust backend. Bare executable
launches still attach to an existing hub. State defaults to
`%LOCALAPPDATA%\Workspacer Native Rust Preview`; `--rust-local-dir` selects another
isolated directory. This isolates backend stores, not every integration with
the owner's provider accounts/home. Uninstall enumerates owned package files
and preserves user data and unknown files.

The Windows smoke verifies installation/upgrade hashes, shortcuts, notification
and uninstall registrations, absence of legacy payloads, and the installed
standalone CLI's `--help`. With Node removed from PATH, the build-output
`native-harness rust-probe` runs the same `NativeHost::Rust` implementation in
isolated state, checks four actual owned listener receipts, catalog/controller
connectivity, joined shutdown and released ports. The harness is not the
installed GUI executable: this is backend/installer evidence, not visual GPUI
launch verification. Linux-local NSIS fixture compilation alone proves neither.

`.github/workflows/rust-native-preview.yml` separately runs Rust service tests
on Linux/macOS and Windows contract/native tests plus preview packaging/smoke.
It uploads a validation installer and has no release/tag publication steps.
Neither that workflow nor the separate general CI workflow is a `needs`
dependency of `release.yml`.
