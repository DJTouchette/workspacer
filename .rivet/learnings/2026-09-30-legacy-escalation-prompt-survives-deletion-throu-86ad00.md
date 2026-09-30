---
title: Legacy escalation prompt survives deletion through exact captured bytes
date: 2026-09-30
confidence: high
suggested_doc: headless-desktop-services
related_paths:
  - apps/desktop/src/main/shared/workerEscalation.test.ts
  - scripts/check-doc-drift.sh
promoted: false
---

# Legacy escalation prompt survives deletion through exact captured bytes

## Observation
Ordinary workerEscalation.test.ts read a Go source constant and would fail after services/hub deletion. Captured933-byte prompt retains original sourceSHA and commit, TS asserts exact current output regardless of legacy presence, and existing actual Rust facade test now asserts full prompt across6 provider/profile cases. Doc drift also previously swallowed grep missing-file failure; explicit current-document checks and grep status handling now distinguish no matches from unperformed scan.
