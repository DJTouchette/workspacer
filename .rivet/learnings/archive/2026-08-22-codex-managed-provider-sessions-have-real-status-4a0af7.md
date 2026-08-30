---
title: Codex/managed-provider sessions have real statusLine usage but agents.list + notify_when read the wrong field (s.usage, always null)
date: 2026-08-22
confidence: high
suggested_doc: usage-accounting
related_paths:
  - apps/desktop/src/main/services/hubCapabilities.ts
  - apps/desktop/src/main/services/thresholdWatch.ts
  - apps/desktop/src/main/services/thresholdWatcher.ts
  - apps/desktop/src/main/services/sessionStore/analyticsWriter.ts
  - apps/desktop/src/main/services/sessionStore/usageAccumulator.ts
promoted: true
---

# Codex/managed-provider sessions have real statusLine usage but agents.list + notify_when read the wrong field (s.usage, always null)

## Observation
Managed-provider sessions (codex/opencode/pi) never populate ClaudeSessionState.usage — it stays `usage: null` (claudeSessionStore.ts:1445), set only by usageAccumulator.applyUsage via the 'usage' conversation-item case in conversationApplier.ts:427-434, which the Rust conversation_item() mapper in services/claudemon/src/providers/mod.rs (~413-448) never emits for AgentUpdate — it only maps AssistantText/UserText/ToolUse/ToolResult. Codex's real numbers live only on session.statusLine, populated provider-agnostically via claudemonStatusLineBridge.ts from claudemon's UsageAcc::status_line (mod.rs:624+) / store.apply_status_line (store.rs:1426). Most consumers correctly read statusLine-first with a usage fallback (InspectorCard.tsx:486, sessionStats.ts:186-189, stallDetector.ts:76, analyticsWriter.ts:41-43 — the last has an explicit comment naming this exact codex/opencode/pi gap and closing it). But two do not: hubCapabilities.ts's `agents.list` capability (line 217, `contextLimit: s.usage?.contextLimit ?? 0`, also model/contextTokens/costUSD on lines 215-218) reads s.usage only, and thresholdWatch.ts/thresholdWatcher.ts (notify_when's tokens/usd predicates) raw-cast claudeSessionStore.getAllSnapshots() to WatchableSession (thresholdWatcher.ts:10) with no statusLine field modeled at all. Separately, session.peakContext is set only inside applyUsage (usageAccumulator.ts:42), so it stays 0 for codex forever, and analyticsWriter.ts:44 persists that 0 into session_history with no statusLine fallback (unlike lines 41-43 immediately above it) — and session.usage.models (usageAccumulator's per-model split) never exists for codex, so analyticsWriter.ts:54's `if (session.usage?.models) sessionHistory.recordModels(...)` never fires — the session_model_usage per-model breakdown table has zero codex rows even though the session-level total is correct.

## Impact
agents.list (the hub capability behind mcp__workspacer__list_agents / the Fleet Manager's fleet view / mobile / remote / MCP facade) reports contextTokens/contextLimit/costUSD as always 0 for codex/opencode/pi workers. notify_when({tokens: N}) and notify_when({usd: N}) can NEVER fire for a codex worker — the manager's sanctioned no-poll safety net silently loses 2 of its 3 predicates for non-Claude providers (idleSeconds still works, it uses ambientState/lastActivity, not usage). This is a direct blocker for making codex a first-class harness alongside Claude.

## Recommendation
Give WatchableSession (thresholdWatch.ts) and the agents.list mapper (hubCapabilities.ts:210-226) the same statusLine-first-then-usage fallback analyticsWriter.ts already uses (lines 41-43). Also add a statusLine fallback for peakContext in analyticsWriter.ts:44, mirroring the pattern one line above it.

## Disposition
Not promoted — verified STALE, all three claimed gaps are already fixed in current code: hubCapabilities.ts agents.list already falls back to s.statusLine for model/contextTokens/contextLimit/costUSD (lines ~255-258, with an explicit comment naming the codex/opencode/pi gap); thresholdWatch.ts's WatchableSession already models statusLine and sessionTokens/crossedBy already fall back to it; claudeSessionStore.applyStatusLine already updates session.peakContext from statusLine (comment: "the only other peakContext writer"), and analyticsWriter.ts's recordModels already falls back to statusLine.modelDisplay for the per-model split. This is exactly the kind of already-fixed claim the task warned to check for before promoting.
