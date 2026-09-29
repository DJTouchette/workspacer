---
title: MCP typed wire omissions require explicit pointer-aware contracts
date: 2026-09-29
suggested_doc: mcp-tool-facade
related_paths:
  - services/hub-rs/src/mcp/wire.rs
  - scripts/mcp-catalog.py
promoted: false
---

# MCP typed wire omissions require explicit pointer-aware contracts

## Observation
The original Go facade omits value zero thresholds, so notify_when(contextUsedPct80,tokens0) forwarded only the valid health predicate. Rust raw forwarding instead kept tokens0 and the threshold service rejected it. Schema optionality cannot safely recreate Go omitempty: skipPermissions:false, forecast0, answer option0/text-empty and empty nonnil routing structs are meaningful. The new per-tool JSON wire rules are captured from Go AST tags/types and verified across100 builtins, while freeform/plugin values bypass projection. Portable Python generation now builds actual runtime schema/help artifacts from JSON contracts with no Go source or SDK dependency.

## Impact
Blanket zero/false omission would trade a compatibility fix for lost explicit permission and routing choices.

## Recommendation
Review new tool fields with explicit wire rules; use scripts/mcp-catalog.py --write/--check and the pointer-retention/no-legacy-source negative tests.
