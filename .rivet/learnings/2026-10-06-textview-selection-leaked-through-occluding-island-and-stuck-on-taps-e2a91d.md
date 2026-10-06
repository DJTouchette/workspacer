---
title: TextView selection leaked through occluding chrome and stuck on one-frame taps
date: 2026-10-06
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - vendor/gpui-component/src/text/text_view.rs
  - apps/native/src/ui/chrome.rs
  - vendor/gpui/src/app/test_context.rs
  - vendor/gpui/src/window.rs
promoted: false
---

# TextView selection leaked through occluding chrome and stuck on one-frame taps

## Observation
Dismissing a title-island notice left transcript text selected. The cause was not
hit-testing within the text (17ba6a3c) and not the island springs (it reproduced with
Reduce motion). gpui-component's TextView started a selection on any press inside its
`bounds`, using a raw `window.on_mouse_event` with `bounds.contains`. `.occlude()` on the
title island never applied to it, and the chat scrolls under the island. The view also
registered its mouse-up listener only on frames painted while `is_selecting`. A press and
release delivered in one frame (xdotool, touchpad tap) left `is_selecting` set, and the
selection then followed the hovering pointer until the next release. The rig showed text
from the ✕ through a table highlighted after one click.

## Fix
TextView inserts a `Normal` hitbox. A press starts a selection only when
`hitbox.is_hovered`, and any other press clears it (capture phase). Mouse-up is always
observed (capture phase), and a mid-drag move with no button pressed ends the drag.
`chrome::drag_region`'s comment ("BlockMouse also excludes underlying transcript
selection") is only true since this fix.

## Testing trap
GPUI's `simulate_*` helpers draw after every event, so they cannot reproduce one-frame
bugs. Use the vendored `VisualTestContext::simulate_events_in_one_frame`.
`Window::dispatch_event` returns a crate-private type and cannot be called from the
app. Find painted selections with `Window::rendered_fills()`, filtered by
`gpui_component::ActiveTheme::theme(cx).selection`, not the native palette's selection value.
Test: `island_controls_do_not_select_the_transcript_under_them`.
