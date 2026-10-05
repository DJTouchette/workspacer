---
title: GPUI scroll containers count absolute measuring canvases as content (phantom scroll range)
date: 2026-10-05
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui.rs
  - vendor/gpui/src/elements/div.rs
promoted: false
---

# GPUI scroll containers count absolute measuring canvases as content (phantom scroll range)

## Observation
GPUI Div::prepaint computes a scroll container's content_size as the union of ALL child layout bounds, absolute children included, then clamp_scroll_position adds the container's padding on top. The native conversation dock (overflow_y_scroll) held its measuring canvas(...).absolute().top_0().left_0().size_full(), so content_size >= bounds and scroll_max.y = pt+pb (28px, 16px compact) even when everything fit. Wheel/trackpad input over the dock's non-occluded gutters or card gaps (the wheel smoother deliberately skips composer_dock_bounds) scrolled the composer/card top edge under the dock clip, and the offset persisted across session switches because scroll state is keyed by element id: bug #26 'top tip of the composer bar gets cut off sometimes'. Reproduced in a GPUI test and real Xvfb (1 wheel = 28px flat cut). Also: a display:none child sits at (0,0) and would extend the union the same way.

## Impact
Any overflow_*_scroll element with a size_full measuring child or hidden child has a phantom range; symptoms are intermittent clipping after a wheel.

## Recommendation
Measure from a non-scrolling frame around the scroll container (native dock: chrome::chat_frame with the canvas, cards in their own capped scroll layer, composer layer never scrolls). Regression: conversation_dock_wheel_never_clips_the_composer_top.
