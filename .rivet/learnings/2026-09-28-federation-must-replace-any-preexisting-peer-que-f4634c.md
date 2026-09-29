---
title: Federation must replace any preexisting peer query marker
date: 2026-09-28
confidence: high
suggested_doc: hub-federation
related_paths:
  - services/hub-rs/src/federation.rs
  - services/hub/internal/federation/federation.go
promoted: false
---

# Federation must replace any preexisting peer query marker

## Observation
The Go peerLinkURL helper appends peer=1, but Go query decoding reads the first peer value. A configured URL with peer=0 can shadow the appended marker; fragments can also swallow an appended query. The Rust port parses the URL and replaces all prior peer keys before adding peer=1. Live tests assert that peer-tagged host-token connections cannot assert authenticated-host identity.

## Recommendation
Preserve structured URL tagging and test it when integrating federation routing; do not use string concatenation for authority-reducing query markers.
