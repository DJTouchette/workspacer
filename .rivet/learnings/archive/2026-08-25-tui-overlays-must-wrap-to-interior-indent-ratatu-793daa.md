---
title: TUI overlays must wrap to (interior − indent); ratatui clips the overflow silently
date: 2026-08-25
confidence: high
suggested_doc: tui-client
related_paths:
  - apps/tui/src/ui/*.rs
promoted: true
promoted_to: tui-client
---

# TUI overlays must wrap to (interior − indent); ratatui clips the overflow silently

## Observation
In `apps/tui/src/ui/`, the idiom `for l in wrap(text, inner_w) { push(format!("    {l}")) }` overflows whenever the indent is added AFTER wrapping. ratatui's Paragraph does not panic or ellipsize — it just drops the tail beyond the block's interior, so the line looks complete. Caught while building `ui/nodes.rs`: the cost sentence "…nothing here can stop it again yet." rendered as "…nothing here can stop" / "again yet.", losing the word "it" mid-sentence, on a screen whose whole job is telling someone they are about to spend money. `modal_rect`'s clamp does not help — it guards the RECT, not the line width.</observation>
<parameter name="impact">Any overlay that indents wrapped prose can silently lose words. The failure is invisible in a passing `contains(...)` assertion unless the assertion happens to straddle the clip point.

## Recommendation
Compute the interior once (`w - 2`) and derive a width per indent level (`body_w = inner - 4`, `note_w = inner - 6`), wrapping each block to its own. When adding an overlay, dump the rendered buffer at a realistic size with a temporary `#[ignore]` test and READ it — the small-terminal sweep in ui_render_tests only proves it does not panic, not that it is legible.
