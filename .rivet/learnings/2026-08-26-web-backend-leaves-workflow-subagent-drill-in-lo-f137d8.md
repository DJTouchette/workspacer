---
title: Web backend leaves workflow subagent drill-in local-only
date: 2026-08-26
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/renderer/src/backend/webBackend.ts
  - apps/desktop/src/renderer/tests/backend/backendParity.test.ts
  - apps/desktop/src/main/services/providerSubagentConversation.ts
promoted: false
---

# Web backend leaves workflow subagent drill-in local-only

## Observation
createWebBackend currently implements workflowAgentTranscript and workflowAgentConversation as explicit null stubs, and backendParity excludes them as local transcript-file reads. That assumption fits Claude workflowWatcher artifacts, but provider-native Codex subagents are already available through claudemon conversations and can be exposed over a view-tier hub method instead of filesystem access.

## Impact
Prevents future work from treating all subagent drill-in as local filesystem-only and missing web/remote Codex support.

## Recommendation
When adding Codex subagent drill-in for web/remote clients, keep Claude workflow artifact reads local-only for runId != null, and add a separate qualified read path for plain provider subagent rows (runId null) over the hub bus.
