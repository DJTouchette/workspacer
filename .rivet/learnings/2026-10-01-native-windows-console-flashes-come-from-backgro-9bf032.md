---
title: Native Windows console flashes come from background child creation flags
date: 2026-10-01
suggested_doc: native-embedded-backend
related_paths:
  - services/claudemon/src/background_process.rs
  - services/hub-rs/src/services/owned_process.rs
promoted: false
---

# Native Windows console flashes come from background child creation flags

## Observation
The release native executable uses the Windows GUI subsystem, but provider stream/model/heartbeat launches lacked CREATE_NO_WINDOW. Both owned_process launch sites and plugin supervisor/install passed flags=0 into suspended Job spawns, which overwrite earlier Command creation flags. Redirected stdio does not suppress a Windows console; job call sites must explicitly include CREATE_NO_WINDOW alongside suspended launch ownership. ConPTY has a separate creation path and should retain its pseudoconsole policy.

## Impact
Model discovery and Git/helper invocations can flash consoles; long-lived provider or plugin processes can leave console windows open.
