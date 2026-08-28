---
title: A wrong context WINDOW also produces absurd TOKEN counts, because managed sessions derive tokens as pct x window
date: 2026-08-26
confidence: high
suggested_doc: usage-accounting
related_paths:
  - apps/desktop/src/main/services/claudeSessionStore.ts
  - apps/desktop/src/main/services/hubCapabilities.ts
  - services/claudemon/src/session/state.rs
  - services/claudemon/src/providers/claude_stream.rs
promoted: false
---

# A wrong context WINDOW also produces absurd TOKEN counts, because managed sessions derive tokens as pct x window

## Observation
Managed (non-Claude) sessions have no direct context-token count. `contextTokensFromStatusLine` (apps/desktop/src/main/services/claudeSessionStore.ts) derives one as `contextUsedPct / 100 * contextWindowSize`, and it feeds both the bus row (`hubCapabilities.ts agents.list`) and `session.peakContext`. Two consequences that are not obvious from either side alone: (1) any bug that inflates the WINDOW inflates the reported TOKENS by the same factor — the `claude_stream.rs` `.max()` across `modelUsage` entries (a 1M sub-agent inflating a 200k parent) showed up as "crazy" token numbers, not only as a wrong meter; (2) `context_used_pct` was read unclamped off the provider payload in `StatusLine::from_claude_json` (session/state.rs) while `UsageAcc::status_line` already clamped the percentage it COMPUTES — so the handed-in path could multiply the window by an out-of-range value. Both are now clamped, at the parser and again at the consumer.

## Impact
Diagnosing an absurd token figure by auditing token-side code alone will miss it: the cause can be entirely on the limit side. Also means `peakContext` (which now drives the drift alarm that disarms a claimed window) can be poisoned by a bad percentage, turning one wrong reading into a permanently hidden meter.

## Recommendation
When a session reports an impossible token count, check the WINDOW first, and check whether that session is managed (usage-less) so its tokens are derived rather than counted. Keep both clamps: the parser one covers the four clients that render `context_used_pct` directly (TUI, /m, remote.html, desktop), the consumer one covers producers other than Claude's statusLine.
