---
title: Native sidebar filtering combines search and project scope
date: 2026-09-29
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/ui/navigation.rs
promoted: false
---

# Native sidebar filtering combines search and project scope

## Observation
The native sidebar is rendered in apps/native/src/ui.rs, separately from ui/navigation.rs. visible_sessions combines search and project_filter and excludes archived sessions. Recent-session navigation uses Screen::Recent; Screen::History is the selected conversation's retained history. The sidebar used a fixed 264px width even at the 720px window minimum.
