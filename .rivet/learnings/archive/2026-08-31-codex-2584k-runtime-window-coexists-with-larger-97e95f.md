---
title: Codex 258.4K runtime window coexists with larger configurable catalog maximum
date: 2026-08-31
author: Codex scout f686cfea
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - services/claudemon/src/providers/codex.rs
  - services/claudemon/src/providers/mod.rs
  - services/claudemon/src/daemon/spawn.rs
  - apps/desktop/src/renderer/src/lib/sessionStats.ts
  - apps/desktop/src/renderer/src/components/claude/SessionStatusBar.tsx
promoted: true
promoted_to: claudemon-providers
---

# Codex 258.4K runtime window coexists with larger configurable catalog maximum

## Observation
Durable Codex rollout data for Workspacer session d9723c3c maps via ~/.workspacer/codex-threads to thread 01a055de. Across 539 token_count events, model_context_window was always 258400, max last input was 227231, and five compactions occurred around 217K-227K; the >250K figure was cumulative total usage, which reached 75,586,943 and matches state_5.sqlite threads.tokens_used. Separately, official Codex config documents model_context_window, `config/read` accepts a session flag override, and `codex debug models` exposes context_window/max_context_window; Workspacer currently persists generic contextWindow but does not pass it into codex::spawn_session.

## Impact
Do not diagnose a >250K session token count as proof that the live context exceeded 258.4K, and do not claim Codex has no context override. The runtime bar, cumulative billed counter, requested value, catalog default/max, and compaction threshold are distinct.

## Recommendation
Keep the active context bar on latest input over provider-reported modelContextWindow; label cumulative totals billed. A future Codex spawn-time context control must pass -c model_context_window, validate against the selected CLI catalog, and wait for runtime confirmation. No live context switch is available.

## Disposition
Promoted into `.rivet/context/modules/claudemon-providers.md` WITH A CORRECTION: "Workspacer persists a generic contextWindow but does not pass it into `codex::spawn_session`" is stale. `set_context_window_in_argv` (`daemon/spawn.rs`) and `codex.rs` now emit `-c model_context_window=<n>`, and `SpawnAgentDialog` offers "Request 1M". The five-numbers distinction and the measured 258,400 specimen were kept; "no live context switch" is still true.
