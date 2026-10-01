---
title: Native structured worker results share parent and child rendering
date: 2026-10-01
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/transcript.rs
  - apps/native/src/ui/markdown.rs
  - apps/native/src/ui/transcript.rs
promoted: false
---

# Native structured worker results share parent and child rendering

## Observation
Native fleet reports previously preserved Structured result JSON as raw text, and assistant_blocks only recognized HTML cards. wks-result fences now become structured cards; completion wake Structured result sections use the same renderer, and child previews already call render_message so gain these cards without separate parsing. Invalid payloads retain raw text and all schema fields remain visible.
