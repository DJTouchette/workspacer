---
title: Policy help must open Settings before selecting its real section
date: 2026-09-07
suggested_doc: renderer-event-buses
related_paths:
  - apps/desktop/src/renderer/src/lib/settingsBus.ts
  - apps/desktop/src/renderer/src/App.tsx
  - apps/desktop/tests/e2e/firstUse.test.ts
promoted: false
---

# Policy help must open Settings before selecting its real section

## Observation
requestSettingsSection queues/selects a section but does not open a Settings tab. The policy-help event is separate so App can open/focus the existing tab and then select, without double-opening callers that already open a pane before requestSettingsSection. Fleet workflows live under the existing supervisor section, not a standalone workflows key.

## Recommendation
Use openPolicySettings for first-task and policy-editor help links; keep the production App browser test that crosses first-task guidance, routing, and Fleet Manager workflows without dispatching.
