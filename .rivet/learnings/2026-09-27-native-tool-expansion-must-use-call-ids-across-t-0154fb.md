---
title: Native tool expansion must use call IDs across transcript reseeds
date: 2026-09-27
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/ui/tools.rs
  - apps/native/src/tool_preview.rs
  - apps/native/src/model.rs
promoted: false
---

# Native tool expansion must use call IDs across transcript reseeds

## Observation
Transcript::snapshot recreates Row keys using a monotonically increasing next_key. Expansion state and detail element IDs keyed by Row.key therefore reset whenever the same server history is re-seeded, closing cards a moment after opening. Native now keys expansion and detail/section IDs by session plus tool call ID, prunes against retained tool identities, and resolves the current row index on click. Regression coverage opens a command then re-seeds repeatedly with shifted rows and a late result, and checks deliberate collapse also persists. Tool descriptions are optional arguments; Codex commandExecution normalization currently keeps command only, including argv arrays, so the client needs an honest command-derived title fallback.
