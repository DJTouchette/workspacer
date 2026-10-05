---
title: Native guarded editor saves require a host operation; Quit must guard popout edits
date: 2026-10-05
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/features.rs
  - apps/native/src/ui.rs
  - services/hub-rs/src/services/files.rs
promoted: false
---

# Native guarded editor saves require a host operation; Quit must guard popout edits

## Observation
The original native editor compared fs.read then called fs.write, allowing two clients to overwrite the same base. fs.compareWrite now validates and compares under an OS file lock also taken by plain fs.write, with a distinct method so older hubs fail closed. External writers ignoring advisory locks remain outside that guarantee. Global GPUI Quit bypassed window-close callbacks; it now defers into the main Workspace confirm_window_close guard, which also raises a dirty editor popout.
