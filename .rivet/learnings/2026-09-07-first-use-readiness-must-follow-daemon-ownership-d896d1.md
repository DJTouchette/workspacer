---
title: First-use readiness must follow daemon ownership and facade bus health
date: 2026-09-07
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/main/services/agentRuntimeStatus.ts
  - apps/desktop/src/renderer/src/components/SpawnAgentDialog.tsx
promoted: false
---

# First-use readiness must follow daemon ownership and facade bus health

## Observation
Daemon readyPromise is a startup/adoption promise, not ongoing liveness. Adopted daemons have no local child exit event. The MCP facade /health can return HTTP 200 with hubConnected:false. Runtime UI now observes existing daemon lifecycle promises, probes known owners read-only, checks facade hubConnected, and retains unknown for old/web/remote hosts. worktreeInfo is IPC host-only; isRepo:false alone cannot distinguish invalid, non-git, or an old web stub. The additive directory/gitStatus facts and exact path+owner key prevent stale replies and local inspection of peer cwd. Session boot reconciliation automatically resumes stopped records; first-use fixtures prove one resumeSessionId call and retained prose/results, not persistence of manager coordination graphs.
