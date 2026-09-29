---
title: Claude spawn binary escape hatch is separate from provider detection
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/spawn_plan.rs
  - services/hub-rs/src/services/models.rs
promoted: false
---

# Claude spawn binary escape hatch is separate from provider detection

## Observation
Go resolveSpawnBin honored configured agents.binaries.claude, then WKS_CLAUDE_BIN, then PATH/bare claude. Rust spawn_plan previously called ordinary resolve_binary and lost the environment fallback. resolve_spawn_binary restores it only on actual spawn plans; providers.checkAll and provider model discovery keep their prior config/PATH semantics. An isolated child test exercises precedence without mutating the parent process environment or executing a provider binary.
