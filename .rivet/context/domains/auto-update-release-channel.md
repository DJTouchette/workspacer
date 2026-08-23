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
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# In-app auto-update (electron-updater, stable vs nightly channels)

## Overview

Workspacer's Electron desktop app auto-updates in place using `electron-updater`
against the project's GitHub Releases. The whole subsystem lives behind one
singleton, `updateService` in `apps/desktop/src/main/services/updateService.ts`,
wired into `apps/desktop/src/main/index.ts` on window creation and torn down on
app shutdown. It only does anything in a packaged production build
(`app.isPackaged`); dev builds have no code signature or update feed and
`electron-updater` would throw, so the service no-ops with a log line. On
startup, and every 4 hours thereafter, it polls the release feed, downloads a
newer build silently in the background, then prompts the user (via
`dialog.showMessageBox`) before calling `autoUpdater.quitAndInstall()`. The
feature exists to let users stay current without a manual download/reinstall,
and to support a separate rolling **nightly** prerelease channel for early
testers that never mixes with the stable release feed.

## Key modules

- `apps/desktop/src/main/services/updateService.ts` — the whole subsystem: `UpdateService` class wraps `electron-updater`'s `autoUpdater`, exposes `start(win)`, `stop()`, `checkNow()`, `installNow()`, `getStatus()`; owns the `UpdateStatus` renderer-visible state machine (`unsupported | disabled | idle | checking | downloading | downloaded | error`).
- `apps/desktop/src/main/services/updateService.test.ts` — vitest suite with a real `EventEmitter`-backed mock of `autoUpdater`; drives every lifecycle event (`checking-for-update`, `update-available`, `download-progress`, `update-downloaded`, `error`) and both gating branches (dev, `updates.enabled=false`).
- `apps/desktop/src/main/services/configService.ts` — source of the `updates` config block (`readUpdatesConfig()` in updateService.ts reads `configService.getConfig().updates`, defaulting `enabled: true`, `channel: 'latest'` when absent).
- `apps/desktop/src/main/ipc.ts` (around line 351-354) — registers `IPC.UPDATES_STATUS_GET → updateService.getStatus()`, `IPC.UPDATES_CHECK → updateService.checkNow()`, `IPC.UPDATES_INSTALL → updateService.installNow()`.
- `apps/desktop/src/main/shared/ipcChannels.ts` — defines `UPDATES_STATUS_GET` (invoke, pull current status), `UPDATES_STATUS` (push, main → renderer on every state transition).
- `apps/desktop/src/main/preload.ts` (~line 399-405) — bridges `updatesGetStatus`, and an `onUpdateStatus` subscription over `ipcRenderer.on(IPC.UPDATES_STATUS, ...)`.
- `apps/desktop/src/renderer/src/App.tsx` (~line 288-302, ~2046-2054) — owns `updateStatus` React state, pulls it once via `updatesGetStatus()` + subscribes via `onUpdateStatus`, and wires the command palette's "check for updates" / "install update" actions to `window.electronAPI.updatesCheck()` / `updatesInstall()`.
- `apps/desktop/src/renderer/src/hooks/useAppVersion.ts` — small hook pulling `current` off `updatesGetStatus()` for display; derives `isNightly` purely by checking `version.includes('-nightly')`.
- `apps/desktop/src/renderer/src/components/settings/UpdatesSection.tsx` — Settings UI: toggles `updates.enabled`, shows current version + nightly badge, static explanatory copy (does not expose the channel selector as UI — `updates.channel` is config-file-only today).
- `apps/desktop/src/renderer/src/backend/webBackend.ts` (~line 164-167) and `bridgedBackend.ts` (~line 94-96) — the web-mirror / bridge stubs that make `updatesGetStatus/-Check/-Install` always resolve `{ state: 'unsupported' }` outside the desktop shell (in-app updates are declared a "desktop-shell concern").
- `apps/desktop/electron-builder.yml` — `publish: { provider: github, owner: DJTouchette, repo: workspacer }` generates the update metadata (`latest.yml` / `latest-linux.yml` / `latest-mac.yml` + `.blockmap`); CI packages with `--publish never` and the release workflow attaches installers + metadata to the GitHub Release itself. `mac.target` is `dmg` only (no `zip`), no CSC/notarization block configured yet.
- `.github/workflows/release.yml` — `gate` job decides stable-vs-nightly and stamps a shared nightly timestamp (`X.Y.Z-nightly.<stamp>`) across all three OS legs so they agree; `publish-nightly` job atomically rolls the rolling `nightly` prerelease tag only after all three legs build successfully (a failed leg leaves yesterday's nightly live, never nothing).

## Failure modes

- **Dev/unpackaged builds**: `start()` returns immediately after `console.log('[updateService] dev build — auto-update disabled')`; `checkNow()` also short-circuits and returns `this.status` unchanged (`state: 'unsupported'`), never touching `autoUpdater`.
- **`updates.enabled: false`**: `start()` sets `state: 'disabled'` and returns without wiring listeners or starting the timer. `checkNow()` still works here (see Gotchas) — an explicit "check now" click bypasses the disabled gate.
- **Updater errors (any reason — offline, feed 404 before first release, unsigned-mac refusal, etc.)**: the `error` listener logs at `console.warn` and sets `state: 'error', error: <message>` — never throws, never shows a dialog. `check()` also wraps `autoUpdater.checkForUpdates()` in try/catch purely to prevent an unhandled promise rejection (the `error` listener has already logged by the time the catch runs).
- **macOS unsigned build**: electron-updater refuses to apply an unsigned update and emits `error` instead of `update-available` — this looks identical to any other network/feed failure from the app's perspective (see Gotchas — currently the *only* path on mac, since there's no zip target either).
- **Overlapping checks**: `promptOpen` guards `onDownloaded()` so a startup check and a manual "check now" landing at the same time can't stack two restart dialogs.
- **Window mid-teardown**: `setStatus()` wraps the `webContents.send` in try/catch — if the window is destroyed between event and send, the push is silently dropped; the renderer re-pulls `updatesGetStatus()` on next mount instead.
- **Nightly rolling tag has a stable-tag naming trap**: the release workflow explicitly verifies the `nightly` git tag is gone before recreating it (`if gh api ".../refs/tags/nightly" ...; then echo "nightly tag still exists — aborting..."`) — a partial delete/create race would otherwise publish a mistargeted release.

## Gotchas

- **Nightly and stable use entirely different `electron-updater` providers so their feeds never cross.** Stable builds use the default `github` provider (`electron-builder.yml`'s `publish` block), which resolves GitHub's `/releases/latest` — GitHub keeps that endpoint free of prereleases automatically. Nightly builds (detected via `app.getVersion().includes('-nightly')`) call `autoUpdater.setFeedURL({ provider: 'generic', url: 'https://github.com/DJTouchette/workspacer/releases/download/nightly', useMultipleRangeRequest: false })` instead — the GitHub provider can't be pointed at a prerelease because it parses release tags as semver, and the rolling tag is literally the string `"nightly"`.
- **`useMultipleRangeRequest: false` on the nightly feed is load-bearing, not cosmetic.** GitHub's release-asset CDN 501s multipart Range requests; single-range gets a 206, which is required for `electron-updater`'s blockmap differential downloads to work at all. (electron-updater's own built-in GitHub provider forces this internally too — the generic-provider config has to opt in explicitly.)
- **Nightly forces `autoUpdater.channel = 'latest'` regardless of the configured `updates.channel`.** The nightly feed only ever publishes `latest*.yml` (no `beta.yml` etc.); if a user had `updates.channel: 'beta'` configured, the generic provider would request `beta.yml` from the nightly feed, 404, and silently kill nightly updates entirely. Channel selection (`updates.channel`) is a stable-feed-only concept — see `updateService.ts` line ~184.
- **`allowDowngrade = true` is set only for nightly builds.** Rolling-nightly version stamps (`X.Y.Z-nightly.<stamp>`) are not guaranteed monotonic across stamp-format changes over time, so nightly explicitly trusts whatever the feed currently serves rather than semver ordering — stable builds keep the default (no downgrades).
- **macOS auto-update is currently a dead path by design, and fails silently.** `electron-builder.yml`'s `mac.target` is `dmg` only — no `zip` target and no code-signing (`CSC_*`) config. `electron-updater` on macOS needs both a signed app AND a zip artifact to apply updates in place; without them it emits `error` instead of `update-available`. Since **every** updater `error` is treated as non-fatal/log-only (never a user dialog — this is a deliberate product choice, see the file's header comment), the mac build will silently never update, indistinguishable from a transient network failure, until someone adds mac signing + a zip target to `electron-builder.yml` — no `updateService.ts` change will be required when that lands.
- **`checkNow()` bypasses the `updates.enabled=false` gate on purpose.** An explicit palette "check for updates" click is treated as explicit user consent and works even when the config toggle is off; `start()` (automatic path) is the only thing gated by `updates.enabled`.
- **Windows nightly installer artifact names are sanitized to avoid a 404.** The release workflow overrides `-c.nsis.artifactName` / `-c.portable.artifactName` to be space-free (`${productName}-Setup-${version}.${ext}`) for nightly builds only: GitHub rewrites spaces in asset names to dots for display, but the generic update feed requests the literal filename recorded in `latest.yml`, so the default `"Workspacer Setup <v>.exe"` naming would 404 the nightly updater specifically. Stable keeps the default naming since the GitHub provider handles that dot-rename itself and the landing page links the default names.
- **Windows code signing (Azure Trusted Signing) is explicitly skipped for nightly even on a `v*` tag dispatch.** The workflow's signing `if:` condition adds `&& needs.gate.outputs.nightly != 'true'` — nightlies always take the unsigned packaging path so the artifact-name override applies and the release notes' "Unsigned." claim stays true.
- **`autoInstallOnAppQuit` is deliberately `false`; `autoDownload` is `true`.** The product choice is: download silently in the background as soon as an update is found, but only ever prompt the user at the install/restart step (`onDownloaded()`'s `dialog.showMessageBox`) — never auto-install without asking, and never ask twice for the same download (see `promptOpen` guard).
- **`updates.channel` has no renderer UI today.** `UpdatesSection.tsx` only exposes the `enabled` toggle; `channel` can only be set by hand-editing `config.yaml`'s `updates.channel` key (defaults to `'latest'` if the block or key is absent — see `readUpdatesConfig()`).
- **The web-mirror / bridge builds never see real update state.** `webBackend.ts` and `bridgedBackend.ts` stub all three update IPC calls to resolve `{ state: 'unsupported' }` — in-app updates are declared a desktop-shell-only concern; there is no equivalent update mechanism for the hub/brain/claudemon daemons or the `workspacer serve` CLI bundle.
