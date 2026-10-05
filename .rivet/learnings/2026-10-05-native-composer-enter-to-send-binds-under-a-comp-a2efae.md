---
title: Native composer Enter-to-send binds under a ComposerEnter context; GPUI propagate falls through to the next binding
date: 2026-10-05
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/ui/features.rs
promoted: false
---

# Native composer Enter-to-send binds under a ComposerEnter context; GPUI propagate falls through to the next binding

## Observation
GPUI dispatch tries each matching binding in order and only stops when a handler does not propagate (vendor/gpui window.rs dispatch loop), so a composer action can cx.propagate() to let gpui-component Input's own Enter (newline / IME commit) run. KeyBinding '>' is any-ancestor. The floating composer's key_context is 'Composer ComposerEnter' only when Settings→Keyboard→Send with Enter is on, so 'enter'→ComposerEnter and 'shift-enter'→input::Enter{secondary:false} bind only there. IME: check EntityInputHandler::marked_text_range (call via the trait path; InputState's ime_marked_range is private). GPUI tests: images in gpui::img have no intrinsic size until async decode, so give thumbnails fixed w/h or hit tests see a 2px element; Settings entries can sit below the 700px test window, so resize before clicking.
