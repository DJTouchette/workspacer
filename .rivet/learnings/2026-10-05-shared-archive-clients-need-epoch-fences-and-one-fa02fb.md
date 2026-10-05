---
title: Shared archive clients need epoch fences and one owner for pending changes
date: 2026-10-05
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/renderer/src/hooks/useSessionArchive.ts
  - apps/native/src/controller.rs
  - apps/native/src/ui/features.rs
promoted: false
---

# Shared archive clients need epoch fences and one owner for pending changes

## Observation
useSessionArchive boolean pending ownership fails archive/restore/archive ABA, and a stale pre-disconnect get can consume shared resync state before a reset-version fresh read. Reads need sequence/event fences; writes need operation identity. Native must also accept the first authoritative document after reconnect even if version decreases. Serialize native writes per session, including migration, so restore during migration is sent after its archive acknowledgement; do not clear pending from an older matching document. Initial lists wait for the first archive read instead of flashing archived rows. None of these transitions changes lifecycle or drafts.
