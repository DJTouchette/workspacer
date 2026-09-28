---
title: Native rich transcript keeps payload and action boundaries through rendering
date: 2026-09-28
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui/transcript.rs
  - apps/native/src/model.rs
  - apps/native/scripts/resolve-nsis.mjs
promoted: false
---

# Native rich transcript keeps payload and action boundaries through rendering

## Observation
Native Row now retains bounded tool IDs/input/output and pairs results by ID, including empty errors. Snapshot reconciliation reuses unchanged row identities. History uses the same renderer with lossless retained tool payloads and paginated literal long text. Reading bookmarks use role/timestamp/tool identity plus a text prefix and occurrence rather than transient list indices, and record content fingerprints. GPUI ListState invokes scroll handlers under an internal mutable borrow: bookmark capture must be deferred before calling logical_scroll_top. Native response HTML is parsed into an inert allowlisted fragment; user/tool text never activates actions, and card diffs call desktop.htmlCardReadDiff with ownerId/target. Release run 36361641744 failed because Chocolatey could not find nsis, not because installer compilation failed; reuse the pinned electron-builder getMakeNsisPath compiler and propagate NSISDIR.

## Impact
Preserves reading and tool context without losing memory bounds or giving model-authored HTML an execution path. Avoids a repeatable Windows release dependency failure.

## Recommendation
Keep live/History rendering shared; run the native suite and rich-transcript smoke after changes. Treat CSS/JS as unsupported native content, preserve fallback text, and keep compiler path resolution with its returned environment.
