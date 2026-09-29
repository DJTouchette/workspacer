---
title: Worker result validation is a deliberately partial schema dialect
date: 2026-09-28
confidence: high
suggested_doc: fleet-manager
related_paths:
  - services/hub-rs/src/services/worker_results.rs
  - apps/desktop/src/main/shared/structuredResult.ts
  - services/hub/cmd/brain/workerescalation.go
promoted: false
---

# Worker result validation is a deliberately partial schema dialect

## Observation
Desktop structuredResult.ts only enforces type/properties/required/items/enum/additionalProperties; false and malformed schema nodes constrain nothing, and unknown keywords like pattern/minLength must be ignored. It counts JavaScript UTF16 code units despite error messages saying bytes. The Go fixed-escalation twin counts UTF8 bytes instead, so nonASCII cap behavior already differs between retiring hosts. Rust worker_results preserves native desktop UTF16 limits and keeps MCP argument validation separate/full-schema.

## Recommendation
Keep portable TS/Rust result fixtures and distinct successful-result vs fixed-escalation outcomes; do not reuse the MCP jsonschema validator for worker reports. Decide/document the preexisting Go-vs-desktop Unicode cap divergence before final legacy deletion.
