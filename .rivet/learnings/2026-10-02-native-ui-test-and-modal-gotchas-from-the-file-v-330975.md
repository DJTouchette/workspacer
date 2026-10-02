---
title: Native UI-test and modal gotchas from the file viewer work
date: 2026-10-02
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/ui/smooth_scroll.rs
  - apps/native/src/ui.rs
promoted: false
---

# Native UI-test and modal gotchas from the file viewer work

## Observation
(1) gpui-component TextView debounces Markdown parsing with a real async-io Timer the GPUI test executor does not drive, so clicking freshly rendered Markdown can miss under CPU load; retry clicks within a bounded real-time wait. (2) The native UI suite is flaky in parallel (guided_launch, keyboard_controls, markdown_table rotate); serialized --test-threads=1 is reliable. (3) smooth_scroll's wheel_smoother captures wheel events over the chat in the CAPTURE phase, so any modal over the conversation must be excluded there (usage_open, file_viewer) or the transcript scrolls behind it. (4) InputState::set_cursor_position before first layout moves the cursor but cannot scroll (scroll_to needs last_layout); re-apply after a frame. (5) Esc with an Input focused dispatches NormalMode via the 'Workspace > Input' binding before key listeners, so capture_key_down never sees it. (6) rivet witness select --format exec emits 'npx jest' for .rs files; run cargo test yourself.

## Impact
Saves the next native UI change from flaky tests, scroll-through modals and dead Esc handlers.
