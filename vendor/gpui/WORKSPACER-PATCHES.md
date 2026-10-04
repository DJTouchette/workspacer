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
