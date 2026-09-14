---
title: Intent reports must redact before any bound and carry projection redaction provenance
date: 2026-09-14
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/shared/intentReport.ts
  - services/claudemon/src/session/intent_report.rs
  - apps/desktop/src/main/headless/intentObservations.ts
  - contracts/intent-report-cases.json
promoted: false
---

# Intent reports must redact before any bound and carry projection redaction provenance

## Observation
Native intentObservation and captureFinalIntentReport previously truncated independently before storage redaction; Rust completion_source also bounded raw credentials. Either could leave a credential boundary fragment that downstream regexes cannot recognize. Both now redact complete assistant text first with dependency-free scanners pinned by shared contracts/intent-report-cases.json, then retain <=4000 UTF-16 units without splitting pairs. truncated measures sanitized text; redacted includes replacements beyond the retained prefix. Headless requires additive redactionVersion=1 and redacted fields and fails closed for older raw-bounded projections.

## Recommendation
Keep native observation/capture, proposal provenance and Rust projection aligned. Run the shared adversarial TS/Rust fixtures and headless protocol tests when changing the scanner or bounds. Do not accept legacy truncated projections or discard upstream redaction metadata.
