---
title: Native custom-model context choice derived options from the current pick
date: 2026-10-05
confidence: low
related_paths:
  - apps/native/src/ui/launch.rs
  - apps/native/src/ui/settings.rs
promoted: false
---

# Native custom-model context choice derived options from the current pick

## Observation
render_context_choice used self.context_window as the only window for a custom model ID, so choosing Default (None) removed the 1M option; the old button also rendered an empty label child plus a second child, stacking two lines (tall, low label). It is now a segmented_enabled control whose custom options include the selected session's window on Change model.
