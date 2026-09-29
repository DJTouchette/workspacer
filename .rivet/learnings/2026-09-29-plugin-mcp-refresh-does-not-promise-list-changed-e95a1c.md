---
title: Plugin MCP refresh does not promise list-changed notifications
date: 2026-09-29
suggested_doc: mcp-tool-facade
related_paths:
  - services/hub-rs/src/mcp/plugin_catalog.rs
  - services/hub-rs/tests/mcp.rs
promoted: false
---

# Plugin MCP refresh does not promise list-changed notifications

## Observation
The Go cmd/mcp/plugins.go polls every15seconds and explicitly emits no list-changed notification; clients refresh tools/list or reconnect. Its plugin-ID sanitizer can collapse distinct IDs such as one.two and one-two to the same prefix. Rust keeps the polling model, checks own-plugin method namespaces and skips first-party tool-name shadows; an ambiguous normalized catalog is rejected before replacing the previous complete map. Dynamic HTTP tests must register a first-party spawn handler before asserting a view token does not gain spawn_agent, otherwise absence could merely mean no backend handler exists.

## Impact
Avoids inventing a parity requirement that never existed and makes scope/collision regression evidence meaningful.

## Recommendation
Retain no-notification behavior, test replacement/removal directly, and exercise view/triage/operator plus provider denial with an actual MCP transport.
