---
title: Generic on-event fixture stubs must not advertise optional detail subscriptions
date: 2026-09-23
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/renderer/src/harness/firstUseHarness.tsx
  - apps/desktop/src/renderer/src/hooks/useClaudeSession.ts
promoted: false
---

# Generic on-event fixture stubs must not advertise optional detail subscriptions

## Observation
Nightly CI firstUse Playwright tests lost live replies/result cards after onClaudeSessionDetail became an optional native IPC capability. The harness Proxy synthesized a no-op function for every unknown on* property, making feature detection choose a stream the fixture never emits. Initial full snapshot fetch still worked and fleet previews sometimes exposed prose, so narrow unit checks passed while five live browser scenarios failed. Explicit onClaudeSessionDetail:undefined keeps the fixture on its implemented onClaudeSessionUpdate stream; all five failing Chromium scenarios then passed unchanged.

## Impact
Optional capability presence is executable behavior; broad fixture proxies can falsely advertise support and create misleading rendering failures.

## Recommendation
Declare unsupported optional APIs explicitly absent in harnesses and retain real browser coverage for subsequent replies, decisions, and structured results, not only initial hydration.
