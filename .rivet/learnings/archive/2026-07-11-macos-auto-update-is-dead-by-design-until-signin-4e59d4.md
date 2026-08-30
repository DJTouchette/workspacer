---
title: macOS auto-update is dead by design until signing + zip target land
date: 2026-07-11
confidence: high
related_paths:
  - apps/desktop/electron-builder.yml
  - apps/desktop/src/main/services/updateService.ts
promoted: true
---

# macOS auto-update is dead by design until signing + zip target land

## Observation
apps/desktop/electron-builder.yml mac.target is only `dmg` (no `zip`) and has no CSC/notarization config. electron-updater's macOS updater requires a zip artifact and a code-signed app to apply updates in-place; without both it emits an `error` event instead of `update-available`. apps/desktop/src/main/services/updateService.ts treats ALL updater errors as non-fatal/log-only (never a user dialog, per the `error` handler at line 209-212), so on the current mac build the auto-update feature silently never does anything — no crash, no user-visible sign of failure, no code change needed later since the wiring already handles it once signing+zip land.

## Impact
Anyone debugging "why doesn't the mac build ever show an update" will not find an exception or error dialog — only a console.warn. This is intentional per the file's header doc-comment but easy to mistake for a bug. Also: mac is silently excluded from the update mechanism entirely right now, not just from nightly.

## Recommendation
When adding mac code signing, also add a `zip` target under mac.target in electron-builder.yml (electron-updater needs it for mac); the updateService code path needs no changes.

## Disposition
Not folded: already reflected in .rivet/context/domains/auto-update-release-channel.md (the "macOS auto-update is currently a dead path by design" gotcha covers this in full).
