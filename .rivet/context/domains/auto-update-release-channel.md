---
title: Updates and release channels (native updater, stable vs nightly)
tags: [auto-update, native, release, nightly, distribution, windows, signing]
related_paths:
  - "VERSION"
  - "scripts/check-version.mjs"
  - "apps/native/src/updates.rs"
  - "apps/native/src/update_helper.ps1"
  - "apps/native/src/ui/updater.rs"
  - "apps/native/src/data_dir.rs"
  - "apps/native/packaging/windows/installer.nsi"
  - "apps/native/scripts/package-windows.mjs"
  - "apps/native/scripts/resolve-nsis.mjs"
  - "apps/native/scripts/sign-windows.ps1"
  - "apps/native/scripts/windows-payload.mjs"
  - "apps/native/scripts/test-windows-installer.ps1"
  - "apps/native/scripts/package-macos.sh"
  - "apps/native/scripts/package-linux.sh"
  - ".github/workflows/release.yml"
  - ".github/workflows/rust-native-preview.yml"
  - "scripts/electron-feed-bridge.py"
  - "scripts/finalize-nightly.py"
  - "scripts/check-nightly-preview.py"
  - "scripts/smoke-server-bundle.py"
  - "scripts/hub-migration.py"
  - "services/hub-rs/migration.json"
  - "apps/desktop/src/main/services/updateService.ts"
owner: Damien Touchette
last_reviewed: 2026-11-14
---

# Updates and release channels

The released product is the native app. Its updater is
`apps/native/src/updates.rs` with the Windows helper
`apps/native/src/update_helper.ps1`. The Electron updater is covered only by
the short note at the end.

## Version source

The release version is the root `VERSION` file (one line). Native packaging
scripts, `release.yml` and build stamps read it. `apps/desktop/package.json`
carries a copy because the `/app` web client reports it;
`scripts/check-version.mjs` fails CI and the release gate when they differ.
Nightly builds rewrite both to `<version>-nightly.<stamp>` in the runner.

## Native updater

Channel follows the installed version: a version containing `-nightly` checks
the rolling `nightly` tag, anything else checks `/releases/latest` (which
excludes prereleases). There is no channel selector.

`check_result` resolves the latest version from this platform's package name
(`Workspacer-<v>-macos-<arch>.dmg`, `workspacer-<v>-linux-<arch>.tar.gz`), on
Windows from the installer name, then from any other platform's package, then
from the release tag. An update is offered only when that version compares
newer. A development build (unparseable version) reports no comparison.

In-app install is **Windows-only** (`installable` is
`cfg!(target_os = "windows") && asset.is_some()`). On macOS and Linux the
Download button opens `download_url`, this platform's package on the release,
which is offered only when its URL starts with this repository's
`releases/download/` prefix; with no package it opens the release page.
Installing is manual there.

Windows download (`download`): the asset name must match
`Workspacer-Setup-<v>-x64.exe` or the pre-0.170
`Workspacer-Native-Rust-Preview-Setup-<v>-x64.exe`, with no path separator;
the URL must start with the repository's release-download prefix; size must be
1 byte to 256 MiB and match exactly; when the API supplies GitHub's
`sha256:` asset digest it must match. Without a digest there is no checksum
check. The app does not read the published `.sha256` file and does not verify
an Authenticode signature.

Windows install: the constant PowerShell helper is run by the system
`powershell.exe` with a JSON plan file. It must report `waiting` before the
app quits. After the app exits and nothing else runs from the install folder,
it runs the installer silently in place (`/S /D=<install folder>`), requires
exit code 0 and the expected version in `build-stamp.json`, and relaunches the
app with its original arguments either way. State and log are in
`<data folder>/updates/last-update.{json,log}`; `state_dir` uses the preview
folder name until the data folder has been renamed. There is no rollback.

## Release pipeline

`.github/workflows/release.yml` runs for `v*` tags, PRs, manual dispatches and
a daily 08:00 UTC schedule. The gate checks `VERSION`, chooses nightly mode
for schedule or the nightly input, skips an unchanged nightly SHA, and emits
one timestamp for all legs. When `services/hub-rs/migration.json` exists,
nightly mode also runs `python3 scripts/hub-migration.py ready`; failure
defers the nightly unless a manual dispatch sets both `nightly=true` and
`migration_preview=true`, which `scripts/check-nightly-preview.py` allows only
on the default branch with a successful `ci.yml` push run for that exact SHA.

Jobs and what they publish:

- `build` (Windows x64, macOS arm64, Linux x64): the web client, the Rust
  backend and claudemon; `workspacer-server-<os>-<arch>` and
  `workspacer-claudemon-<os>-<arch>` bundles. The server bundle holds
  `workspacer-rust` (and a `workspacer` copy), `claudemon`, `web/`,
  `examples/`, a README and a build stamp. No Electron installer is built.
- `native-windows-build` then `native-windows-package`:
  `Workspacer-Setup-<v>-x64.exe` and its `.sha256`, written to
  `apps/native/target/package/`.
- `native-macos` (arm64 and x64 legs): `Workspacer-<v>-macos-arm64.dmg` and
  `Workspacer-<v>-macos-x64.dmg`, ad-hoc signed, not notarized, each with a
  `.sha256`.
- `native-linux` (x64 and arm64 legs): `workspacer-<v>-linux-x64.tar.gz` and
  `workspacer-<v>-linux-arm64.tar.gz`, each with a `.sha256`. No AppImage or
  `.deb`.

Version-tag attachment uses a **draft** GitHub release with notes cut from
`CHANGELOG.md`; a missing tagged section fails the notes step. Building a tag
is not publishing: a draft is invisible to `/releases/latest`.

### Windows signing

Signing is conditional. Three steps (Azure login, sign `wks-native.exe` and
`workspacer-rust.exe` before packaging, sign the installer) run only when the
ref is a `v*` tag, the run is not a nightly, and the `AZURE_CODESIGN_PROFILE`
repository variable is non-empty (`apps/native/scripts/sign-windows.ps1`).
Otherwise the installer is unsigned, as before. Do not assert that a given
Windows artifact is signed without checking it.

### Electron feed files on native releases

v0.170.0 was the last Electron release. Installed copies still poll for
`latest*.yml`.

- Stable tags other than `v0.170.0` download v0.170.0's `latest*.yml` and
  attach them **unchanged** to the new draft, so an Electron 0.170.0 install
  asking `/releases/latest` reads "version 0.170.0" and reports up to date.
- The nightly publishes copies rewritten by `scripts/electron-feed-bridge.py`
  so each `url`/`path` is an absolute link to the v0.170.0 asset; the last
  Electron nightly then updates itself to 0.170.0.

Both are best effort: a missing v0.170.0 feed warns and the release proceeds.

## Nightly rollout failure boundary

The publish job waits for all matrix build legs and the native Windows, macOS
and Linux packaging/smoke jobs, serializes on a shared nightly lock,
and checks expected assets before changing the live release. It removes stray
drafts, uploads a new draft, deletes the old live release, deletes/polls the tag
up to six times, then publishes the new draft. A build or pre-deletion upload
failure leaves the old release in place.

`scripts/finalize-nightly.py` validates the uploaded draft against the local asset
names/sizes and exact built commit, publishes with an explicit JSON boolean, then
independently verifies non-draft release state, assets and the actual tag target.
A successful PATCH exit code or a `published_at` timestamp alone is insufficient:
an observed release remained draft with public download URLs returning 404. The
fake-GitHub publication regressions run in primary CI and before release mutation.

This sequence is **not atomic**. After old-release deletion, failed tag cleanup
or failed draft publication can leave no live nightly. The tag check retries
propagation delays, but a failed lookup is not independently distinguished from
an absent tag by that shell condition. Avoid promising uninterrupted feed
availability or automatic rollback. The workflow's own comments use stronger
language than its executable sequence supports.

## Native Windows installer

`native-windows-build` compiles `wks-native` and `native-harness` in release
mode and uploads them with the x64 CRT DLLs from that runner, with file
SHA-256s and provenance. `native-windows-package` downloads those and the
Windows backend executable from the `build` leg, verifies every hash against
the receipts, optionally signs, packages and smoke-tests. It does not compile
Rust.

`apps/native/scripts/resolve-nsis.mjs` finds `makensis`: `MAKENSIS`, then
`PATH`, then the standard Program Files folders, then the copy under
`apps/desktop/node_modules`. The release job uses the runner's NSIS and no
`node_modules`. Icons come from `apps/native/packaging/icons`.

`windows-payload.mjs` stages `wks-native.exe`, standalone
`workspacer-rust.exe`, Visual C++ runtime DLLs, licence/icon/README, optional
examples and build stamp. It includes no Go services, `desktop-host.cjs` or
Node runtime.

The NSIS product and identity are **"Workspacer Native"** / `Workspacer.Native`
(`installer.nsi`). The earlier preview identity ("Workspacer Native Rust
Preview", `Workspacer.Native.RustPreview`) is handled as legacy: with no
`/D=`, the installer installs over this product's recorded folder, else a
preview install's folder, else the default; a preview registration replaced in
place is retired (uninstall key, registry key, shortcuts) so Windows lists one
app; a preview in a different folder is left alone. The installer never
touches `%LOCALAPPDATA%\Workspacer Native` or the preview data folder. The app
itself renames the data folder on first launch (`apps/native/src/data_dir.rs`)
when that is safe; device-local settings keep their preview-path scope key
(`RustOptions::settings_scope`).

Start menu and finish-page launches pass `--local`. Uninstall removes owned
package files and preserves user data and unknown files.

The Windows smoke (`test-windows-installer.ps1`) covers install, upgrade from
a preview, payload hashes, shortcuts, registrations, the installed CLI's
`--help`, and `native-harness rust-probe` with Node absent from PATH. The
harness is not the installed GUI executable: this is backend/installer
evidence, not visual launch verification.

`.github/workflows/rust-native-preview.yml` separately runs Rust service tests
and preview packaging/smoke on PRs and manual dispatch. It publishes nothing
and is not a `needs` dependency of `release.yml`.

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

## Electron (final 0.170.0)

`apps/desktop/src/main/services/updateService.ts` (electron-updater) remains
in the tree but no release builds an Electron installer. Installed Electron
copies stay on 0.170.0: stable installs read the carried-forward
`latest*.yml`; nightly installs are bridged to 0.170.0 as above. Electron
packaging (`make package-desktop`, `apps/desktop/electron-builder.yml`) still
works locally and is not part of a release.

## Validation scope

Reviewed 2026-11-14 by reading `updates.rs`, `update_helper.ps1`,
`installer.nsi`, the packaging scripts and `release.yml` in the working tree.
No workflow run, signing, publication or installed update was exercised for
this review. See [deployment](../modules/fly-node-deploy.md) for artifact
stamp validation and [serve CLI](../modules/workspacer-serve-cli.md) for
headless bundle ownership.
