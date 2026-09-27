---
title: Native Vim bindings require direct root focus and focus notifications
date: 2026-09-27
suggested_doc: pane-system
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/ui/navigation.rs
  - apps/native/src/navigation.rs
promoted: false
---

# Native Vim bindings require direct root focus and focus notifications

## Observation
Single-letter native shortcuts bind to VimNormal, added to the Workspace key context only while its root focus handle is directly focused and Vim navigation is enabled. Root focus/blur subscriptions notify Workspace so mouse or keyboard entry into any Input removes that context before subsequent keystrokes. All screens share shell action wiring; Projects/Settings must not send a hidden composer draft. Project bookmarks live in native-settings.json keyed by sanitized hub endpoint, separate from shared config.yaml.
