---
title: TUI overlays panicked on small terminals until modal_rect centralised the clamp
date: 2026-07-30
promoted: true
---

# TUI overlays panicked on small terminals until modal_rect centralised the clamp

## Observation
Every overlay in apps/tui/src/ui.rs hand-rolled its own centred rect. The preferred size was clamped to the frame at only some sites: height was .min(area.height)'d in rename/palette/picker/search but NOT in notes or spawn, and width was never clamped anywhere except whichkey. ratatui panics when a widget rect leaves the buffer, so the whole TUI aborted on sizes a user reaches by dragging a window shorter: notes at 80x3 and 35x3, spawn and rename at 10x4. Separately render_whichkey used clamp(16, area.width.saturating_sub(2)) — Rust's Ord::clamp panics when min > max, so any terminal under 18 columns crashed there regardless of overlay state. Fixed 2026-07-30 by routing all 8 sites through a single ui::modal_rect(area, want_w, want_h, ModalY) helper that clamps both axes and degrades the placement inset before the size. Placement is arithmetically identical to the old code whenever the box fits, so nothing changes at normal sizes. Found by a new TestBackend render harness (apps/tui/src/ui_render_tests.rs), not by any existing test — the pre-existing ui.rs tests only covered pure string helpers and never invoked the draw path.

## Disposition
Folded into .rivet/context/domains/tui-client.md (modal_rect note).
