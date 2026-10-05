---
title: Native UI tests: use secondary- for platform shortcuts and real element bounds under app-drawn caption
date: 2026-10-05
author: claude (native AV worker)
confidence: high
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/ui/chrome.rs
  - apps/native/src/ui/file_viewer.rs
promoted: false
---

# Native UI tests: use secondary- for platform shortcuts and real element bounds under app-drawn caption

## Observation
macOS failed file_viewer_contains_keys_over_a_nonempty_draft because gpui-component Input binds SelectAll/Copy to cmd-a/cmd-c on macOS (ctrl-a is MoveToLineStart there); 'secondary-' in Keystroke::parse maps to cmd on macOS, ctrl elsewhere. Windows failed chat_links_route_web_refused_and_image_targets because custom_caption() is true on Windows and the file-viewer backdrop starts at CAPTION_HEIGHT=32, so a click at (4,4) hits the caption strip; reproducible on Linux with WKS_NATIVE_CAPTION=1.

## Impact
Linux-green UI tests can still fail on macOS/Windows because of platform keymaps and Windows-only chrome.

## Recommendation
Use secondary-* for editor shortcuts in tests; aim clicks from debug_bounds(...) of the target; run caption-sensitive tests locally with WKS_NATIVE_CAPTION=1.
