---
title: Managed routing patches need stricter validation than the host loader
date: 2026-09-07
suggested_doc: limit-aware-routing
related_paths:
  - services/hub/internal/routing/preferences.go
  - services/hub/internal/routing/load.go
promoted: false
---

# Managed routing patches need stricter validation than the host loader

## Observation
The trusted host loader deliberately tolerates invalid policy values and leaves fallbacks to Select. In particular, its structural validator does not reject unknown manual mode strings or invalid forecast weights. Managed preference composition must validate these explicitly before saving, or a successful Apply could silently fall back instead of selecting the requested mode.

## Recommendation
Keep strict managed-input validation separate from permissive host loading; test invalid manual modes and forecast bounds as no-write/no-install cases.
