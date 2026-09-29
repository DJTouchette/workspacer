---
title: Remote workers require receiver-issued identity without local lineage claims
date: 2026-09-28
suggested_doc: hub-federation
related_paths:
  - services/hub-rs/src/services/agent_spawn.rs
  - services/hub-rs/src/services/remote_dispatch/receiver.rs
promoted: false
---

# Remote workers require receiver-issued identity without local lineage claims

## Observation
The Rust RemoteAdmission is non-cloneable and minted only after the receiver durably consumes an identity-bound lease and records its generated session UUID. SpawnCoordinator uses the same routed lifecycle but an explicit allowlist drops peer-supplied parent, manager, task, profile, integration and worktree allocation claims; only the leased execution cwd/provider and original repository routing context survive. remoteOrigin contains protocol and dispatchId only, while terminal escalation instructions remain available without inventing a local parent.
