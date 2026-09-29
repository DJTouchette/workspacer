---
title: Core bus parity spans actor ownership rather than Go registration APIs
date: 2026-09-29
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub-rs/reviews/bus-core.json
  - services/hub/internal/bus/bus.go
  - services/hub/internal/bus/rpc.go
promoted: false
---

# Core bus parity spans actor ownership rather than Go registration APIs

## Observation
Full bus.go/rpc.go accounting maps connection/auth/HTTP/event responsibilities into auth, server/policy, protocol and Core, and routes spawn policy through external provider, owned coordinator and Origin. Production Go main derives ScopedIdent.Methods from rec.Scope.Methods, so arbitrary hand-edited Methods fixtures are not a live production API. recon.callers misses the AuthorizedForPlugin method value passed to pluginSettingsForRequest at main.go928; source reading confirms its real settings/UI consumer. Rust bounds queues and peers, freezes handler maps at startup, rejects anonymous network startup and restricts event.hub to the internal federation adapter.

## Impact
An API-name or aggregate-test comparison can miss both real consumers and deliberate ownership changes during migration.

## Recommendation
Keep per-responsibility source mappings and explicit ownership differences with the two core migration rows; use concrete tests for retained behavior rather than restoring synthetic Go extension points.
