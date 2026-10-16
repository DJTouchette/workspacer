---
title: Adding a Rust-only MCP tool and pr.* bus methods
date: 2026-10-16
confidence: high
related_paths:
  - services/hub-rs/assets/mcp-tools.json
  - scripts/mcp-catalog.py
  - services/hub-rs/src/mcp.rs
  - apps/desktop/tests/support/capabilitySource.ts
promoted: false
---

# Adding a Rust-only MCP tool and pr.* bus methods

## Observation
No MCP tool had been added since the Go capture. A new one needs: an entry in
`mcp-tools.json` (tier list, sorted), a help group (every tier tool must be in
exactly one group) and optional guidance in `mcp-help.json`, a typed rule in
`mcp-wire-contract.json` `inputs` (optional strings `omit: zero`; a defaulting
boolean must be `pointer: true, omit: nil` or `false` is dropped), then
`python3 scripts/mcp-catalog.py --write`. Map the tool in `mcp.rs::method` and
dispatch adapters before the generic call; `identity.session_id` names the
calling session.

New bus methods registered with `options.handler` must loop over a LITERAL
array: the desktop scanner (`capabilitySource.ts`) rejects `for m in CONST`
("registration needs one literal loop"). Methods outside the sealed
hub-vocabulary go in `contracts/backend-capabilities.json` `full` +
`currentAdditions` (then regenerate brain-capabilities.json), and must NOT be
listed in authorization-compositions.json / compositionDecisions.ts.
