---
title: First-use renderer spawn failures must precede persistence; failed detection rechecks invalidate missing verdicts
date: 2026-09-07
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/renderer/src/hooks/useAgentManager.ts
  - apps/desktop/src/renderer/src/App.tsx
  - apps/desktop/src/renderer/src/hooks/useProviderDetection.ts
promoted: false
---

# First-use renderer spawn failures must precede persistence; failed detection rechecks invalidate missing verdicts

## Observation
spawnAgent previously swallowed IPC rejection and allocated a session-less workspace. App also persisted launch defaults before awaiting spawn. Existing provider detection is binary-only and its failed-refresh path retained a stale missing verdict, which becomes a false launch block once missing providers are gated. The renderer fix rejects malformed/empty ids before workspace mutation, awaits before launch persistence, and returns detection to unknown after a failed refresh. First messages already ride kickoffMessage -> message -> firstMessage; retry must not add a follow-up send.
