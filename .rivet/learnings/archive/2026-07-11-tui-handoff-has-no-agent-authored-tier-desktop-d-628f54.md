---
title: TUI handoff has no agent-authored tier; desktop does — silent capability gap
date: 2026-07-11
confidence: high
related_paths:
  - apps/tui/src/app/input.rs
  - apps/desktop/src/renderer/src/panes/ClaudePane.tsx
  - apps/desktop/src/main/services/agentHandoff.ts
  - apps/desktop/src/main/services/hubCapabilities.ts
  - apps/desktop/src/renderer/src/components/claude/ConversationEmptyState.tsx
  - apps/desktop/src/renderer/src/App.tsx
promoted: true
---

# TUI handoff has no agent-authored tier; desktop does — silent capability gap

## Observation
Cross-provider handoff has two brief-authoring tiers: agent-authored (source agent writes its own markdown brief, polled for up to 150s, apps/desktop/src/main/services/agentHandoff.ts) and mechanical/deterministic (claudemon builds it from the conversation timeline, services/claudemon/src/session/handoff.rs::build_brief, exposed at POST /sessions/:id/handoff). The Electron desktop app (apps/desktop/src/renderer/src/panes/ClaudePane.tsx handleHandoff) offers the user a choice between both tiers via a context menu ('agent' vs 'mechanical'). The Rust TUI (apps/tui/src/app/input.rs::do_handoff) ONLY ever calls drv.handoff() → the mechanical/deterministic brief — there is no TUI code path that asks the source agent to author its own brief. This isn't a bug, just an unimplemented parity gap; a future TUI editor adding handoff features should know the agent-authored tier exists on desktop and has no Rust/bus counterpart (no `claude.handoffAgentBrief` bus capability either — only `claude.handoffBrief` exists in apps/desktop/src/main/services/hubCapabilities.ts).

## Impact
Someone auditing cross-provider handoff for feature parity between TUI and desktop, or extending the handoff UX, could assume both surfaces share the same two-tier flow when only desktop does. Also: the two composer takeover prompt strings (App.tsx line ~1516 and input.rs line ~1135) are hand-duplicated word-for-word ('First read the handoff brief at {path}, then continue...') and apps/desktop/src/renderer/src/components/claude/ConversationEmptyState.tsx detects handoff takeover via regex /handoff brief at /i on initialPrompt — changing the wording in one place without the others breaks that detection silently (empty-state UI just won't show the "Taking over from a handoff" treatment).

## Recommendation
If adding TUI agent-authored handoff, mirror agentHandoff.ts's poll-with-timeout-then-fallback pattern and add a bus capability, or explicitly document the gap as intentional. Any edit to the handoff takeover prompt string must be made in both apps/desktop/src/renderer/src/App.tsx and apps/tui/src/app/input.rs::do_handoff, and must keep the substring 'handoff brief at ' intact for ConversationEmptyState.tsx's regex.

## Disposition
Not folded: already reflected in .rivet/context/domains/cross-provider-handoff.md (TUI tier gap + hand-duplicated takeover-prompt gotchas).
