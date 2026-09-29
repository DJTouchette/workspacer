---
title: External daemon bridge retains SSE transport only in explicit borrowed mode
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/external_claudemon.rs
promoted: false
---

# External daemon bridge retains SSE transport only in explicit borrowed mode

## Observation
Go claudemon bridge source maps to ExternalDaemon for explicit hub-only borrowed daemons; full standalone/native use the owned engine. Existing Rust parser preserves mapped fields, chunked CRLF/multiline/default event names and EOF flush; added actual503 retry/cancellation plus live-body-close regressions. Rust intentionally rejects redirects and credential-bearing URLs, adds cumulative frame bound, and shutdown closes the link without stopping the external daemon.
