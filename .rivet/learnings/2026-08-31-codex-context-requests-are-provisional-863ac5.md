---
title: Codex context requests are provisional
date: 2026-08-31
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/providers/codex.rs
  - apps/desktop/src/renderer/src/lib/sessionStats.ts
promoted: false
---

# Codex context requests are provisional

## Observation
Codex model_context_window is a numeric launch request shared by app-server, rollout fallback, and hybrid TUI, while statusLine.contextWindowSize is the runtime-confirmed effective denominator. SessionUsage.contextLimit may contain the request and must never drive AgentCard, SideBar, or SessionStatusBar percentages.

## Impact
Conflating request/catalog metadata with runtime truth makes a requested 1M window masquerade as effective and lets cumulative/session estimates corrupt active occupancy bars.

## Recommendation
Route all Codex launch shapes through one config-argv builder; keep requested/effective/default/maximum labels separate; derive both bars only from runtime contextWindowSize.
