---
title: Project groups
tags: [projects, groups, hub, config, revision, scratch]
related_paths:
  - "services/hub-rs/src/services/project_groups.rs"
  - "services/hub-rs/src/services/scratch.rs"
  - "services/hub-rs/src/runtime.rs"
  - "services/hub-rs/tests/project_groups.rs"
  - "docs/features/projects.md"
  - "apps/native/src/projects.rs"
owner: Damien Touchette
last_reviewed: 2026-11-01
---

# Project groups

Design and contract: `docs/features/projects.md`.

## A separate file, not a config.yaml key

Groups live in `<config_dir>/project-groups.json`, written only by
`projectGroups.save` / `projectGroups.remove` in `services/project_groups.rs`.
Both config.yaml writers (hub `config.save` and desktop's TypeScript writer)
deep-merge saves, so a client holding a stale copy would resurrect a deleted
group or undo a rename. The per-folder `projects:` map in config.yaml is
untouched and still owns label/icon/colour/pin/worktree settings per folder.

## Non-obvious rules

- **Revision compare-and-set.** Every successful write bumps `revision`; a
  `revision` param that differs from the stored one is refused with an error
  starting `conflict:`. Omitting `revision` is last-writer-wins. The file is
  read and written under `ConfigLock::take(&path)`.
- **One group per folder.** Folders are normalised like native `project_key`
  (trim, `\`→`/`, no trailing slash, `/` and `C:/` stay roots), must be absolute
  and carry no `.`/`..` segment, and are de-duplicated. A folder already in a
  different group is refused with that group's name; nested folders are
  allowed (membership is longest-prefix). Comparison is exact for POSIX paths
  on Linux and case-folded for drive/UNC paths and on Windows/macOS.
- **Folders are labels.** Nothing opens, canonicalizes or spawns in them; a
  group grants nothing. That is why the parameter policy calls `folders` inert
  membership strings.
- **Save is a replace.** `name` and `folders` are required; `color`/`icon` left
  out are cleared. `createdAt` survives; unknown fields on a stored group are
  kept.
- **A damaged file is never overwritten.** Missing file = `{version:1,
  revision:0, groups:{}}`; invalid JSON, a version other than 1, a missing
  revision or a group without name/folders is an error on every method.
- **Installed outside the `control_plane_only` gate** (runtime.rs, beside
  scratch) whenever `config_dir` is set, because Electron runs the hub
  `--hub-only` and desktop never writes this file.
- `projectGroups.changed {revision}` is guarded by `projectGroups.list` and has
  no provider publisher; `projectGroups.list` is quiescence-passive.

## Scratch pending counts

`scratch.pending {}` reads `index.json` + `project.json` of each directory under
`claudemon::scratch::base(home)` (1,000 dirs max, symlinks and damaged indexes
skipped, never walks `files/`) and returns `{projects:[{key,root,pending}],
truncated}` for pending > 0, sorted by key. `scratch.changed` now carries
`{key, pending}`, computed from the index the mutation just wrote.
