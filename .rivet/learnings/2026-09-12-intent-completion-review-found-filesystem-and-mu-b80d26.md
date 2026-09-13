---
title: Intent completion review found filesystem and mutable Git config boundaries
date: 2026-09-12
confidence: high
related_paths:
  - apps/desktop/src/main/services/intentKnowledgeStore.ts
  - apps/desktop/src/main/services/intentEvidenceStore.ts
  - apps/desktop/src/main/services/intentEvidenceCapture.ts
promoted: false
---

# Intent completion review found filesystem and mutable Git config boundaries

## Observation
Initial completion review found (1) knowledge path checks walked parents but opened leaf path with O_NOFOLLOW, which does not bind intermediate directories against substitution; (2) evidence artifact storage had no static directory symlink refusal; and (3) Git filter key enumeration once before several commands cannot freeze new executable filter keys introduced by later config edits. Follow-up owners are hardening with pinned directory handles and isolated Git metadata; verify their tests before describing these paths as confined/read-only.

## Recommendation
Keep deterministic parent-swap tests, static symlink refusal, late driver config injection coverage, and durable unknown receipt assertions.
