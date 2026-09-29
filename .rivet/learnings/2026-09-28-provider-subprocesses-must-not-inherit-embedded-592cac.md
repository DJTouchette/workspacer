---
title: Provider subprocesses must not inherit embedded hub authority
date: 2026-09-28
promoted: false
---

# Provider subprocesses must not inherit embedded hub authority

## Observation
Embedding hub and claudemon in one process makes inherited HUB_TOKEN/WKS_MCP_TOKEN visible to provider children unless stripped per child. New claudemon child_env removes those exact host authority keys after provider env overlays and from shared PTY construction; account keys and scoped facade URL/config remain. It never mutates process-global environment. Isolated subprocess fixtures cover std/Tokio/PTY paths; this is credential inheritance hygiene, not an OS sandbox or a filesystem privilege boundary.
