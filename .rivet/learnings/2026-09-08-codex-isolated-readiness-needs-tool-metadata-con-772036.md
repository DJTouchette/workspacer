---
title: Codex isolated readiness needs tool metadata controls and the advertised model transport
date: 2026-09-08
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/codexReadinessPing.ts
  - apps/desktop/src/main/services/codexReadinessPing.native.test.ts
  - docs/fleet-provider-readiness.md
promoted: false
---

# Codex isolated readiness needs tool metadata controls and the advertised model transport

## Observation
Installed native Codex 0.153.4 can send one real isolated readiness request despite the generic completion adapter correctly refusing no-tools. exec --ignore-user-config/--ignore-rules/--ephemeral plus explicit feature/tool switches, skills.include_instructions=false, project_doc_max_bytes=0 and a local tool-free model catalog produce an empty tool registry and tiny input. Native loopback tests proved both classic Responses and Responses Lite shapes. Available small models must come from model/list; a bundled mini id was rejected by the live account. Preserve use_responses_lite from the CLI public models_cache.json metadata or a valid advertised model fails transport. A live native Codex ping succeeded after these controls. Keep version pinning, account/config prerequisite checks, and the generic complete(requireNoTools) guard.
