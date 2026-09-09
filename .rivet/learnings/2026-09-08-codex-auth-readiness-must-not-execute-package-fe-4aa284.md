---
title: Codex auth readiness must not execute package-fetching PATH wrappers
date: 2026-09-08
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/agentProviders.ts
promoted: false
---

# Codex auth readiness must not execute package-fetching PATH wrappers

## Observation
The installed PATH codex shell launcher invokes npx --yes --prefer-online before forwarding arguments, so even login status would run package/network operations. Binary discovery alone does not establish a safe status adapter. Desktop readiness needs a separately verified local executable contract and must remain advisory/unsupported for unreviewed launchers.
