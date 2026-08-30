---
title: runId === null is the Codex-vs-Claude discriminator for subagent drill-in, and it is re-implemented in four places
date: 2026-08-26
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/main/ipc.ts
  - apps/desktop/src/renderer/src/backend/webBackend.ts
  - apps/desktop/src/renderer/src/backend/bridgedBackend.ts
  - apps/desktop/src/renderer/src/panes/AgentWatchPane.tsx
promoted: true
promoted_to: renderer-backend-seam
---

# runId === null is the Codex-vs-Claude discriminator for subagent drill-in, and it is re-implemented in four places

## Observation
`workflowAgentTranscript` / `workflowAgentConversation` kept their old `(sessionId, runId, agentId)` signature, and `runId === null` was quietly overloaded into the routing switch between Claude workflow artifacts and provider-native (Codex) child threads. The rule is written out FOUR separate times and they must agree:

- `main/ipc.ts` — try `workflowWatcher` first; fall back to `readProviderSubagentConversation` only if the watcher returned falsy AND `runId === null`.
- `backend/webBackend.ts` — `readProviderSubagentConversation` returns null immediately unless `runId === null`.
- `backend/bridgedBackend.ts` — `runId !== null` goes straight to preload IPC; `runId === null` tries the bus first, then falls back to IPC.
- `panes/AgentWatchPane.tsx` — hardcodes `null` for `kind: 'subagent'`.

`WorkflowTimeline.tsx:75` (`txRunId`) is the ONLY caller in the tree that ever passes a non-null runId. So in practice: workflow-run rows = non-null = local artifact files; every plain subagent row = null = eligible for the provider path.

Note the fallback ordering differs by backend on purpose. In bridged mode the BUS is tried before IPC for `runId === null`, specifically so desktop dev exercises the same path web/remote uses; IPC is only the older-hub fallback. In main's ipc.ts the order is reversed (watcher first). Do not "harmonize" them.</observation>
<parameter name="impact">A Codex subagent row that ever acquires a runId silently routes to the Claude artifact reader and renders empty. Conversely a future workflow row that also wants provider data can never reach it, because a non-null runId short-circuits before the provider branch in all three backends.

## Recommendation
If subagent drill-in needs a third source, add an explicit discriminator to the signature rather than overloading runId further — and update all four sites plus `backendParity.test.ts` (these two methods moved from KNOWN_STUBS to BUS_BACKED, so the parity guard now demands real bus implementations).</recommendation>
<parameter name="confidence">high
