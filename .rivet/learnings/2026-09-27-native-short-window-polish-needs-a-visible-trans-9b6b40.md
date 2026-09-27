---
title: Native short-window polish needs a visible transcript budget
date: 2026-09-27
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui.rs
  - apps/native/scripts/smoke.py
promoted: false
---

# Native short-window polish needs a visible transcript budget

## Observation
A real 720x480 X11 capture showed header, approval details and composer could consume the whole transcript viewport even while UI tests passed. Native render now compacts these regions below 620px height, and omits the composer with no selected session. smoke.py supports theme, width, height, no-input and new-session captures with temporary appearance settings.
