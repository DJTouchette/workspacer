---
title: Reactive intent release supersedes manual status-only revision audit
date: 2026-09-13
confidence: high
related_paths:
  - apps/desktop/src/main/services/intentWorkspaceStore.ts
  - apps/desktop/src/main/services/intentAutomationStore.ts
  - docs/intent-workspaces-guide.md
promoted: false
---

# Reactive intent release supersedes manual status-only revision audit

## Observation
The combined September 13 nightly candidate advances intent storage to schema 6 and introduces owner-host reactive manager execution. Status-only updates retain the requirements revision, unlike the earlier same-day audit. Requirement changes increment revision; activated intent review can transition to Complete or resume Active. docs/intent-workspaces-audit.md is explicitly historical; current user behavior is in docs/intent-workspaces-guide.md.

## Recommendation
When assessing lifecycle or releasing, use schema-6 behavior and rerun the combined automation tests; do not apply the earlier manual-only audit to reactive execution.
