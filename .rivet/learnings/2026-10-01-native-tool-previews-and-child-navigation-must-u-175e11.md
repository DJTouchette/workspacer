---
title: Native tool previews and child navigation must use retained receipts
date: 2026-10-01
promoted: false
---

# Native tool previews and child navigation must use retained receipts

## Observation
Native tool preview patch sections now reuse transcript file boundaries so deleted-file unified patches and History agree. Workspacer spawn_agent receipts identify fleet sessionId, distinct from provider-native Agent IDs; native navigation enables an explicit Open child session action only while that session is available. Tests cover direct/MCP receipts and disappeared children.
