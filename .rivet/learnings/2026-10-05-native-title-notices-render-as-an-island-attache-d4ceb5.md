---
title: Native title notices render as an island attached to the title capsule
date: 2026-10-05
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui/chrome.rs
  - apps/native/src/ui.rs
promoted: false
---

# Native title notices render as an island attached to the title capsule

## Observation
Conversation-screen notices (status = local_notice else view.notice, extras.notice, connection banner, refresh progress, omitted-history hint, conversation-unavailable Retry) are built by Workspace::render_title_island in ui/chrome.rs, which wraps render_title_bar/render_child_title_bar in one surface (ISLAND_RADIUS 20 = half the 40px capsule, floating_shadow, border). The tray width comes from the measured capsule outline (title_bar_width) because a w_0()+min_w_full() tray makes taffy measure text at width 0 (a 970px-tall island). Inside the island the bar drops its border and 2px of height so the outline is pixel-identical. Dismissal records (slot, text) in extras.dismissed_notices and never clears the owner (states.rs reads local_notice prefixes); Retry rows are not dismissible. Sidebar UI-bus notices (#4) are separate.

## Impact
New conversation notices should be island rows, not loose header children.

## Recommendation
Add a slot in render_title_island; test with title_notices_grow_the_capsule_into_one_island.
