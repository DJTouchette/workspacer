---
title: Native tool Running status used an unanimated loader SVG
date: 2026-09-29
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui/tools.rs
promoted: false
---

# Native tool Running status used an unanimated loader SVG

## Observation
tools::card rendered IconName::LoaderCircle through the same static Icon path as Done and Failed. The loader glyph does not animate by itself. Running tools now attach an 800ms repeating GPUI rotation keyed by session and stable call identity; completed and errored tools have no animation.

## Recommendation
When rendering native busy icons, explicitly attach GPUI animation or use Spinner. Keep animation identities stable through transcript updates and remove them after completion.
