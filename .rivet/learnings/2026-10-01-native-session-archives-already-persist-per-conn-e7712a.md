---
title: Native session archives already persist per connection
date: 2026-10-01
suggested_doc: session-lifecycle
related_paths:
  - apps/native/src/ui/features.rs
  - apps/native/src/ui/sidebar.rs
promoted: false
---

# Native session archives already persist per connection

## Observation
Native Settings.archived stores session IDs per connection scope. visible_sessions filters them, and Session history has Archived/Restore controls. Sidebar archive actions can reuse toggle_archive without any backend terminate or select command.
