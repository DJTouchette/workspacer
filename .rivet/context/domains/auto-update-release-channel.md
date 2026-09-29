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
also runs `python3 scripts/hub-migration.py ready`; failure sets `build=false`
and defers the nightly. That command checks reviewed source hashes, replacement
and test evidence files, and verified cutover gates. It does not execute tests
or query CI results. Ordinary non-nightly manual/PR/tag builds do not take this
migration-readiness gate, so they can produce validation artifacts mid-migration.

The release workflow **does not check the separate CI or Rust-preview workflow
conclusion**. Its publish job depends on its own `gate` and `build` jobs. Do not
describe the desired green-CI release policy as an enforced workflow dependency.

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

The publish job waits for all build legs, serializes on a shared nightly lock,
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

## Native Windows installer

The Windows release leg builds `wks-native` and `native-harness` in release
mode, then runs `apps/native/scripts/package-windows.mjs`. The current separate
unsigned artifact is
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
