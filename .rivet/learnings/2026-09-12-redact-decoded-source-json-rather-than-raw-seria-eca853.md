---
title: Redact decoded source JSON rather than raw serialized bytes
date: 2026-09-12
confidence: high
related_paths:
  - apps/desktop/src/main/services/intentSourceAdapters.ts
  - apps/desktop/src/main/services/intentSourceAdapters.test.ts
promoted: false
---

# Redact decoded source JSON rather than raw serialized bytes

## Observation
Provider source responses may echo a configured credential inside nested strings or object keys. Replacing the literal secret in raw JSON does not remove JSON-escaped quotes/backslashes/unicode. Intent source adapters now redact decoded keys and values recursively, including token and Basic-header encodings, and reject nesting beyond 64 levels before retaining provider fields.
