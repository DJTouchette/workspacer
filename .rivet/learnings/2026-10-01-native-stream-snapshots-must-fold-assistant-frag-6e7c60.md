---
title: Native stream snapshots must fold assistant fragments like live deltas
date: 2026-10-01
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/model.rs
  - apps/native/src/controller.rs
promoted: false
---

# Native stream snapshots must fold assistant fragments like live deltas

## Observation
Transcript snapshot and reset previously pushed fragments with streaming=false, while live deltas coalesced them. A bold phrase split across assistant_text items became separate Markdown documents after a read or resync. Controller snapshot reads and reset folding now retain stream transport semantics; PTY snapshots stay separate.
