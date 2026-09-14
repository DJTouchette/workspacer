---
title: Intent account lease tested across local OS processes, not machines
date: 2026-09-13
confidence: high
related_paths:
  - apps/desktop/src/main/services/intentSourceSyncStore.test.ts
  - docs/intent-provider-sync.md
promoted: false
---

# Intent account lease tested across local OS processes, not machines

## Observation
The new deterministic sync-store regression holds a lease inside one OS process adapter while a second Node process opens its own connection to the same SQLite file and attempts import. It verifies exclusion during the lease and cooldown, then acquisition after the injected clock advances. This verifies same-machine SQLite coordination only; shared-filesystem/cross-machine locks and provider quotas remain unverified.
