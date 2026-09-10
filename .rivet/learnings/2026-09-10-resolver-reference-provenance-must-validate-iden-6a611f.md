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

Hosted integration caveat: DispatchHistoryStore.list() projects a moving wallMs
for live attempts, so before/after view equality fails across an awaited MCP
call even when persisted data did not change. For complete no-mutation assertions,
read task records through requestTransaction (which clones its return value);
retain byte-for-byte fixture-file assertions in synchronous resolver tests.

Resolver URL identity validation must run whenever a pullRequest URL is supplied,
including URL-only mappings. Presence in original text proves source, not PR kind:
ordinary URLs remain generic references. Keep URL-only, number-only, both and
neither forms in one provenance matrix to prevent optional-field validation gaps.
