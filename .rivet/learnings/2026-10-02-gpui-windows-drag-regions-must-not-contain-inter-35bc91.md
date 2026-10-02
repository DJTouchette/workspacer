---
title: GPUI Windows drag hitboxes respect occlusion; focus default prevention cancels moves
date: 2026-10-02
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/ui/sidebar.rs
  - apps/native/src/ui/chrome.rs
  - apps/native/src/ui/navigation.rs
promoted: false
---

# Correction to the original drag-region learning

## Observation
The earlier claim that occluding descendants leave ancestor Drag hitboxes eligible was incorrect. In pinned GPUI 0.2.2, `Interactivity::occlude_mouse` sets `BlockMouse`; `Frame::hit_test` walks hitboxes in reverse and stops at that hitbox. The native control callback only considers retained IDs. A later occluding title pill or button group therefore protects its entire bounds from an underlying Drag region. Element IDs are not required to register native control hitboxes.

The actual source-level cancellation path is different: `Workspace::shell` tracks focus, and GPUI's automatic focus mouse-down listener calls `window.prevent_default()`. Windows `handle_nc_mouse_down_msg` returns without native default handling when input dispatch reports `default_prevented`. Original commit cd7a5028 added non-occluding Drag regions inside that shell, allowing shell focus to cancel HTCAPTION movement. The drag surface itself must occlude the focusable shell. This is source/harness evidence, not a Windows runtime observation.

## Recommendation
Use occluding, non-focusable drag surfaces with later occluding interactive controls, or explicit disjoint surfaces. Test both positive geometry and mouse-down default propagation. Keep a reachable drag target with the sidebar collapsed on every screen. `start_window_move` has no Windows implementation in this pinned GPUI version.
