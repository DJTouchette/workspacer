---
title: An intentionally disabled facade must not disable the owned Rust launch graph
date: 2026-09-29
author: codex
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/hub-rs/src/runtime.rs
  - services/hub-rs/src/services/session_facade.rs
  - services/hub-rs/tests/no_mcp_spawn.rs
  - services/hub-rs/tests/session_facade.rs
promoted: false
---

# An intentionally disabled facade must not disable the owned Rust launch graph

## Observation
Rust serve already exposed --no-mcp, but Lifecycle construction required a bound MCP address and launchReady always required mcp_ready, so the flag also disabled agents.spawn. Go has no literal matching flag, but its missing-facade-binary reduced stack starts brain with no invented facade URL; buildSessionFacade returns nil before minting credentials or installing facade skill pointers. Rust now represents deliberate absence with Readiness::Disabled and endpoint None, builds the same owned lifecycle/coordinator, and only requires mcp_ready when a listener is configured. Common preparation still preserves profile prompts, structured result/escalation contracts, and explicitly selected library MCP servers, without a Workspacer entry, bearer, or generated tool/skill claims. Existing credential cleanup remains generation-aware; absent token stores are not created in disabled mode. Pi remains product-unsupported.

## Impact
A readiness dependency on an optional transport silently removes unrelated launch capabilities. Treating unavailable configured listeners as disabled would hide a real failure, while injecting a guessed URL would mint authority for no proven service.

## Recommendation
Keep None/Disabled distinct from a configured failed listener. Exercise the actual managed-provider path with a credential-free local fake CLI: assert first-message receipt, selected library config, no token store or Workspacer skill claims, and owned PID cleanup. The current Unix no_mcp_spawn fixture passes; selected third-party MCP server execution itself is deliberately not performed.
