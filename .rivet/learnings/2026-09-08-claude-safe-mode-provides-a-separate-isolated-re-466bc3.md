---
title: Claude safe mode provides a separate isolated readiness prerequisite
date: 2026-09-08
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/directCompletion.ts
  - apps/desktop/src/main/services/providerReadinessRuntime.ts
  - docs/fleet-provider-readiness.md
promoted: false
---

# Claude safe mode provides a separate isolated readiness prerequisite

## Observation
Installed native Claude 2.1.258 --help documents --safe-mode disabling CLAUDE.md, skills, plugins, hooks and MCP customizations while preserving auth; --bare instead excludes subscription OAuth and is unsuitable. completeReadinessPing uses the exact configured native executable after checking its help flags, safe mode/no tools/no persistence, fixed prompt, cheap same-provider default, and retry/output/deadline bounds. This does not relax complete(requireNoTools) or add Codex/OpenCode support. See docs/fleet-provider-readiness.md for verified scope and no-live-inference caveat.
