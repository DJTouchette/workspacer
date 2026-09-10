---
title: Resolver reference provenance must validate identifiers before additive singleton merging
date: 2026-09-10
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/shared/requestReferences.ts
  - apps/desktop/src/main/services/managerRequestService.ts
promoted: false
---

# Resolver reference provenance must validate identifiers before additive singleton merging

## Observation
applyTaskReferences is an authorized additive editor, not an original-text provenance validator: successive pullRequest upserts overwrite earlier values before final validation. The request resolver must validate each original PR/MR identifier and URL-number agreement, enforce whole Unicode ticket identifier boundaries, reject distinct PR mappings per target, and preflight reference edits before mutating tasks or deleting inbox text. Manual update_task_references remains separately authorized.
