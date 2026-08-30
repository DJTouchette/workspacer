---
title: Worktree node_modules auto-link only reached depth ≤2 — renderer's node_modules is 4 levels deep
date: 2026-08-23
confidence: high
suggested_doc: workspacer-serve-cli
related_paths:
  - apps/desktop/src/main/services/worktreeService.ts
  - apps/desktop/src/main/services/worktreeService.test.ts
promoted: true
---

# Worktree node_modules auto-link only reached depth ≤2 — renderer's node_modules is 4 levels deep

## Observation
discoverNodeModules() in worktreeService.ts capped its scan at parent-depth ≤2 (`node_modules`, `<a>/node_modules`, `<a>/<b>/node_modules`), which covers root node_modules and apps/desktop/node_modules but NOT apps/desktop/src/renderer/node_modules (parent is 4 segments deep: apps/desktop/src/renderer). There was no worktreeSetup config entry for the workspacer project in config.yaml either (checked ~/.config/workspacer/config.yaml — projects./home/djtouchette/Work/worky/workspacer only has delivery+yolo, no worktreeSetup), so the gap was pure omission, not misconfiguration — every fresh worktree silently lacked the renderer's deps and had to be hand-symlinked.

## Impact
Any new nested npm package added anywhere in the repo tree (at any depth) now gets its node_modules auto-linked into fresh agent worktrees with zero code or config changes — this was the recurring papercut that made three ship workers hand-symlink the renderer's node_modules today.

## Recommendation
Fixed by making discoverNodeModules recurse to any depth (still skipping descent into node_modules itself and dot-dirs, so cost stays bounded — measured ~3900 dirs / ~17ms full walk on this repo). Verified with a real scratch worktree: `git worktree add` + the fixed linker + a full renderer vitest run (1142/1142 passing) with zero manual symlinking, then worktree removed. Prefer widening the code's depth handling over adding a per-project worktreeSetup symlink command — a config-only fix wouldn't help a fresh clone and this repo has a documented history of config.yaml being clobbered.

## Disposition
Promoted into .rivet/context/domains/agent-spawn.md (hand-authored notes 2026-08-22/23, worktree node_modules bullet) rather than the originally suggested workspacer-serve-cli — that module is about the headless CLI launcher, not the desktop's worktree service.
