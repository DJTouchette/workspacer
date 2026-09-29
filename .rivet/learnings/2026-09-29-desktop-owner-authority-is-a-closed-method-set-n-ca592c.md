---
title: Desktop owner authority is a closed method set, not a namespace wildcard
date: 2026-09-29
promoted: false
---

# Desktop owner authority is a closed method set, not a namespace wildcard

## Observation
The Go bus desktop guard explicitly rejects desktop.internal.acceptSpawn even for the actual owner. Rust initially gated all desktop.* solely by authenticated_host, which left provider-registered private-shaped names reachable. The guard now also requires exact membership in contracts/desktop-service-methods.json ownerMethods. Real broker tests prove private rejection before provider forwarding, normal desktop success, and unaffected plugins.* plus actual plugin-namespace extension routing; lib257 passed.
