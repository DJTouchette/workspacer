---
title: Windows intent helper uses per-operation PowerShell budget longer than UI RPC
date: 2026-09-13
confidence: high
related_paths:
  - apps/desktop/src/main/services/intentWindowsFiles.ts
  - apps/desktop/src/main/services/intentKnowledgeStore.ts
  - apps/desktop/src/renderer/src/backend/desktopServices.ts
  - apps/desktop/src/renderer/src/backend/hubBusClient.ts
promoted: false
---

# Windows intent helper uses per-operation PowerShell budget longer than UI RPC

## Observation
intentWindowsFiles.runIntentWindowsFiles spawns Windows PowerShell 5.1 synchronously with a 30-second timeout for every read/write/remove. Knowledge listing batches per category, while a context promotion currently performs multiple helper-backed reads, a write and verification. Renderer desktopServices.intentWorkspaceRequest still uses HubBusClient's default 15-second timeout, so the UI can time out before an otherwise bounded Windows operation completes. Separately, Windows Ordinary rejects hardlinks but Linux knowledge readBoundFile lacked an nlink check during review.

## Recommendation
Measure cold Windows helper timings in real CI, align operation deadlines with renderer RPC timeout, and keep Linux/Windows ordinary-file checks consistent.
