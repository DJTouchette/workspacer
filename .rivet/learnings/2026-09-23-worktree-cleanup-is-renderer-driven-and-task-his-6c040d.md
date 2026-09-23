---
title: Worktree cleanup is renderer-driven and task history omits ordinary children
date: 2026-09-23
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/services/worktreeService.ts
  - apps/desktop/src/renderer/src/hooks/useAgentManager.ts
  - apps/desktop/src/main/headless/desktopHost.ts
promoted: false
---

# Worktree cleanup is renderer-driven and task history omits ordinary children

## Observation
removeAgentWorktree only removes linked trees under the currently configured root using non-force git worktree remove, preserving branches and awaiting fleetReviewStore.beforeRemove. Current callers are explicit renderer close or ended child auto-drop with a resolvable parent card; headless internal.observe records histories but does not cleanup. Dirty/untracked/locked trees are intentionally skipped and fire-and-forget renderer callers do not surface results. DispatchHistoryStore.accept admits only live local isWakeTarget owners and excludes trackTask:false, so history is not a complete allocation registry for ordinary-agent child worktrees. node_modules links refer back to the source checkout and directory-only ignore rules cause link rollback. worktreeRoot is renderer-typed and carried via generic config maps, absent canonical defaults and main config type; Go brain delegates worktree creation to shared desktopHost, no independent Go allocation path for that field.

## Impact
Stopped workers can retain ignored build outputs indefinitely when no renderer teardown fires or the tree has valuable dirty source. A sweep based only on task history misses ordinary children and old/pruned rows. Following dependency links while cleaning could affect the original checkout.

## Recommendation
Use a backend-owned durable allocation registry plus daemon-confirmed stopped state, canonical path/identity fences, and all-session cwd containment checks. Clean only recognized ignored generated outputs without following links, preserve dirty source and branches, and expose plan/apply through a named project CLI. Idle Input sessions are live, never cleanup candidates. Preserve review capture before whole-tree removal.
