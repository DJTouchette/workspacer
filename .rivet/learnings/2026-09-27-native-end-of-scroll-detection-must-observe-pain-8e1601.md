---
title: Native end-of-scroll detection must observe painted tail geometry
date: 2026-09-27
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/ui/scroll.rs
promoted: false
---

# Native end-of-scroll detection must observe painted tail geometry

## Observation
A regression using Ctrl+U then Ctrl+D reproduced visible transcript bottom while follow stayed false: ListState can clamp to the bottom during layout without invoking its scroll handler; keyboard scrolling bypasses that handler too. Native now observes the painted final row (both tool and message rows), defers the check until ListState finishes layout, and resumes following when its bottom is above the composer. It rejects stale session/Arc<Row> observations and hides the button without calling scroll_to, avoiding a corrective snap. GPUI 0.2.2 Frame::clear does NOT clear debug_bounds, so a selector from an old frame can remain after its element disappeared; do not use debug_bounds(...).is_none() as an absence oracle. The original failing regression asserted follow=false at physically visible bottom, and passes after geometry reconciliation.
