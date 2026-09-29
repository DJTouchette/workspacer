---
title: Facade readiness must require exact HTTP success and be rechecked per launch
date: 2026-09-29
confidence: high
suggested_doc: mcp-tool-facade
related_paths:
  - services/hub-rs/src/services/session_facade.rs
promoted: false
---

# Facade readiness must require exact HTTP success and be rechecked per launch

## Observation
The retained Go facade probe requires HTTP200. Rust Legacy readiness used error_for_status, which accepts201/302; a ready-looking JSON body could therefore mint credentials under a non-health response. The probe now requires exact200 before decoding. A real HTTP matrix/recovery test checks status and all identity/readiness fields, no bearer/query disclosure on health, no token writes on refusal, recovery without cached failure, and replacement of an existing t query value.
