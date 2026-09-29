---
title: Modern MCP list results require explicit private cache hints in rmcp
date: 2026-09-29
related_paths:
  - services/hub-rs/src/mcp.rs
  - services/hub-rs/tests/mcp.rs
promoted: false
---

# Modern MCP list results require explicit private cache hints in rmcp

## Observation
Windows Claude Code2.1.284 rejected tools/list after negotiating2026-07-28. Reproduced with actual authenticated Rust facade wire test: rmcp3.5 ListToolsResult::default omits ttlMs/cacheScope while server/discover supplies them. Explicit zero TTL and private scope preserve identity-filtered catalogs. Empty prompts/resources/template list defaults need the same metadata; resources/read remains method-not-found. Native-client success does not establish which version or endpoint was negotiated.
