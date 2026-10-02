---
title: GPUI Windows native dragging is cancelled by focusable ancestors unless the drag hitbox occludes
date: 2026-10-02
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/ui/chrome.rs
  - apps/native/src/ui/navigation.rs
  - apps/native/src/ui.rs
  - apps/native/src/ui/sidebar.rs
promoted: false
---

# GPUI Windows native dragging is cancelled by focusable ancestors unless the drag hitbox occludes

## Observation
Pinned GPUI 0.2.2 div.rs paint_mouse_listeners automatically calls window.prevent_default() for hovered track_focus ancestors. Workspace::shell already had track_focus in cd7a5028. Windows events.rs handle_nc_mouse_down_msg dispatches input and returns Some(0) when default_prevented, preventing DefWindowProc's HTCAPTION move. Original drag_region did not occlude, so shell focus handling cancels drag even though WM_NCHITTEST returns HTCAPTION. Frame::hit_test does respect occlude/BlockMouse; making the drag surface itself occlude excludes the focusable ancestor as well as underlying selectable content.

## Impact
A valid native Drag hitbox alone does not prove a native drag starts. Title-pill occlusion protects controls but does not protect blank drag areas from the focusable shell.

## Recommendation
Test mouse-down default prevention on actual rendered drag surfaces through Window::default_prevented(); DispatchEventResult is private to GPUI. Do not stop propagation on native drag surfaces. Keep interactive title pills occluding and add an independent collapsed-sidebar drag surface. Source/harness evidence is not Windows runtime verification.
