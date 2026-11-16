---
title: Hub-run config keys had no native Settings control; HubSettings request now covers them
date: 2026-11-16
confidence: high
suggested_doc: config
related_paths:
  - apps/native/src/features.rs
  - apps/native/src/ui/hub_settings.rs
  - apps/native/src/ui/settings.rs
  - services/hub-rs/assets/config-defaults.json
promoted: false
---

# Hub-run config keys had no native Settings control; HubSettings request now covers them

## Observation
After Electron's retirement, several config keys the Rust hub still acts on were editable only by hand in config.yaml: claude.keepWarm.*, agents.artifactCleanup.{enabled,minAgeHours}, agents.fleetFullAccess, agents.managerProvider, agents.checkProviderOnStartup, usage.pollOnBoot. Native now exposes them through one generic Request::HubSettings { set: Option<HubChange> } (apps/native/src/features.rs). HubChange is a JSON pointer + value, saved as a nested config.save patch and verified on readback, with numbers compared by value. hub_settings() mirrors the hub's defaults: artifactCleanup is ON unless `enabled` is present and not true; keepWarm.providers null means [claude]. UI is in apps/native/src/ui/hub_settings.rs, under Settings → Background plus two Agents rows. Leftover keys with no runtime reader: agents.statusSummary is only normalized in config.rs, and defaultCwd/fleetRoot have no hub-rs reader.

## Impact
Before adding another hub-shared toggle, reuse HubChange instead of adding a bespoke Request variant like ChildAccess/JobAccess/ExternalMcp. usage.pollOnBoot is read only at backend boot and the WORKSPACER_USAGE_POLL_ON_BOOT env var overrides it.

## Recommendation
New hub-shared boolean/enum settings: add a HubChange pointer + a field in hub_settings(), and teach harness.rs save_config the key.
