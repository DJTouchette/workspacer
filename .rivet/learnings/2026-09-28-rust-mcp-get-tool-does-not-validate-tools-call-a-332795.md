---
title: Rust MCP get_tool does not validate tools call arguments
date: 2026-09-28
confidence: high
suggested_doc: mcp-tool-facade
related_paths:
  - services/hub-rs/src/mcp.rs
  - services/hub-rs/tests/mcp.rs
promoted: false
---

# Rust MCP get_tool does not validate tools call arguments

## Observation
rmcp3.5.0 get_tool is consumed by streamable HTTP tool_schema for Mcp-Param header checks; raw ServerHandler.call_tool does not automatically validate arguments against inputSchema. A live plugin test demonstrated a missing required argument reaching its handler. Rust facade now explicitly validates every resolved tool argument object with jsonschema, with HTTP/file schema resolution disabled.

## Recommendation
Keep integration regression using a required plugin argument and ensure invalid input never invokes its provider; do not assume catalog schema publication enforces it.
