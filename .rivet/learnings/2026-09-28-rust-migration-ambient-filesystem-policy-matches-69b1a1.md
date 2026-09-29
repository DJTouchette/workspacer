---
title: Rust migration ambient filesystem policy matches current authenticated Go and TS paths
date: 2026-09-28
suggested_doc: hub-bus-control-plane
related_paths:
  - Read actual implementation call sites before treating retained secret helpers or old compatibility fixtures as active authorization.
promoted: false
---

# Rust migration ambient filesystem policy matches current authenticated Go and TS paths

## Observation
Go cmd/brain/fsguard.go assertPathAllowed only canonicalizes and deliberately ignores roots. TS pathConfinement.ts assertPathAllowed does the same. Go internal/bus/bus.go authorize returns immediately for trusted, scoped and plugin identities; secret/root checks remain solely in an unreachable anonymous legacy unit harness. A migration must preserve this current ambient policy and retain semantic selected-object containment without silently reintroducing the retired broad secret denylist.
