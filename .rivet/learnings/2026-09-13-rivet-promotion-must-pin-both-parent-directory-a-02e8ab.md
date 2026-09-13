---
title: Rivet promotion must pin both parent directory and compared file bytes
date: 2026-09-13
confidence: high
suggested_doc: webview-security-hardening
related_paths:
  - apps/desktop/src/main/services/intentKnowledgeStore.ts
  - apps/desktop/src/main/services/intentArtifactFiles.ts
  - apps/desktop/src/renderer/src/components/IntentKnowledge.tsx
promoted: false
---

# Rivet promotion must pin both parent directory and compared file bytes

## Observation
Intent knowledge retrieval and reviewed promotion now reuse descriptor-pinned intentArtifactFiles storage. O_NOFOLLOW on the leaf alone does not prevent swapping an intermediate .rivet/context directory. For context replacement, compare the opened/pinned target bytes as well as the displayed project path before rename; checking only the unanchored path can validate a replacement directory and overwrite a concurrently edited pinned file. Written status is historical; current-file equality is checked only on write or explicit hash reconciliation.
