---
title: Native title actions reveal on hover via clipping, not visibility
date: 2026-10-05
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui/island.rs
  - apps/native/src/ui/chrome.rs
promoted: false
---

# Native title actions reveal on hover via clipping, not visibility

## Observation
The conversation title capsule's secondary actions (chat_actions) are hidden at rest by ui/island.rs: a zero-width overflow_hidden clip. In pinned GPUI, Frame::hit_test intersects each hitbox with its content mask, so clipped controls have no hit area. They still paint and register tab stops, because only visibility Hidden skips tab_stops.insert, so Tab reaches them. Four traps found while building it. (1) Focusable controls take focus on mouse-down, so 'focus within reveals' kept the island open after any click; only focus not left by a pointer press counts. The bar's bubble on_mouse_down runs after the child's focus transfer (bubble listeners run in reverse registration order), so it records window.focused there. (2) track_focus on the bar would steal composer focus on surface clicks; the bar prevent_defaults its own mouse-down. (3) Div::on_hover keeps state per element (stale after remount), and MouseExitEvent never updates hover. The capsule uses a canvas hitbox plus window MouseMove/MouseExit listeners compared against Workspace state. (4) The notice tray width is a fixed point of the measured island width, so widening the bar on hover would rewrap notices. With notices present, the bar reserves the actions' room and centers its content (pl = rem + hidden/2).

## Impact
Hidden-but-reachable chrome in native GPUI: opacity(0) (transcript copy buttons) still takes clicks; zero-width clipping does not.

## Recommendation
Reuse reveal_title_actions for new title actions. Every pointer on_click must check title_action_allowed(event), which allows keyboard clicks and pointer clicks only once fully revealed. Tests: reveal_title()/settle_title() helpers; a focused terminal swallows ctrl-backtick.
