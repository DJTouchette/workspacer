---
title: ToolTraceCard tool-name colours: ToolSearch and mcp__* share one token, web is a literal
date: 2026-09-03
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/desktop/src/renderer/src/components/claude/ToolTraceCard.tsx
  - apps/desktop/src/renderer/src/components/claude/WorkCard.tsx
  - apps/desktop/src/renderer/src/themes.ts
promoted: false
---

# ToolTraceCard tool-name colours: ToolSearch and mcp__* share one token, web is a literal

## Observation
The trace card (apps/desktop/src/renderer/src/components/claude/ToolTraceCard.tsx) colours tool chips via CATEGORY_COLOR keyed by categoryOf(name): read=--wks-accent-text, edit=--wks-success, cmd=--wks-warning, search/agent/skill=--wks-purple, web=literal #38bdf8, everything else (ToolSearch, all mcp__* tools) =--wks-text-muted. In Dracula the muted text is a blue grey, so ToolSearch and mcp rows look like different colours in screenshots but are the same token. ToolCall carries only id/name/input/response/status/startedAt/completedAt: no exit code, output size, approval state or retry link. Approval pending lives on session.pendingApproval, not on the call.

## Impact
Any redesign of the tool card that wants per-tool colours, an approval-waiting row state, exit codes or nested subagent steps needs new tokens in cssVarsOf() and new fields on ToolCall / SubagentInfo. Design mockups for these live in .workspacer/design/2026-09-02/i-tool-card (gitignored).

## Recommendation
Add --wks-tool-* category tokens and a --wks-state-wait tone to themes.ts cssVarsOf() before restyling; replace the #38bdf8 literal with --wks-tool-web.
