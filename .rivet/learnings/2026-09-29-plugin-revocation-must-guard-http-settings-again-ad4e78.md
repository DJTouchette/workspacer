---
title: Plugin revocation must guard HTTP settings against the live broker registry
date: 2026-09-29
promoted: false
---

# Plugin revocation must guard HTTP settings against the live broker registry

## Observation
The plugin manager caches stable and pane credentials, so comparing HTTP UI tokens only against its loaded entries accepts credentials revoked directly through Handle.revoke_plugin. The HTML settings seed must query the actor-owned current plugin registry for exact plugin identity; normal manager unload cleanup alone is insufficient. Real HTTP regression must revoke in the broker while retaining manager cache.
