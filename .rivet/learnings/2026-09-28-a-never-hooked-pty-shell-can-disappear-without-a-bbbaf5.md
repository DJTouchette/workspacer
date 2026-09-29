---
title: A never-hooked PTY shell can disappear without a session update
date: 2026-09-28
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/session/store.rs
  - services/hub-rs/src/services/sessions.rs
promoted: false
---

# A never-hooked PTY shell can disappear without a session update

## Observation
In claudemon SessionStore.drop_pending_spawn, EOF cleanup removes state and byte plumbing but emits no SessionUpdate. Shell PTYs normally never send provider SessionStart hooks, so natural exit uses this path. A terminal byte subscriber sees channel closure, while a session projection seeded from SessionUpdate can retain the original active row indefinitely unless another operation triggers reseed. The Rust terminal adapter verifies GET404/stopped before pty.exit, but authoritative removal notification is also needed to update the shared session projection.
