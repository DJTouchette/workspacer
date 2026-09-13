---
title: Intent experience needs a separate config axis from fleet/focus
date: 2026-09-12
confidence: high
suggested_doc: ui-modes-manifest
related_paths:
  - apps/desktop/src/renderer/src/lib/uiMode.ts
  - apps/desktop/src/main/services/db/database.ts
promoted: false
---

# Intent experience needs a separate config axis from fleet/focus

## Observation
The proposed project-owned intent workspace experience changes the primary work surface. Existing ui.mode fleet/focus and MODE_MANIFEST only govern agent attention and explicitly preserve pane identity. Use a separate opt-in ui.intentWorkspaces flag rather than extending fleet/focus or remounting session panes. Workspacer also already has an application database in services/db/database.ts, beyond claudemon's session-event store.
