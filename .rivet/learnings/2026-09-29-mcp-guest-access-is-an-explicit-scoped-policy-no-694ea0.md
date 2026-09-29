---
title: MCP guest access is an explicit scoped policy, not an empty owner credential
date: 2026-09-29
author: codex
confidence: high
suggested_doc: mcp-tool-facade
related_paths:
  - services/hub-rs/src/mcp/access.rs
  - services/hub-rs/src/mcp.rs
  - services/hub-rs/src/mcp/legacy_sse.rs
  - services/hub-rs/src/runtime.rs
promoted: false
---

# MCP guest access is an explicit scoped policy, not an empty owner credential

## Observation
Go MCP ships untokened deny and explicitly allows view/operator only by dial. A separately configured static MCP token disables guest access regardless of the dial; effective guest access is loopback-only by checkBindPolicy. The Rust hub owner token is distinct from that optional facade static token. Rust now uses private scoped ephemeral connections for missing-credential opt-ins and static facade credentials, with no persistent token mint or session label; declared plugin method intent and operator facade intent stay narrow and never become Host. Dynamic policy must read current config bytes, not Config last-known-good data after a malformed or unreadable lockdown. Legacy SSE sessions bind credential, scope, guest kind and recheck policy while idle. UpstreamCaller already requires a dedicated scoped operator facade credential and refuses actual Host credentials, so the existing tier-filtered worker facade path can serve guests without borrowing an owner key.

## Impact
Treating an empty token as the host would bypass administration boundaries; falling back to guest after a bad credential would undo revocation. Reusing a view SSE session after a policy upgrade would silently change its authority, and reusing last-good config could retain access after a failed lockdown.

## Recommendation
Keep deny as default, reject every recognized present-but-invalid credential, preserve separate static-token semantics and loopback backstop, and test local/worker guest tiers plus per-request and idle SSE policy revocation. Omit config-derived untokened CLI flags in Rust Electron startup so live config remains authoritative; explicit CLI flags remain overrides.
