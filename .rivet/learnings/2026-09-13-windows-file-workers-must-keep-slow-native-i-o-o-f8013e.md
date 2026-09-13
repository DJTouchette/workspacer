---
title: Windows file workers must keep slow native I/O outside SQLite write locks
date: 2026-09-13
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/main/services/intentFileWorker*.ts
  - apps/desktop/src/main/services/intentArtifactStore*.ts
  - apps/desktop/src/main/services/intentEvidenceStore*.ts
  - apps/desktop/src/main/services/intentKnowledgeStore*.ts
promoted: false
---

# Windows file workers must keep slow native I/O outside SQLite write locks

## Observation
Moving synchronous PowerShell I/O to WorkerThreads alone does not preserve owner responsiveness: BEGIN IMMEDIATE held by the worker blocks main-thread observation/metadata writers for busy_timeout. IntentArtifactStore and IntentKnowledgeStore now perform bounded file I/O before short metadata CAS transactions; IntentEvidenceStore writes unique bytes before its revision/idempotency commit and reads artifact bytes outside a write transaction. The broker permits one active and eight queued allowed file requests, expires each150s from enqueue, strips transcript snapshots, rejects queued requests on worker failure and never replays. Native helpers use the remaining deadline. The same bundled intent-file-worker.cjs ships beside headless host and unpacked in Electron.

## Recommendation
Preserve two-connection tests that attempt main-thread writes from inside file-I/O callbacks, not merely timer responsiveness tests. Any new file action must join the explicit worker allowlist and ship through all native/headless/server packaging paths. PowerShell5.1 coerces null string arguments to empty strings; native create-new uses String.IsNullOrEmpty(expectedDigest).
