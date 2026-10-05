---
title: Hub already serves project icons: desktop.downloadProjectIcon + ui.asset kind icon
date: 2026-10-05
confidence: high
related_paths:
  - apps/native/src/projects.rs
  - apps/native/src/features.rs
  - services/hub-rs/src/services/ui_assets.rs
promoted: false
---

# Hub already serves project icons: desktop.downloadProjectIcon + ui.asset kind icon

## Observation
The Rust hub (services/hub-rs/src/services/ui_assets.rs) implements desktop.downloadProjectIcon {url} (http(s) image types, 2 MiB, content-addressed <sha256[..32]>.<ext> under <configDir>/project-icons) and ui.asset {file, kind:'icon'} (dataBase64+mime). Desktop ProjectsSection writes favicon (URL provenance) + iconFile (what renders); reset writes empty strings. Native project identity now uses these same methods/fields; it removes cleared fields instead of writing '' so a pin-only entry stays forgettable, and writes all same_dir aliases. gpui::Image cannot decode .ico; native decodes icons off-thread with the image crate (ico feature added, no Cargo.lock change) to a 64px PNG and refuses SVG.

## Impact
Project identity across desktop/native/TUI needs no new hub API; a native-only icon store would fork device-local truth.
