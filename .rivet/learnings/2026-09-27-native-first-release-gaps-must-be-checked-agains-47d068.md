---
title: Native first-release gaps must be checked against current controls
date: 2026-09-27
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/controller.rs
  - apps/native/src/ui.rs
  - apps/native/src/ui/navigation.rs
  - apps/native/src/main.rs
promoted: false
---

# Native first-release gaps must be checked against current controls

## Observation
At 09224f8d, native launch already has provider model catalogs, context-window choices, explicit permission modes, and a Windows installer; earlier same-day learnings and the README first-slice follow-on list are partly stale. The current controller Action/Command enums expose send, approve, interrupt, free-text answer, select, refresh, create, and model discovery, but no resume, rename, archive, termination, or running-model change commands. Transcript clipping labels older content as retained by the server but offers no history pagination. Native question UI prints the question JSON and answers from the composer; Settings exposes appearance, Vim navigation, default provider, and shortcuts. Window close stops the owned local engine unless --keep-running was supplied.
