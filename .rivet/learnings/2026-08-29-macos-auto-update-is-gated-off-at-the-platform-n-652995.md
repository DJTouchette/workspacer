---
title: macOS auto-update is gated OFF at the platform, not silently failing
date: 2026-08-29
confidence: high
suggested_doc: auto-update-release-channel
related_paths:
  - apps/desktop/src/main/services/updateService.ts
  - apps/desktop/src/renderer/src/components/settings/UpdatesSection.tsx
  - apps/desktop/src/renderer/src/components/CommandPalette.tsx
promoted: false
---

# macOS auto-update is gated OFF at the platform, not silently failing

## Observation
updateService.ts used to run electron-updater on every platform; on macOS (unsigned, dmg-only, no zip) every check ended in `error` — at launch and then every 4h forever. As of the darwin-gate change there is a NO_AUTO_UPDATE_PLATFORMS list ['darwin'] + exported autoUpdateSupportedOn(platform); start() returns early with a new UpdateStatus state 'manual' BEFORE the updates.enabled config gate, so no listeners are wired and no interval is scheduled. checkNow() on a gated platform opens the releases page via openExternalUrl instead of checking. The 'manual' state surfaces in the command palette ("Download the Latest Release" instead of "Check for Updates") and in Settings → Updates (warning copy + the enabled toggle rendered disabled, fed by useAppVersion() which now also returns updateState).

## Impact
The context doc's gotcha "macOS auto-update is a dead path that fails silently — no updateService.ts change will be required when signing lands" is now stale on both halves: it fails loudly-but-honestly (state 'manual'), and re-enabling it requires deleting 'darwin' from NO_AUTO_UPDATE_PLATFORMS in updateService.ts once signing + a mac zip target land.

## Recommendation
When mac signing + a `zip` target land in electron-builder.yml, remove 'darwin' from NO_AUTO_UPDATE_PLATFORMS; nothing else changes. Note updateService.test.ts now pins process.platform per test (beforeEach sets 'linux'), so the main suite no longer changes meaning when run on a developer's mac.
