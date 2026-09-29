---
title: Readiness is an isolated model response, not CLI discovery or authentication
date: 2026-09-28
promoted: false
---

# Readiness is an isolated model response, not CLI discovery or authentication

## Observation
Rust provider_utilities ports the headless provider readiness contract: startup is inert until first readiness request, then one delayed default-manager check; read polling never spends allowance, manual checks coalesce, config changes cancel owned futures and hide stale facts. Readiness only returns state and timestamp, never model output, account metadata or executable paths. Claude requires native safe-mode capability evidence. Codex is pinned to the source-verified0.153.4 contract: exact wrapper resolution, native version, read-only app-server metadata/policy/cheap-model checks, public transport metadata and an isolated no-tools catalog; unsupported contexts do not fall back to another provider. Cosmetic title generation uses provider-specific argv/parsers with its own fallback and must not be reused as readiness proof.
