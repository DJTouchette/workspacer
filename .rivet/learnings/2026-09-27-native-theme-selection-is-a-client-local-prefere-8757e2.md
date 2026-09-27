---
title: Native theme selection is a client-local preference
date: 2026-09-27
suggested_doc: theme-system
related_paths:
  - apps/native/src/appearance.rs
  - apps/native/src/ui.rs
promoted: false
---

# Native theme selection is a client-local preference

## Observation
Native appearance.rs owns Dark/Light/Nord palettes and native-theme.json, separate from shared config.yaml. UI palette and GPUI Theme must switch together, including ThemeMode for Markdown syntax highlighting. Startup falls back to Dark on malformed settings; save errors remain visible in the sidebar.
