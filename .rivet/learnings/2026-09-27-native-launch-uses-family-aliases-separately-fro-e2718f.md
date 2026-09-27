---
title: Native launch uses family aliases separately from live provider catalogs
date: 2026-09-27
promoted: false
---

# Native launch uses family aliases separately from live provider catalogs

## Observation
The native launcher reads claude.listModels aliases, groups canonical model/contextWindow variants, and intentionally ignores transcript-inferred version labels and seen IDs. Codex discovery uses providers.listModels with the remote project cwd. Catalog reads are generation/connection fenced; launch sends exact model/contextWindow and explicit provider-native permissionMode plus skipPermissions. Witness currently reports native sources unmapped, so use the full apps/native cargo test suite with ui-tests.
