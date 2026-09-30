---
title: Native chat alignment requires shared outer measure and gutters
date: 2026-09-30
confidence: high
related_paths:
  - apps/native/src/ui/chrome.rs
  - apps/native/src/ui/transcript.rs
  - apps/native/src/appearance.rs
promoted: false
---

# Native chat alignment requires shared outer measure and gutters

## Observation
Native transcript rows previously capped their outer wrapper at CHAT_WIDTH and then applied 20px horizontal gutters inside it, while header/composer capped at CHAT_WIDTH + 40px before applying the same gutters. On wide windows transcript content was 40px narrower than composer. The shared chrome::chat_column now owns the outer measure and gutters for all three. Native themes are independent semantic palettes, not projections of Electron custom themes.

## Impact
Keep width and gutter ownership together when changing chat geometry or adding native themes.
