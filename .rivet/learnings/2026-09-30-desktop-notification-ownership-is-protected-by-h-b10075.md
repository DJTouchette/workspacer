---
title: Desktop notification ownership is protected by hub-only composition, not local catalog inventory
date: 2026-09-30
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/main/services/brainDelegation.ts
  - apps/desktop/src/main/services/hubDaemon.ts
  - services/hub-rs/src/runtime.rs
  - services/hub-rs/src/provider_relay/methods.rs
promoted: false
---

# Desktop notification ownership is protected by hub-only composition, not local catalog inventory

## Observation
Current Electron bootstrap hard-codes DELEGATE_CATALOG_TO_BRAIN=false and starts Rust serve --hub-only with external claudemon. That sets control_plane_only and skips services::install_config, so its log-only notifications handler does not shadow the desktop OS/in-app provider by default. Outbound Catalog relay separately filters notifications.post from offered methods. Inspect actual composition/export boundaries before treating an engine-less local Hub handler inventory as the legacy catalog surface. Full adopted notification degradation remains documented.
