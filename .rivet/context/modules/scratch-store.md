---
title: Project scratch store
tags: [scratch, hub, mcp, spawn, env, worktree, sensitive]
related_paths:
  - "services/claudemon/src/scratch.rs"
  - "services/claudemon/src/daemon/spawn.rs"
  - "services/hub-rs/src/services/scratch.rs"
  - "services/hub-rs/src/services/launch_instructions.rs"
  - "services/hub-rs/tests/scratch.rs"
  - "docs/features/scratch.md"
owner: Damien Touchette
last_reviewed: 2026-11-01
---

# Project scratch store

Design and contract: `docs/features/scratch.md`.

## One derivation, two consumers

`claudemon::scratch` is the only place a cwd becomes a scratch directory
(`~/.workspacer/scratch/<basename>-<fnv64(root)>/files`). claudemon's two spawn
handlers call `inject_env` to set `WORKSPACER_SCRATCH`; hub-rs depends on the
claudemon crate and `services/scratch.rs` calls the same `project_root`/`key`/
`project_dir`/`ensure`. Do not re-derive the path anywhere else (TS included):
ask the hub (`scratch.list` returns `project.files`).

Project root resolution is pure filesystem, no `git` subprocess: walk up to the
first `.git`; a `.git` *file* follows `gitdir:` → `commondir` to the main
checkout, so every linked worktree shares one scratch; a gitdir with no
`commondir` (submodule) is its own project.

## Why outside the checkout

`git worktree remove` (no `--force`) deletes ignored files with the tree, and
`git clean -fdx` does too. Scratch inside a checkout would be lost on session
close. Nothing in workspacer deletes scratch; `keep` is a fence for a future
sweep, not a current behavior.

## Non-obvious rules

- Spawn injection skips non-git cwds and roots under the system temp dir, so
  tests (claudemon's spawn tests do not isolate `$HOME`) and throwaway launches
  do not create scratch dirs in the real home.
- Installed outside the `control_plane_only` gate (runtime.rs), unlike
  `library.*`, because Electron runs the hub `--hub-only`.
- `sensitive` content is withheld only from **federated** callers; view/triage
  tokens cannot call scratch at all. Credential-shaped content forces
  `sensitive` and the file is written 0600. There is intentionally no execute
  method.
- OpenCode and Pi receive no spawn env, so they never see the variable.
