---
title: Codex app-server empty environments retain default MCP authority
date: 2026-09-06
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - apps/desktop/src/main/services/directCompletion.ts
  - docs/agent-status-summary.md
promoted: false
---

# Codex app-server empty environments retain default MCP authority

## Observation
Codex 0.153.4 app-server rejects --ignore-user-config; cli/src/main.rs passes LoaderOverrides::default(). Config override tables merge recursively, so mcp_servers={} retains inherited entries. core/src/mcp.rs runtime_config_with_context grants Unrestricted authority to DEFAULT_MCP_SERVER_ENVIRONMENT_ID when environments has no selection. session/mcp_runtime.rs retains the fallback cwd and effective configured servers for eager MCP startup. Verified with installed help/schema and version-pinned source, without reading credentials or starting a provider session.

## Impact
The accepted clock/user-input utility exception does not resolve MCP isolation. An app-server summary could retain server-owned MCP execution even with environments=[] and dynamicTools=[]; client request denial cannot close that path.

## Recommendation
Keep Codex summaries fail-closed until a supported app-server config-isolation route preserving CLI auth is verified. Exposing the existing ignore_user_config loader option is a concrete upstream prerequisite, but also verify managed config and startup sources. See docs/agent-status-summary.md for links and reproduction.
