---
title: Worktree launch rescans all visible directories and model cache misses are not coalesced
date: 2026-09-23
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/services/worktreeService.ts
  - services/claudemon/src/providers/mod.rs
  - services/claudemon/src/providers/codex.rs
promoted: false
---

# Worktree launch rescans all visible directories and model cache misses are not coalesced

## Observation
Performance audit after f96d0010 plus local batching fixes: discoverNodeModules in worktreeService.ts serially readdir-walks every non-dot, non-node_modules directory, then stats node_modules under every visited directory. It does not prune target/build/dist/vendor or consult Git ignores. createWorktree awaits linkNodeModules before configured setup and provider spawn; each concurrent worktree independently repeats discovery. Provider cached_or_fetch in services/claudemon/src/providers/mod.rs checks its 600s completed-value cache then directly awaits fetch without a per-key inflight guard. Concurrent cold/expired requests can each launch a catalog subprocess. Codex fetch_models additionally awaits debug models --bundled after its 10s app-server read timeout, with no timeout or kill_on_drop on that second command; listProviderModels in Electron also has no fetch AbortSignal. Composer controls fetch catalogs only on menu open, so do not claim every mounted pane triggers this. Spawn dialog, settings and other clients are additional callers.

## Impact
Large ignored build trees can delay worktree-backed launches, while concurrent catalog misses amplify startup process/memory pressure and a hung bundled-model probe can leave model selection loading indefinitely. Runtime magnitude has not been profiled on the reporting desktop.

## Recommendation
Cache or coalesce node_modules discovery per canonical repo with explicit refresh and validated paths; prune known build directories or discover workspace manifests, and preserve clean-tree checks. Add per-key inflight catalog deduplication, bounded command deadlines and child cleanup, and a client request deadline. Measure worktree_dependency_links and catalog fetch separately.
