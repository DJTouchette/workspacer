---
title: Native transcript parity requires preserving structured events
date: 2026-09-28
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/model.rs
  - apps/native/src/ui.rs
  - apps/native/src/features.rs
promoted: false
---

# Native transcript parity requires preserving structured events

## Observation
Audited native GPUI against Electron at 67555a92. apps/native/src/model.rs Item::display flattens tool_use/tool_result into role/text; Row retains no tool IDs, structured input, or timestamp. Both live chat and features.rs history_document use this projection. ui.rs uses TextView::markdown for both user and assistant text, unlike Electron literal user bubbles, and has no Workspacer attachment-marker thumbnail rendering. On selected-session changes ui.rs resets ListState and follow=true; Electron-only commit db0dc488 added persisted reading anchors and unread markers. Native Changes is current git state, not Electron frozen per-turn ChangedFilesCard. Native History preserves long text via 32 KiB chunks, but rendering each chunk independently can split Markdown fences. Recon search for message omitted native symbols; direct native source inspection was required, so negative recon results are not evidence of absence.

## Impact
Cosmetic row styling alone cannot achieve transcript parity; tool correlation and stable reading anchors require retaining event identity through the bounded model.

## Recommendation
Preserve structured event identity within existing memory bounds, then add paired collapsible tools and inline diff/read views; implement per-session reading restoration and unread markers, attachment-aware rendering, and literal user text. Share transcript presentation between live and History views. Validate with mixed tool/result, image-marker, reconnect, and session-switch fixtures.
