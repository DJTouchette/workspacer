---
title: Native working state needs responding alias and a real BrandSpinner
date: 2026-09-27
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/model.rs
  - apps/desktop/src/renderer/src/components/Brand.tsx
promoted: false
---

# Native working state needs responding alias and a real BrandSpinner

## Observation
Native session_status previously omitted daemon mode responding, while BrandMark was static everywhere. Session::working now normalizes responding/working/thinking/streaming/running and excludes pending approvals/questions and stopped sessions; disconnected badges show Offline. Native BrandSpinner mirrors Electron Brand.tsx dimensions and uses a repeating 1.6s GPUI animation with eased forward/reverse movement, rendered only for loading/working indicators. Scroll follow is again tied to GPUI's measured is_scrolled flag, so deliberately scrolling to the bottom clears Jump to latest; existing anchor and constant-padding fixes prevent the prior scroll-stop jumps.
