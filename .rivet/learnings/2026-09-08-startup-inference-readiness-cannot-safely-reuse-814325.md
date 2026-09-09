---
title: Startup inference readiness cannot safely reuse Codex one-shot completion
date: 2026-09-08
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/directCompletion.ts
  - services/claudemon/src/daemon/oneshot.rs
promoted: false
---

# Startup inference readiness cannot safely reuse Codex one-shot completion

## Observation
directCompletion.complete rejects requireNoTools for Codex and OpenCode with no-tools-unsupported before invoking their adapters. Codex exec --ephemeral --sandbox read-only still permits tools; OpenCode --pure only skips external plugins. The Claude /oneshot no_tools request has no isolation capability handshake and runs in the home directory, so its existence alone cannot certify no inherited instructions or safe behavior on old daemons. A startup account ping must not bypass these guards or substitute another provider.
