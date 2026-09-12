---
title: Mobile spawn blanket full-access denial lagged the router's operator policy
date: 2026-09-11
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/hub/cmd/hub/mobile.html
  - services/hub/internal/bus/bus.go
  - services/hub/internal/bus/rpc.go
promoted: false
---

# Mobile spawn blanket full-access denial lagged the router's operator policy

## Observation
mobile.html renderSpawn disabled bypassPermissions/yolo unconditionally and said all remote requests were stripped. Current conn.mayBypassPermissions explicitly admits ordinary operator and host tokens; only peer links need an explicit yolo grant. The mobile spawn form also offered only remembered directories and a prompt() path, no models/effort, and automatically sent library content without an editable task field. Added hello.spawnFullAccess from the canonical router predicate, remote fs.listDir browsing, config.projects entries, provider catalog/model/context-window selection, effort, and editable first-message text.

## Recommendation
Derive UI permissions from the router's current result, never from stale blanket statements about remote access. Preserve canonical model/contextWindow pairs including transitional aliases with a canonical model and legacy value suffix.
