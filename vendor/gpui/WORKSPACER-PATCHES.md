# Workspacer's GPUI patch

This is the crates.io `gpui` 0.2.2 source (`src`, `resources`, `build.rs`,
manifest, README and Apache 2.0 license; examples, tests and docs omitted, and
their targets removed from `Cargo.toml`). `apps/native/Cargo.toml` pins it
through a crates.io patch. 0.2.2 is the newest release; reapply on upgrade, or
drop this copy once upstream fixes text truncation.

`src/elements/text.rs`, `TextLayout::layout`: truncated text (`truncate()` /
`text_ellipsis()`) measured incorrectly across taffy's measurement passes.

- The measurement cache key now includes the truncation width. Upstream
  returned any cached size for unwrapped text, so text first measured
  unconstrained kept that size when flex later shrank its box: clipped, never
  "…". With wrapping on, a tiny first measurement stuck at a lone "…".
- Each measurement starts from a clone of the original text runs. Upstream
  `truncate_line` rewrote the captured runs in place, so after one truncating
  pass a later pass applied the first run (e.g. bold) to only a few characters.

`truncated_text_ellipsizes_at_its_flex_width_and_recovers` (apps/native UI
tests) pins the first fix and fails against upstream 0.2.2. The test platform's
text system ignores fonts, so the second is checked by real-window captures
(`apps/native/VALIDATION.md`, 2026-10-03).

`src/window.rs` + `src/elements/div.rs`: keyboard-only focus styles, as CSS
`:focus-visible` (2026-10-06). GPUI focuses a focusable div on mouse-down, so
a `.focus(..)` ring outlived every click: a native chip or disclosure toggled
off by mouse kept its accent border, reading as still "on", until focus moved.

- `Window` records whether the latest key or pointer press was a key
  (`dispatch_event`: `KeyDown` sets it, `MouseDown` clears it) and exposes it
  as `Window::last_input_was_keyboard()`. When it changes, after the event is
  handled, only the view that drew the focused element is marked dirty
  (`DispatchTree::view_id_for_node`). Not a window refresh (re-renders every
  cached view), not `cx.notify` (runs the entity's observers), and not before
  dispatch (`dispatch_key_event` draws a dirty window first): each added a
  frame that broke the native transcript's page-up/follow tests. It starts
  as pointer, so focus set programmatically at startup draws no ring.
- `InteractiveElement::focus_visible(..)` is `.focus(..)` gated on that flag.
  Upstream 0.2.2 has neither; check a newer release for an equivalent before
  reapplying on upgrade.
- Test support only: `Window::rendered_borders()` lists the last frame's
  visibly bordered quads in logical pixels, so UI tests check the painted ring.

Pinned by `pointer_focus_draws_no_ring_but_keyboard_focus_does` and
`fleet_wakes_render_as_named_worker_cards_not_user_bubbles` (apps/native UI
tests); both fail with `.focus(..)` in `chrome::interactive_control`.
