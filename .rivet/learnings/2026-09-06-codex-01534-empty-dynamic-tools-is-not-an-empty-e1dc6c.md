---
title: Codex 0.153.4 empty dynamic tools is not an empty registry
date: 2026-09-06
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - apps/desktop/src/main/services/directCompletion.ts
  - docs/agent-status-summary.md
promoted: false
---

# Codex 0.153.4 empty dynamic tools is not an empty registry

## Observation

Installed Codex 0.153.4 generated thread/start and turn/start schemas have no whole-registry tools or tool_choice control. Matching rust-v0.153.4 core/src/tools/spec_plan.rs add_core_utility_tools registers async user questions and clock from model metadata even with environments empty and relevant ordinary toggles disabled. ToolsToml and tool_registry config contain no blanket deny control. Strict summary support therefore remains blocked; dynamicTools empty, read-only, and config isolation do not prove no tools.

## Impact

Enabling the Codex status-summary adapter using only those fields would violate its required no-tools contract.

## Recommendation

Keep no-tools-unsupported until an upstream complete registry/dispatch restriction is verified, or obtain an explicit revised utility-tool contract. See docs/agent-status-summary.md for version-specific evidence and scope.
