---
title: Git typed operands must not collapse into pathless mutations
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/git.rs
promoted: false
---

# Git typed operands must not collapse into pathless mutations

## Observation
services/git.rs previously read path with as_str().unwrap_or(""); malformed numeric/bool path values silently became pathless stage-all/unstage-all. Git now validates each method actual consumed string/bool/int fields before starting any Git process and refuses noncanonical Go-fold aliases rather than ignoring them. git.numstat again ignores diff-only path/untracked fields as its Go request did. Non-repository diagnostics now retain the reference message; malformed Unicode status columns are skipped without slicing panic.
