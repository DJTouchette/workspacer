---
title: Native tool cards need explicit language grammars and bounded result joins
date: 2026-09-27
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/Cargo.toml
  - apps/native/Cargo.lock
  - apps/native/src/model.rs
  - apps/native/src/tool_preview.rs
  - apps/native/src/ui/tools.rs
  - apps/native/src/ui.rs
promoted: false
---

# Native tool cards need explicit language grammars and bounded result joins

## Observation
GPUI Component 0.5.1 with default-features=false enables only the JSON grammar; code fences alone do not highlight Rust/TS/diff. Native now enables tree-sitter-languages (without webview), with cc pinned by the resolved lockfile to 1.2.67 because tree-sitter-sequel requires ~1.2.1. Native Row retains bounded tool ID/name plus serialized arguments and joined output; raw event seq remains independent of the reduced row count. Result joins must replace Arc<Row> so controller/UI incremental invalidation sees updates, and retained memory accounting must include metadata and results. Floating header and composer use measured List padding, not a smaller viewport.
