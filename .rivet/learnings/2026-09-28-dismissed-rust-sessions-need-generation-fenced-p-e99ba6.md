---
title: Dismissed Rust sessions need generation-fenced projection tombstones
date: 2026-09-28
suggested_doc: session-lifecycle
related_paths:
  - services/hub-rs/src/services/sessions.rs
  - services/hub-rs/src/services/agent_ops.rs
promoted: false
---

# Dismissed Rust sessions need generation-fenced projection tombstones

## Observation
Claudemon retains stopped rows and emits later updates. agents.close must suppress both seed and update ingestion for the dismissed lifecycle generation, retain ended parent metadata for orphan discovery, and unhide only an intentional new generation. A simple BTreeMap remove resurrects rows immediately. Explicit close also needs the lifecycle launch mutex plus a replacement admission fence to avoid revoking credentials belonging to a concurrent resume.
