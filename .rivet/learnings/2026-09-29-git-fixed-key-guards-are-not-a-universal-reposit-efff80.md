---
title: Git fixed-key guards are not a universal repository-config sandbox
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/git.rs
  - services/hub-rs/tests/git.rs
promoted: false
---

# Git fixed-key guards are not a universal repository-config sandbox

## Observation
The retained Go and desktop GIT_NO_EXEC prefix neutralizes ten fixed config keys; --no-ext-diff covers the declared nine diff-family subcommands. It does not itself sandbox every named clean/textconv driver or commit hook, and older comments relying on a blanket .git write denial are stale under ambient host path policy. Rust Git tests isolate global/system config, empty hooks/attributes, use authored nonexistent executor sentinels for probes, and push only to temporary bare local repositories with non-file transports denied.
